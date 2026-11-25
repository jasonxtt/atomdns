use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use atomdns_upstream_core::TransportCancellation;
use tokio::net::UdpSocket;
use tokio::task::JoinSet;

#[cfg(test)]
use crate::assembly::HostOptions;
use crate::assembly::{ForwardCatalog, HostAssembly};
use crate::cache::CacheCatalog;
#[cfg(test)]
use crate::config::CompiledConfig;
use crate::config::ListenerKind;
#[cfg(test)]
use crate::execution::execute_request;
use crate::execution::{
    ExecutionRequest, ExecutionResult, RootEnv, RootRun, SuspendedRoot, SyncStop,
};
use crate::observer::{
    AdmittedQueryGuard, QueryObserver, QueryTerminalOutcome, QueryTransport, TerminalObservation,
};
use crate::runtime_snapshot::AdmittedSnapshot;

#[cfg(any(not(target_os = "linux"), test))]
const MAX_UDP_PACKET: usize = 65535;
/// Datagrams the receive loop handles synchronously before it yields, so the
/// requests it handed to other tasks, upstream replies, refreshes, the
/// management API and shutdown run in between. At full load one hit takes
/// about 17 µs, so other tasks wait at most about half a millisecond; one
/// yield costs about a microsecond, spread over 32 datagrams.
const INLINE_BURST: usize = 32;
#[cfg(test)]
const REFUSED: u8 = 5;

/// A local UDP listener owned by the native host. Its request tasks run on the
/// host's current-thread local task set because sequence executors are not
/// widened to `Send + Sync` by this migration slice.
pub struct UdpServer {
    /// The upstream and cache owners an unsupervised listener closes when it
    /// stops. A supervised listener holds none: its requests use the admitted
    /// snapshot and the supervisor closes every generation, so holding them
    /// here would keep a replaced generation alive.
    owners: Option<(Rc<ForwardCatalog>, Rc<CacheCatalog>)>,
    socket: Arc<UdpSocket>,
    observer: Arc<QueryObserver>,
    control: crate::runtime_snapshot::RuntimeControl,
    listener_tag: String,
    listener_addr: SocketAddr,
    #[cfg(test)]
    mode: DispatchMode,
}

/// Test seam: how the receive loop hands a datagram over.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DispatchMode {
    /// The product path: handled in the receive loop (batched on Linux,
    /// per-datagram elsewhere), moved to a task only when it has to wait.
    Inline,
    /// Per-datagram inline handling on Linux, for direct comparison with the
    /// batched Linux path.
    #[cfg(target_os = "linux")]
    SingleInline,
    /// Every datagram in its own task from the start (the path before the
    /// inline handling), for comparing the two.
    Legacy,
}

impl Drop for UdpServer {
    /// A listener that served to the end, or was staged for a configuration
    /// apply that never published it, no longer appears in the health document.
    fn drop(&mut self) {
        self.control.forget_udp_receive_buffer(&self.binding_key());
    }
}

impl UdpServer {
    /// Binds a UDP listener after strict configuration compilation. The
    /// supplied address is testable with port zero; the compiled graph still
    /// determines that the selected listener role is UDP.
    pub async fn bind(assembly: &HostAssembly, listen: SocketAddr) -> Result<Self, UdpServerError> {
        Self::bind_listener(assembly, &assembly.config().listener, listen, false).await
    }

    pub(crate) async fn bind_listener(
        assembly: &HostAssembly,
        declared: &crate::config::ListenerConfig,
        listen: SocketAddr,
        supervised: bool,
    ) -> Result<Self, UdpServerError> {
        if declared.kind != ListenerKind::Udp {
            return Err(UdpServerError::WrongListener(declared.kind));
        }
        let socket = crate::bind::udp(listen)
            .await
            .map_err(UdpServerError::Bind)?;
        let server = Self {
            owners: (!supervised).then(|| (assembly.forwards_handle(), assembly.cache_handle())),
            socket: Arc::new(socket),
            observer: assembly.observer_handle(),
            control: assembly.control(),
            listener_tag: declared.tag.clone(),
            listener_addr: declared.listen,
            #[cfg(test)]
            mode: DispatchMode::Inline,
        };
        server.publish_receive_buffer();
        Ok(server)
    }

    #[cfg(test)]
    pub(crate) fn with_mode(mut self, mode: DispatchMode) -> Self {
        self.mode = mode;
        self
    }

    /// Makes the receive buffer the kernel granted visible to the health endpoint.
    fn publish_receive_buffer(&self) {
        self.control.set_udp_receive_buffer(
            self.binding_key(),
            crate::bind::receive_buffer(&self.socket),
        );
    }

    pub(crate) async fn bind_snapshot(
        control: crate::runtime_snapshot::RuntimeControl,
        observer: Arc<QueryObserver>,
        declared: &crate::config::ListenerConfig,
    ) -> Result<Self, UdpServerError> {
        let bound = crate::bind::udp(declared.listen)
            .await
            .map_err(UdpServerError::Bind)?;
        let server = Self {
            owners: None,
            socket: Arc::new(bound),
            observer,
            control,
            listener_tag: declared.tag.clone(),
            listener_addr: declared.listen,
            #[cfg(test)]
            mode: DispatchMode::Inline,
        };
        server.publish_receive_buffer();
        Ok(server)
    }

    pub(crate) fn binding_key(&self) -> crate::runtime_snapshot::BindingKey {
        (
            self.listener_tag.clone(),
            ListenerKind::Udp,
            self.listener_addr,
        )
    }

    /// Binds the listener address declared by the compiled W1 YAML.
    pub async fn bind_configured(assembly: &HostAssembly) -> Result<Self, UdpServerError> {
        Self::bind(assembly, assembly.config().listener.listen).await
    }

    pub fn local_addr(&self) -> Result<SocketAddr, UdpServerError> {
        self.socket.local_addr().map_err(UdpServerError::LocalAddr)
    }

    /// Receives datagrams, stops admission on cancellation, then cancels/joins
    /// every request task before draining the upstream owner.
    ///
    /// A datagram is handled in the receive loop itself: a query that is
    /// answered without waiting (a cache hit, a local answer) is answered and
    /// sent right here. A query that has to wait moves, with everything it has
    /// done so far, to its own task. On Linux the receive loop batches
    /// non-blocking `recvmmsg` / `sendmmsg` calls under load while keeping
    /// single-packet sends on `try_send_to` at low load.
    pub async fn serve(self, shutdown: TransportCancellation) -> Result<(), UdpServerError> {
        #[cfg(target_os = "linux")]
        {
            #[cfg(test)]
            if self.mode != DispatchMode::Inline {
                return self.serve_single(shutdown).await;
            }
            self.serve_batched(shutdown).await
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.serve_single(shutdown).await
        }
    }

    #[cfg(target_os = "linux")]
    async fn serve_batched(self, shutdown: TransportCancellation) -> Result<(), UdpServerError> {
        let mut tasks = JoinSet::new();
        let mut recv_batch = crate::udp_batch::RecvBatch::new(INLINE_BURST);
        let mut send_scratch = crate::udp_batch::SendScratch::new(INLINE_BURST);
        let mut pending: Vec<PendingSend> = Vec::with_capacity(INLINE_BURST);
        let mut receive_error = None;
        let mut task_error = None;
        let mut handled_since_yield = 0_usize;

        loop {
            let has_tasks = !tasks.is_empty();
            let max_count = INLINE_BURST - handled_since_yield;
            tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                joined = reap_one_task(&mut tasks, &mut task_error), if has_tasks => {
                    if joined && task_error.is_some() {
                        shutdown.cancel();
                    }
                }
                received = recv_batch.recv(&self.socket, max_count) => {
                    match received {
                        Ok(count) => {
                            for index in 0..count {
                                if shutdown.is_cancelled() {
                                    break;
                                }
                                let Some((raw, peer)) = recv_batch.packet(index) else {
                                    handled_since_yield += 1;
                                    continue;
                                };
                                // A panic while handling one datagram ends the
                                // listener exactly as a panicking request task does.
                                let handled = self.handle_caught_batched(
                                    &mut tasks,
                                    raw,
                                    peer,
                                    &shutdown,
                                    &mut pending,
                                );
                                if let Err(message) = handled {
                                    if task_error.is_none() {
                                        task_error = Some(message);
                                    }
                                    // Flush replies already computed earlier in
                                    // this batch before cancelling shutdown so
                                    // their outcomes match per-datagram order.
                                    self.flush_pending(
                                        &mut tasks,
                                        &mut send_scratch,
                                        &mut pending,
                                        &shutdown,
                                    );
                                    shutdown.cancel();
                                    break;
                                }
                                handled_since_yield += 1;
                                if pending.len() >= INLINE_BURST {
                                    self.flush_pending(
                                        &mut tasks,
                                        &mut send_scratch,
                                        &mut pending,
                                        &shutdown,
                                    );
                                }
                            }
                            self.flush_pending(
                                &mut tasks,
                                &mut send_scratch,
                                &mut pending,
                                &shutdown,
                            );
                        }
                        Err(error) => {
                            receive_error = Some(error);
                            shutdown.cancel();
                            break;
                        }
                    }
                }
            }
            if handled_since_yield >= INLINE_BURST {
                handled_since_yield = 0;
                #[cfg(test)]
                YIELDS.with(|count| count.set(count.get() + 1));
                tokio::task::yield_now().await;
            }
        }

        debug_assert!(pending.is_empty());
        self.finish_serve(shutdown, tasks, task_error, receive_error)
            .await
    }

    #[cfg(any(not(target_os = "linux"), test))]
    async fn serve_single(self, shutdown: TransportCancellation) -> Result<(), UdpServerError> {
        let mut tasks = JoinSet::new();
        let mut packet = vec![0_u8; MAX_UDP_PACKET];
        let mut receive_error = None;
        let mut task_error = None;
        let mut handled_since_yield = 0_usize;

        loop {
            let has_tasks = !tasks.is_empty();
            tokio::select! {
                biased;
                () = shutdown.cancelled() => break,
                joined = reap_one_task(&mut tasks, &mut task_error), if has_tasks => {
                    if joined && task_error.is_some() {
                        shutdown.cancel();
                    }
                }
                received = self.socket.recv_from(&mut packet) => {
                    match received {
                        Ok((length, peer)) => {
                            #[cfg(test)]
                            if self.mode == DispatchMode::Legacy {
                                self.spawn_legacy(&mut tasks, &packet[..length], peer, &shutdown);
                                continue;
                            }
                            // A panic while handling one datagram ends the
                            // listener exactly as a panicking request task does.
                            let handled = self.handle_caught(&mut tasks, &packet[..length], peer, &shutdown);
                            if let Err(message) = handled {
                                if task_error.is_none() {
                                    task_error = Some(message);
                                }
                                shutdown.cancel();
                            }
                            handled_since_yield += 1;
                        }
                        Err(error) => {
                            receive_error = Some(error);
                            shutdown.cancel();
                            break;
                        }
                    }
                }
            }
            if handled_since_yield >= INLINE_BURST {
                handled_since_yield = 0;
                #[cfg(test)]
                YIELDS.with(|count| count.set(count.get() + 1));
                tokio::task::yield_now().await;
            }
        }

        self.finish_serve(shutdown, tasks, task_error, receive_error)
            .await
    }

    async fn finish_serve(
        self,
        shutdown: TransportCancellation,
        mut tasks: JoinSet<()>,
        mut task_error: Option<String>,
        receive_error: Option<io::Error>,
    ) -> Result<(), UdpServerError> {
        shutdown.cancel();
        if let Some((_, cache)) = &self.owners {
            cache.stop_admission();
        }
        drain_tasks(&mut tasks, &mut task_error).await;
        if let Some((forwards, cache)) = &self.owners {
            if let Err(error) = cache.stop_refreshes().await {
                task_error = Some(error.to_string());
            }
            forwards.close_all().await;
        }
        if let Some(error) = task_error {
            return Err(UdpServerError::Task(error));
        }
        if let Some(error) = receive_error {
            return Err(UdpServerError::Receive(error));
        }
        Ok(())
    }

    /// [`UdpServer::handle_inline`], with a panic turned into the listener's
    /// task error.
    #[cfg(any(not(target_os = "linux"), test))]
    fn handle_caught(
        &self,
        tasks: &mut JoinSet<()>,
        raw: &[u8],
        peer: SocketAddr,
        shutdown: &TransportCancellation,
    ) -> Result<(), String> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.handle_inline(tasks, raw, peer, shutdown);
        }))
        .map_err(|panic| panic_message(panic.as_ref()))
    }

    #[cfg(target_os = "linux")]
    fn handle_caught_batched(
        &self,
        tasks: &mut JoinSet<()>,
        raw: &[u8],
        peer: SocketAddr,
        shutdown: &TransportCancellation,
        pending: &mut Vec<PendingSend>,
    ) -> Result<(), String> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.handle_inline_with(
                tasks,
                raw,
                peer,
                shutdown,
                |_tasks, wire, peer, admitted, admission| {
                    pending.push(PendingSend {
                        wire,
                        peer,
                        admitted,
                        admission: Some(admission),
                    });
                },
            );
        }))
        .map_err(|panic| panic_message(panic.as_ref()))
    }

    /// Handles one datagram without waiting. Whatever has to wait (an upstream
    /// exchange, a sleep, a branch policy, a rewrite lookup, a send the socket
    /// cannot take now) continues in a task of `tasks`.
    #[cfg(any(not(target_os = "linux"), test))]
    fn handle_inline(
        &self,
        tasks: &mut JoinSet<()>,
        raw: &[u8],
        peer: SocketAddr,
        shutdown: &TransportCancellation,
    ) {
        self.handle_inline_with(
            tasks,
            raw,
            peer,
            shutdown,
            |tasks, wire, peer, admitted, admission| {
                self.send_single(tasks, wire, peer, admitted, Some(admission), shutdown);
            },
        );
    }

    fn send_single(
        &self,
        tasks: &mut JoinSet<()>,
        wire: Vec<u8>,
        peer: SocketAddr,
        admitted: Option<AdmittedQueryGuard>,
        admission: Option<AdmittedSnapshot>,
        shutdown: &TransportCancellation,
    ) {
        match try_send(&self.socket, &wire, peer, shutdown) {
            TrySend::Done(outcome) => {
                if let Some(admitted) = admitted {
                    admitted.finish(outcome);
                }
                drop(admission);
            }
            TrySend::WouldBlock => {
                let socket = Arc::clone(&self.socket);
                let shutdown = shutdown.clone();
                tasks.spawn_local(async move {
                    let outcome = send_with_control(&shutdown, socket.send_to(&wire, peer)).await;
                    if let Some(admitted) = admitted {
                        admitted.finish(outcome);
                    }
                    drop(admission);
                });
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn flush_pending(
        &self,
        tasks: &mut JoinSet<()>,
        send_scratch: &mut crate::udp_batch::SendScratch,
        pending: &mut Vec<PendingSend>,
        shutdown: &TransportCancellation,
    ) {
        if pending.is_empty() {
            return;
        }
        // Single-reply low-load fast path: skip `sendmmsg` header setup and use
        // the exact same `try_send_to` path as per-datagram mode.
        if pending.len() == 1 {
            let mut item = pending.pop().expect("pending has one entry");
            self.send_single(
                tasks,
                item.wire,
                item.peer,
                item.admitted.take(),
                item.admission.take(),
                shutdown,
            );
            return;
        }
        let mut cursor = 0;
        while cursor < pending.len() {
            if shutdown.is_cancelled() {
                for mut item in pending.drain(cursor..) {
                    if let Some(admitted) = item.admitted.take() {
                        admitted.finish(QueryTerminalOutcome::Canceled);
                    }
                    drop(item.admission.take());
                }
                pending.clear();
                return;
            }
            let result =
                try_send_batch_slice(&self.socket, send_scratch, &pending[cursor..], shutdown);
            match result {
                Ok(sent) if sent > 0 => {
                    let sent = sent.min(pending.len() - cursor);
                    for item in &mut pending[cursor..cursor + sent] {
                        if let Some(admitted) = item.admitted.take() {
                            admitted.finish(QueryTerminalOutcome::SendSucceeded);
                        }
                        drop(item.admission.take());
                    }
                    cursor += sent;
                }
                Ok(_) => {
                    for mut item in pending.drain(cursor..) {
                        let socket = Arc::clone(&self.socket);
                        let shutdown = shutdown.clone();
                        let wire = item.wire;
                        let peer = item.peer;
                        let admitted = item.admitted.take();
                        let admission = item.admission.take();
                        tasks.spawn_local(async move {
                            let outcome =
                                send_with_control(&shutdown, socket.send_to(&wire, peer)).await;
                            if let Some(admitted) = admitted {
                                admitted.finish(outcome);
                            }
                            drop(admission);
                        });
                    }
                    pending.clear();
                    return;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    for mut item in pending.drain(cursor..) {
                        let socket = Arc::clone(&self.socket);
                        let shutdown = shutdown.clone();
                        let wire = item.wire;
                        let peer = item.peer;
                        let admitted = item.admitted.take();
                        let admission = item.admission.take();
                        tasks.spawn_local(async move {
                            let outcome =
                                send_with_control(&shutdown, socket.send_to(&wire, peer)).await;
                            if let Some(admitted) = admitted {
                                admitted.finish(outcome);
                            }
                            drop(admission);
                        });
                    }
                    pending.clear();
                    return;
                }
                Err(_) => {
                    // `sendmmsg` only returns -1 when the very first datagram in
                    // the slice fails; mark `pending[cursor]` failed and keep
                    // sending the remaining datagrams in the batch.
                    if let Some(admitted) = pending[cursor].admitted.take() {
                        admitted.finish(QueryTerminalOutcome::SendFailed);
                    }
                    drop(pending[cursor].admission.take());
                    cursor += 1;
                }
            }
        }
        pending.clear();
    }

    /// Runs the synchronous prefix of one datagram and hands any framed reply
    /// (with the raw `peer`) to `on_send`, or completes/hands off in place.
    fn handle_inline_with(
        &self,
        tasks: &mut JoinSet<()>,
        raw: &[u8],
        peer: SocketAddr,
        shutdown: &TransportCancellation,
        on_send: impl FnOnce(
            &mut JoinSet<()>,
            Vec<u8>,
            SocketAddr,
            Option<AdmittedQueryGuard>,
            AdmittedSnapshot,
        ),
    ) {
        #[cfg(test)]
        if PANIC_NEXT.with(|flag| flag.replace(false)) {
            panic!("injected request panic");
        }
        #[cfg(test)]
        if PANIC_COUNTDOWN.with(|countdown| match countdown.get() {
            Some(0) => {
                countdown.set(None);
                true
            }
            Some(remaining) => {
                countdown.set(Some(remaining - 1));
                false
            }
            None => false,
        }) {
            panic!("injected request panic");
        }
        let Some(mut admission) =
            self.control
                .capture(&self.listener_tag, ListenerKind::Udp, self.listener_addr)
        else {
            return;
        };
        // `capture` cloned these options for this request alone.
        admission.options.admission_deadline =
            Some(Instant::now() + admission.options.request_deadline);
        let (header, question) = match atomdns_dns_core::parse_query(raw) {
            Ok(parsed) => parsed,
            Err(error) => {
                // Malformed or unsupported input is isolated to this datagram. It is
                // answered with a header-only FORMERR/NOTIMP when it carries an ID
                // and is not itself a response, so clients do not wait for a timeout.
                self.observer.record_malformed();
                if let Some(reply) = atomdns_dns_core::query_error_reply(raw, error) {
                    if !shutdown.is_cancelled() {
                        on_send(tasks, reply, peer, None, admission);
                    }
                }
                return;
            }
        };

        // A dual-stack wildcard socket reports IPv4 peers as IPv4-mapped IPv6;
        // audit and client matching see the plain IPv4 address. Sending still
        // uses the raw `peer`.
        let client_addr = crate::bind::canonical_peer(peer);
        // The listener's own token: this query needs a token of its own only if
        // it moves to a task, where branch tokens are derived from it.
        let mut admitted = match self.observer.try_admit_on_listener(
            admission.enable_audit,
            client_addr,
            QueryTransport::Udp,
            &question,
            shutdown.clone(),
        ) {
            Ok(admitted) => admitted,
            Err(error) => {
                crate::diag::error_line!("native query admission failed: {error}");
                return;
            }
        };
        let udp_limit = crate::udp_response::payload_limit(raw);
        let capture_audit_details = admitted.execution_checkpoint().capture_audit_details();
        let snapshot = &admission.snapshot;
        let env = RootEnv {
            config: &snapshot.config,
            cache: &snapshot.cache,
            options: &admission.options,
            executor: snapshot.forwards.as_ref(),
            raw,
            request_shutdown: shutdown,
        };
        let request = ExecutionRequest {
            client: atomdns_sequence_core::ClientContext::from_peer(
                client_addr.ip(),
                atomdns_sequence_core::ClientTransport::Udp,
            ),
            config: env.config,
            cache: env.cache,
            options: env.options,
            raw,
            header,
            question,
        };
        // Runs until the query ends or has to wait; either way the borrows of
        // the admission and its checkpoint end here.
        let inline = match RootRun::start(request, admitted.execution_checkpoint(), shutdown) {
            Err(result) => Ok(result),
            Ok((mut run, step)) => match run.run_sync(&env, step) {
                SyncStop::Done(result) => Ok(result),
                SyncStop::Await(dispatch, wait) => Err(run.suspend(dispatch, wait)),
            },
        };
        let execution = match inline {
            Ok(result) => result,
            Err(suspended) => {
                tasks.spawn_local(resume_request(HandedOff {
                    suspended: Some(Box::new(suspended)),
                    socket: Arc::clone(&self.socket),
                    admitted: Some(admitted),
                    admission,
                    raw: raw.to_vec(),
                    peer,
                    udp_limit,
                    capture_audit_details,
                    shutdown: shutdown.clone(),
                }));
                return;
            }
        };
        let (observation, settled) = settle(
            execution,
            raw,
            udp_limit,
            capture_audit_details,
            shutdown.is_cancelled(),
        );
        admitted.capture_execution(observation);
        match settled {
            Settled::Terminal(outcome) => {
                admitted.finish(outcome);
                drop(admission);
            }
            Settled::Send(framed) => {
                if shutdown.is_cancelled() {
                    admitted.finish(QueryTerminalOutcome::Canceled);
                    drop(admission);
                    return;
                }
                on_send(tasks, framed, peer, Some(admitted), admission);
            }
        }
    }

    #[cfg(test)]
    fn spawn_legacy(
        &self,
        tasks: &mut JoinSet<()>,
        packet: &[u8],
        peer: SocketAddr,
        shutdown: &TransportCancellation,
    ) {
        let raw = packet.to_vec();
        let socket = Arc::clone(&self.socket);
        let Some(admission) =
            self.control
                .capture(&self.listener_tag, ListenerKind::Udp, self.listener_addr)
        else {
            return;
        };
        let config = admission.snapshot.config.clone();
        let forwards = admission.snapshot.forwards.clone();
        let cache = admission.snapshot.cache.clone();
        let observer = Arc::clone(&self.observer);
        let mut options = admission.options.clone();
        options.admission_deadline = Some(Instant::now() + options.request_deadline);
        let request_shutdown = shutdown.child_token();
        let enable_audit = admission.enable_audit;
        tasks.spawn_local(async move {
            process_request(RequestTask {
                socket,
                config,
                forwards,
                cache,
                observer,
                options,
                raw,
                peer,
                request_shutdown,
                enable_audit,
                _admission: admission,
            })
            .await;
        });
    }
}

#[cfg(target_os = "linux")]
struct PendingSend {
    wire: Vec<u8>,
    peer: SocketAddr,
    // Declared before `admission`: the query is recorded before its admitted
    // snapshot is released, as when a request task ends.
    admitted: Option<AdmittedQueryGuard>,
    admission: Option<AdmittedSnapshot>,
}

#[cfg(all(target_os = "linux", not(test)))]
fn try_send_batch_slice(
    socket: &UdpSocket,
    send_scratch: &mut crate::udp_batch::SendScratch,
    slice: &[PendingSend],
    _shutdown: &TransportCancellation,
) -> io::Result<usize> {
    send_scratch.try_send_batch(socket, slice.len(), |i| {
        (slice[i].wire.as_slice(), slice[i].peer)
    })
}

#[cfg(all(target_os = "linux", test))]
#[derive(Debug)]
enum InjectedBatchSend {
    /// Actually send the first `limit` datagrams of the slice, then return that
    /// partial count as if the kernel stopped there.
    RealThenStop(usize),
    /// Actually send the first `limit` datagrams of the slice, cancel
    /// `shutdown`, and return that partial count.
    RealThenCancel(usize),
    /// Return this error immediately without sending.
    Error(io::ErrorKind),
    /// Return `Ok(0)` without sending.
    Zero,
}

#[cfg(all(target_os = "linux", test))]
fn try_send_batch_slice(
    socket: &UdpSocket,
    send_scratch: &mut crate::udp_batch::SendScratch,
    slice: &[PendingSend],
    shutdown: &TransportCancellation,
) -> io::Result<usize> {
    BATCH_SEND_CALLS.with(|count| count.set(count.get() + 1));
    if let Some(injected) = INJECTED_BATCH_SENDS.with(|sends| sends.borrow_mut().pop_front()) {
        return match injected {
            InjectedBatchSend::RealThenStop(limit) => {
                let count = limit.min(slice.len());
                send_scratch
                    .try_send_batch(socket, count, |i| (slice[i].wire.as_slice(), slice[i].peer))
            }
            InjectedBatchSend::RealThenCancel(limit) => {
                let count = limit.min(slice.len());
                let sent = send_scratch
                    .try_send_batch(socket, count, |i| (slice[i].wire.as_slice(), slice[i].peer))?;
                shutdown.cancel();
                Ok(sent)
            }
            InjectedBatchSend::Error(kind) => Err(io::Error::from(kind)),
            InjectedBatchSend::Zero => Ok(0),
        };
    }
    send_scratch.try_send_batch(socket, slice.len(), |i| {
        (slice[i].wire.as_slice(), slice[i].peer)
    })
}

#[cfg(test)]
thread_local! {
    /// Test seam: the next inline datagram panics.
    static PANIC_NEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test seam: countdown of inline datagrams before panicking.
    static PANIC_COUNTDOWN: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    /// Test seam: results the next `try_send` calls return instead of sending.
    static INJECTED_SENDS: std::cell::RefCell<std::collections::VecDeque<io::ErrorKind>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    /// Test seam: scripted outcomes for Linux `try_send_batch_slice` calls.
    #[cfg(target_os = "linux")]
    static INJECTED_BATCH_SENDS: std::cell::RefCell<std::collections::VecDeque<InjectedBatchSend>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    /// How many times the Linux batched send path invoked `try_send_batch_slice`.
    #[cfg(target_os = "linux")]
    static BATCH_SEND_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How often the receive loop yielded after a burst.
    static YIELDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    let message = panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned());
    match message {
        Some(message) => format!("request panicked with message {message:?}"),
        None => "request panicked".to_owned(),
    }
}

/// A query that has to wait, handed to its own task with everything it has
/// done so far.
struct HandedOff {
    /// Boxed: the paused execution is large, and the task's state would
    /// otherwise carry it beside the execution it becomes.
    suspended: Option<Box<SuspendedRoot>>,
    socket: Arc<UdpSocket>,
    // Declared before `admission`: the query is recorded before its admitted
    // snapshot is released, as when a request task ends.
    admitted: Option<AdmittedQueryGuard>,
    admission: AdmittedSnapshot,
    raw: Vec<u8>,
    peer: SocketAddr,
    udp_limit: usize,
    capture_audit_details: bool,
    /// The listener's token; the query derives its own from it.
    shutdown: TransportCancellation,
}

impl Drop for HandedOff {
    /// A task dropped before it ran still reports what the query did so far,
    /// as a dropped execution does.
    fn drop(&mut self) {
        if let (Some(suspended), Some(admitted)) = (self.suspended.take(), self.admitted.as_mut()) {
            suspended.abandon(
                &self.admission.snapshot.config,
                admitted.execution_checkpoint(),
            );
        }
    }
}

/// Continues a handed-off query from the dispatch it stopped on.
async fn resume_request(mut handed: HandedOff) {
    let Some(suspended) = handed.suspended.take() else {
        return;
    };
    let Some(admitted) = handed.admitted.as_mut() else {
        return;
    };
    // From here on the query has a token of its own, as a request task has.
    let request_shutdown = handed.shutdown.child_token();
    admitted.replace_cancellation(request_shutdown.clone());
    let snapshot = &handed.admission.snapshot;
    let env = RootEnv {
        config: &snapshot.config,
        cache: &snapshot.cache,
        options: &handed.admission.options,
        executor: snapshot.forwards.as_ref(),
        raw: &handed.raw,
        request_shutdown: &request_shutdown,
    };
    let execution = continue_execution(
        suspended,
        &env,
        admitted.execution_checkpoint(),
        &request_shutdown,
    )
    .await;
    let (observation, settled) = settle(
        execution,
        &handed.raw,
        handed.udp_limit,
        handed.capture_audit_details,
        request_shutdown.is_cancelled(),
    );
    admitted.capture_execution(observation);
    let outcome = match settled {
        Settled::Terminal(outcome) => outcome,
        Settled::Send(framed) => {
            send_response(
                Arc::clone(&handed.socket),
                framed,
                handed.peer,
                request_shutdown,
            )
            .await
        }
    };
    if let Some(admitted) = handed.admitted.take() {
        admitted.finish(outcome);
    }
}

/// Binds the suspended execution again and runs it to its end.
async fn continue_execution(
    suspended: Box<SuspendedRoot>,
    env: &RootEnv<'_, ForwardCatalog>,
    checkpoint: &mut crate::observer::ExecutionCheckpoint,
    request_shutdown: &TransportCancellation,
) -> ExecutionResult {
    // The paused execution moves out of its box here, before the first await,
    // so the task's state holds it only once.
    let (mut run, dispatch, wait) =
        match (*suspended).resume(env.config, checkpoint, request_shutdown) {
            Ok(resumed) => resumed,
            Err(result) => return result,
        };
    run.drive_from(env, dispatch, wait).await
}

/// How a finished execution leaves the listener.
enum Settled {
    /// The query ended without a reply to send.
    Terminal(QueryTerminalOutcome),
    /// The framed reply to send.
    Send(Vec<u8>),
}

/// Frames the reply of a finished execution and builds its terminal
/// observation. `canceled` is whether the request's token was canceled.
fn settle(
    mut execution: ExecutionResult,
    raw: &[u8],
    udp_limit: usize,
    capture_audit_details: bool,
    canceled: bool,
) -> (TerminalObservation, Settled) {
    let response_formed = matches!(
        &execution.response,
        crate::observer::ResponseState::Dns { .. }
    );
    let mut framed_response = None;
    let terminal = if canceled {
        Some(QueryTerminalOutcome::Canceled)
    } else if !response_formed {
        Some(QueryTerminalOutcome::NoResponse)
    } else {
        match crate::udp_response::prepare_response(
            std::mem::take(&mut execution.response_wire),
            udp_limit,
            raw,
        ) {
            Ok((framed, records_removed)) => {
                if records_removed && capture_audit_details {
                    execution.response_details =
                        crate::execution::observe_and_diagnose_wire(&framed).1;
                }
                framed_response = Some(framed);
                None
            }
            Err(_) => {
                execution.failure_provenance =
                    Some(crate::observer::FailureProvenance::LocalFailure(
                        crate::observer::LocalFailureKind::ResponseConstruction,
                    ));
                Some(QueryTerminalOutcome::SendFailed)
            }
        }
    };
    let observation = TerminalObservation {
        outcome: terminal.unwrap_or(QueryTerminalOutcome::NoResponse),
        response: execution.response,
        response_details: execution.response_details,
        cache_status: execution.cache_status,
        final_sequence: execution.final_sequence,
        matched_group: execution.matched_group,
        domain_set: execution.domain_set,
        effective_tag: execution.effective_tag,
        matched_rule_source: execution.matched_rule_source,
        final_upstream: execution.final_upstream,
        upstream_targets: execution.upstream_targets,
        selected_upstream: execution.selected_upstream,
        upstream_attempts: execution.upstream_attempts,
        upstream_diagnostics: execution.upstream_diagnostics,
        failure_provenance: execution.failure_provenance,
        elapsed: std::time::Duration::ZERO,
    };
    let settled = match (terminal, framed_response) {
        (None, Some(framed)) => Settled::Send(framed),
        (terminal, _) => Settled::Terminal(terminal.unwrap_or(QueryTerminalOutcome::NoResponse)),
    };
    (observation, settled)
}

enum TrySend {
    Done(QueryTerminalOutcome),
    /// The socket cannot take the datagram now; send it with `send_to`.
    WouldBlock,
}

/// Sends without waiting. Cancellation wins before the send, as in
/// [`send_with_control`]; `try_send_to` is what the first poll of `send_to`
/// does, so only a full socket buffer leaves the send to an async task.
fn try_send(
    socket: &UdpSocket,
    wire: &[u8],
    peer: SocketAddr,
    shutdown: &TransportCancellation,
) -> TrySend {
    if shutdown.is_cancelled() {
        return TrySend::Done(QueryTerminalOutcome::Canceled);
    }
    #[cfg(test)]
    let injected = INJECTED_SENDS.with(|sends| sends.borrow_mut().pop_front());
    #[cfg(not(test))]
    let injected: Option<io::ErrorKind> = None;
    let result = match injected {
        Some(kind) => Err(io::Error::from(kind)),
        None => socket.try_send_to(wire, peer),
    };
    match result {
        Ok(_) => TrySend::Done(QueryTerminalOutcome::SendSucceeded),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => TrySend::WouldBlock,
        Err(_) => TrySend::Done(QueryTerminalOutcome::SendFailed),
    }
}

#[cfg(test)]
struct RequestTask {
    socket: Arc<UdpSocket>,
    config: Rc<CompiledConfig>,
    forwards: Rc<ForwardCatalog>,
    cache: Rc<CacheCatalog>,
    observer: Arc<QueryObserver>,
    options: HostOptions,
    raw: Vec<u8>,
    peer: SocketAddr,
    request_shutdown: TransportCancellation,
    enable_audit: bool,
    _admission: crate::runtime_snapshot::AdmittedSnapshot,
}

#[cfg(test)]
async fn process_request(task: RequestTask) {
    let RequestTask {
        socket,
        config,
        forwards,
        cache,
        observer,
        options,
        raw,
        peer,
        request_shutdown,
        enable_audit,
        _admission,
    } = task;
    let (header, question) = match atomdns_dns_core::parse_query(&raw) {
        Ok(parsed) => parsed,
        Err(error) => {
            // Malformed or unsupported input is isolated to this datagram. It is
            // answered with a header-only FORMERR/NOTIMP when it carries an ID
            // and is not itself a response, so clients do not wait for a timeout.
            observer.record_malformed();
            if let Some(reply) = atomdns_dns_core::query_error_reply(&raw, error) {
                let _ = send_with_control(&request_shutdown, socket.send_to(&reply, peer)).await;
            }
            return;
        }
    };

    // A dual-stack wildcard socket reports IPv4 peers as IPv4-mapped IPv6;
    // audit and client matching see the plain IPv4 address.
    let client_addr = crate::bind::canonical_peer(peer);
    let mut admitted = match observer.try_admit_on_listener(
        enable_audit,
        client_addr,
        QueryTransport::Udp,
        &question,
        request_shutdown.clone(),
    ) {
        Ok(admitted) => admitted,
        Err(error) => {
            crate::diag::error_line!("native query admission failed: {error}");
            return;
        }
    };
    let udp_limit = crate::udp_response::payload_limit(&raw);
    let capture_audit_details = admitted.execution_checkpoint().capture_audit_details();
    let mut execution = execute_request(
        ExecutionRequest {
            client: atomdns_sequence_core::ClientContext::from_peer(
                client_addr.ip(),
                atomdns_sequence_core::ClientTransport::Udp,
            ),
            config: &config,
            cache: &cache,
            options: &options,
            raw: &raw,
            header,
            question,
        },
        &forwards,
        request_shutdown.clone(),
        admitted.execution_checkpoint(),
    )
    .await;
    let response_formed = matches!(
        &execution.response,
        crate::observer::ResponseState::Dns { .. }
    );
    let mut framed_response = None;
    let terminal = if request_shutdown.is_cancelled() {
        Some(QueryTerminalOutcome::Canceled)
    } else if !response_formed {
        Some(QueryTerminalOutcome::NoResponse)
    } else {
        match crate::udp_response::prepare_response(
            std::mem::take(&mut execution.response_wire),
            udp_limit,
            &raw,
        ) {
            Ok((framed, records_removed)) => {
                if records_removed && capture_audit_details {
                    execution.response_details =
                        crate::execution::observe_and_diagnose_wire(&framed).1;
                }
                framed_response = Some(framed);
                None
            }
            Err(_) => {
                execution.failure_provenance =
                    Some(crate::observer::FailureProvenance::LocalFailure(
                        crate::observer::LocalFailureKind::ResponseConstruction,
                    ));
                Some(QueryTerminalOutcome::SendFailed)
            }
        }
    };
    admitted.capture_execution(TerminalObservation {
        outcome: terminal.unwrap_or(QueryTerminalOutcome::NoResponse),
        response: execution.response,
        response_details: execution.response_details,
        cache_status: execution.cache_status,
        final_sequence: execution.final_sequence,
        matched_group: execution.matched_group,
        domain_set: execution.domain_set,
        effective_tag: execution.effective_tag,
        matched_rule_source: execution.matched_rule_source,
        final_upstream: execution.final_upstream,
        upstream_targets: execution.upstream_targets,
        selected_upstream: execution.selected_upstream,
        upstream_attempts: execution.upstream_attempts,
        upstream_diagnostics: execution.upstream_diagnostics,
        failure_provenance: execution.failure_provenance,
        elapsed: std::time::Duration::ZERO,
    });
    let terminal = match terminal {
        Some(outcome) => outcome,
        None => {
            send_response(
                socket,
                framed_response.expect("response is framed before the send await"),
                peer,
                request_shutdown,
            )
            .await
        }
    };
    admitted.finish(terminal);
}

async fn send_response(
    socket: Arc<UdpSocket>,
    response: Vec<u8>,
    peer: SocketAddr,
    request_shutdown: TransportCancellation,
) -> QueryTerminalOutcome {
    send_response_after_gate(
        socket,
        response,
        peer,
        request_shutdown,
        std::future::ready(()),
    )
    .await
}

async fn send_response_after_gate<F>(
    socket: Arc<UdpSocket>,
    response: Vec<u8>,
    peer: SocketAddr,
    request_shutdown: TransportCancellation,
    before_send: F,
) -> QueryTerminalOutcome
where
    F: Future<Output = ()>,
{
    before_send.await;
    send_with_control(&request_shutdown, socket.send_to(&response, peer)).await
}

async fn send_with_control<F>(
    request_shutdown: &TransportCancellation,
    send: F,
) -> QueryTerminalOutcome
where
    F: Future<Output = io::Result<usize>>,
{
    tokio::select! {
        biased;
        () = request_shutdown.cancelled() => QueryTerminalOutcome::Canceled,
        result = send => if result.is_ok() {
            QueryTerminalOutcome::SendSucceeded
        } else {
            QueryTerminalOutcome::SendFailed
        },
    }
}

fn record_task_result(result: Result<(), tokio::task::JoinError>, task_error: &mut Option<String>) {
    if let Err(error) = result {
        if task_error.is_none() {
            *task_error = Some(error.to_string());
        }
    }
}

pub(crate) async fn reap_one_task(
    tasks: &mut JoinSet<()>,
    task_error: &mut Option<String>,
) -> bool {
    let Some(result) = tasks.join_next().await else {
        return false;
    };
    record_task_result(result, task_error);
    true
}

pub(crate) async fn drain_tasks(tasks: &mut JoinSet<()>, task_error: &mut Option<String>) {
    while reap_one_task(tasks, task_error).await {}
}

#[cfg(test)]
async fn finish_server(
    forwards: &ForwardCatalog,
    tasks: &mut JoinSet<()>,
    task_error: &mut Option<String>,
    receive_error: Option<std::io::Error>,
) -> Result<(), UdpServerError> {
    drain_tasks(tasks, task_error).await;
    forwards.close_all().await;
    if let Some(error) = task_error {
        return Err(UdpServerError::Task(error.clone()));
    }
    if let Some(error) = receive_error {
        return Err(UdpServerError::Receive(error));
    }
    Ok(())
}

/// Failures that can occur while owning the UDP listener.
#[derive(Debug)]
pub enum UdpServerError {
    WrongListener(ListenerKind),
    Bind(std::io::Error),
    LocalAddr(std::io::Error),
    Receive(std::io::Error),
    Task(String),
}

impl fmt::Display for UdpServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongListener(kind) => write!(formatter, "listener is not UDP: {kind:?}"),
            Self::Bind(error) => write!(formatter, "UDP bind failed: {error}"),
            Self::LocalAddr(error) => write!(formatter, "UDP local address failed: {error}"),
            Self::Receive(error) => write!(formatter, "UDP receive failed: {error}"),
            Self::Task(error) => write!(formatter, "UDP request task failed: {error}"),
        }
    }
}

impl std::error::Error for UdpServerError {}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use crate::observer::{
        CacheStatus, QueryObserver, QueryTerminalOutcome, QueryTransport, ResponseSource,
        ResponseState, TerminalObservation,
    };
    use atomdns_dns_core::{parse_query, synthesize_response, validate_response};
    use atomdns_sequence_core::{ExecutionControl, ExecutionState, ExecutorOutcome, MachineStep};
    use atomdns_upstream_core::{LifecycleState, TransportCancellation};
    use tokio::net::UdpSocket;
    use tokio::sync::Notify;
    use tokio::task::JoinSet;

    use super::{
        REFUSED, drain_tasks, finish_server, reap_one_task, send_response_after_gate,
        send_with_control,
    };
    use crate::assembly::{HostAssembly, HostRuntime};
    use crate::config::compile_yaml;
    use crate::execution::response_from_state;

    const UDP_CONFIG: &str =
        include_str!("../../../tests/phase5a-baseline/configs/forward-udp.yaml");

    #[test]
    fn a_completed_sequence_without_response_maps_to_refused() {
        let config = compile_yaml(UDP_CONFIG).expect("frozen UDP config");
        let query = [
            0x50, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, b'n',
            b'o', b'p', 0x00, 0x00, 0x01, 0x00, 0x01,
        ];
        let (header, question) = parse_query(&query).expect("valid query");
        let state = ExecutionState::new(header, question.clone());
        let mut machine = config
            .new_machine(state, ExecutionControl::with_fuel(8))
            .expect("compiled sequence");
        let dispatch = match machine.step().expect("forward dispatch") {
            MachineStep::Dispatch(dispatch) => dispatch,
            MachineStep::ScopeComplete(completion) => {
                panic!("unwatched machine yielded {completion:?}")
            }
            MachineStep::ScopeAborted(completion) => {
                panic!("unwatched machine aborted {completion:?}")
            }
            MachineStep::Complete(_) => panic!("forward must dispatch"),
        };
        let completion = machine
            .resume(dispatch.executable(), Ok(ExecutorOutcome::Continue))
            .expect("resume without a response");
        assert!(matches!(completion, MachineStep::Complete(_)));

        let response = response_from_state(&mut machine, &header, &question);
        validate_response(&response).expect("REFUSED response must be valid");
        assert_eq!(response[3] & 0x0f, REFUSED);
    }

    #[test]
    fn completed_tasks_are_reaped_and_failures_do_not_skip_remaining_tasks() {
        let runtime = HostRuntime::new().expect("test runtime");
        let observed = Arc::new(AtomicBool::new(false));
        runtime.block_on(async {
            let mut tasks = JoinSet::new();
            for _ in 0..128 {
                tasks.spawn_local(async {});
            }
            let mut task_error = None;
            let mut reaped = 0;
            while reap_one_task(&mut tasks, &mut task_error).await {
                reaped += 1;
            }
            assert_eq!(reaped, 128);
            assert!(tasks.is_empty());
            assert!(task_error.is_none());

            let panic_task = tasks.spawn_local(async {
                panic!("intentional task failure");
            });
            let observed_by_task = Arc::clone(&observed);
            tasks.spawn_local(async move {
                observed_by_task.store(true, Ordering::SeqCst);
            });
            let mut task_error = None;
            drain_tasks(&mut tasks, &mut task_error).await;
            assert!(panic_task.is_finished());
            assert!(observed.load(Ordering::SeqCst));
            assert!(task_error.is_some());
            assert!(tasks.is_empty());
        });
    }

    #[test]
    fn task_failure_still_closes_upstream_after_full_drain() {
        let runtime = HostRuntime::new().expect("test runtime");
        let host = HostAssembly::from_yaml(UDP_CONFIG).expect("frozen UDP config");
        runtime.block_on(async {
            let mut tasks = JoinSet::new();
            tasks.spawn_local(async {
                panic!("intentional task failure");
            });
            let mut task_error = None;
            let result =
                finish_server(&host.forwards_handle(), &mut tasks, &mut task_error, None).await;
            assert!(matches!(result, Err(super::UdpServerError::Task(_))));
            assert!(tasks.is_empty());
            assert_eq!(
                host.forward()
                    .expect("primary forward")
                    .upstream()
                    .lifecycle_state(),
                LifecycleState::Closed
            );
        });
    }

    #[test]
    fn cancellation_wins_at_the_pre_send_commit_gate() {
        let runtime = HostRuntime::new().expect("test runtime");
        runtime.block_on(async {
            let sender = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("sender socket"));
            let receiver = UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("receiver socket");
            let peer = receiver.local_addr().expect("receiver address");
            let cancellation = TransportCancellation::new();
            let observer = Arc::new(QueryObserver::new(true, ["phase5a_forward".to_owned()], 2));
            let question = atomdns_dns_core::QuestionInfo {
                qname_wire: vec![3, b'n', b'o', b'p', 0],
                qtype: 1,
                qclass: 1,
            };
            let mut admitted =
                observer.admit(peer, QueryTransport::Udp, &question, cancellation.clone());
            let gate = Arc::new(Notify::new());
            let send = tokio::task::spawn_local(send_response_after_gate(
                sender,
                vec![0xaa, 0xbb],
                peer,
                cancellation.clone(),
                Arc::clone(&gate).notified_owned(),
            ));

            cancellation.cancel();
            gate.notify_one();
            let outcome = send.await.expect("send task");
            assert_eq!(outcome, QueryTerminalOutcome::Canceled);
            admitted.capture_execution(TerminalObservation {
                outcome,
                response: ResponseState::NoResponse,
                cache_status: CacheStatus::NotApplicable,
                final_sequence: None,
                matched_group: None,
                final_upstream: None,
                upstream_attempts: crate::observer::UpstreamAttemptList::default(),
                failure_provenance: None,
                elapsed: std::time::Duration::ZERO,
                ..Default::default()
            });
            admitted.finish(outcome);
            let metrics = observer.metrics_snapshot();
            assert_eq!(metrics.completed_total, 1);
            assert_eq!(metrics.canceled_total, 1);
            assert_eq!(metrics.send_succeeded_total, 0);
            assert!(
                tokio::time::timeout(
                    std::time::Duration::from_millis(20),
                    receiver.recv_from(&mut [0_u8; 8]),
                )
                .await
                .is_err()
            );
        });
    }

    #[test]
    fn udp_send_failure_and_pending_send_cancellation_are_distinct() {
        let runtime = HostRuntime::new().expect("test runtime");
        runtime.block_on(async {
            let cancellation = TransportCancellation::new();
            let failure = send_with_control(&cancellation, async {
                Err(std::io::Error::other("injected UDP send failure"))
            })
            .await;
            assert_eq!(failure, QueryTerminalOutcome::SendFailed);

            let cancellation = TransportCancellation::new();
            cancellation.cancel();
            let canceled = send_with_control(&cancellation, std::future::pending()).await;
            assert_eq!(canceled, QueryTerminalOutcome::Canceled);
        });
    }

    #[test]
    fn udp_listener_records_one_terminal_audit_for_the_actual_request() {
        let upstream = std::net::UdpSocket::bind("127.0.0.1:0").expect("upstream socket");
        upstream.set_nonblocking(true).expect("nonblocking socket");
        let upstream_addr = upstream.local_addr().expect("upstream address");
        let yaml = UDP_CONFIG
            .replace("127.0.0.1:15453", &upstream_addr.to_string())
            .replace("enable_audit: false", "enable_audit: true");
        let assembly = HostAssembly::from_yaml(&yaml).expect("native host");

        assembly.block_on(async {
            let upstream = UdpSocket::from_std(upstream).expect("Tokio upstream socket");
            let upstream_task = tokio::task::spawn_local(async move {
                let mut request = [0_u8; 2048];
                let (length, client) = upstream
                    .recv_from(&mut request)
                    .await
                    .expect("receive query");
                let (header, question) = parse_query(&request[..length]).expect("upstream query");
                let response = synthesize_response(&header, &question, 0).expect("DNS response");
                upstream
                    .send_to(&response, client)
                    .await
                    .expect("send response");
            });

            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let shutdown = TransportCancellation::new();
            let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
            let client = UdpSocket::bind("127.0.0.1:0").await.expect("client socket");
            let client_addr = client.local_addr().expect("client address");
            let query = [
                0x31, 0x41, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, b'w',
                b'w', b'w', 0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0x00, 0x00, 0x01, 0x00,
                0x01,
            ];
            client
                .send_to(&query, server_addr)
                .await
                .expect("send query");
            let mut response = [0_u8; 2048];
            let (length, _) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                client.recv_from(&mut response),
            )
            .await
            .expect("listener response timeout")
            .expect("receive response");
            validate_response(&response[..length]).expect("valid response");

            while assembly.metrics_snapshot().completed_total != 1 {
                tokio::task::yield_now().await;
            }
            let metrics = assembly.metrics_snapshot();
            assert_eq!(metrics.admitted_total, 1);
            assert_eq!(metrics.completed_total, 1);
            assert_eq!(metrics.in_flight, 0);
            assert_eq!(metrics.send_succeeded_total, 1);
            assert_eq!(metrics.malformed_total, 0);
            assert_eq!(
                metrics.forward_attempts_by_upstream["phase5a_forward"].responses_total,
                1
            );

            let audit = assembly.audit_snapshot();
            assert_eq!(audit.records.len(), 1);
            let record = &audit.records[0];
            assert_eq!(record.client_addr, client_addr);
            assert_eq!(record.transport, QueryTransport::Udp);
            assert_eq!(record.qname, "www.example.");
            assert_eq!(record.qtype, 1);
            assert_eq!(record.qclass, 1);
            assert_eq!(record.terminal_outcome, QueryTerminalOutcome::SendSucceeded);
            assert_eq!(
                record.response,
                ResponseState::Dns {
                    rcode: 0,
                    source: ResponseSource::Upstream("phase5a_forward".into())
                }
            );
            assert_eq!(record.cache_status, CacheStatus::NotApplicable);
            assert_eq!(record.final_sequence.as_deref(), Some("phase5a_entry"));
            assert_eq!(record.final_upstream.as_deref(), Some("phase5a_forward"));
            assert_eq!(record.upstream_attempts.len(), 1);

            client
                .send_to(&[0_u8, 0_u8], server_addr)
                .await
                .expect("send malformed datagram");
            while assembly.metrics_snapshot().malformed_total != 1 {
                tokio::task::yield_now().await;
            }
            let metrics = assembly.metrics_snapshot();
            assert_eq!(metrics.admitted_total, 1);
            assert_eq!(metrics.completed_total, 1);
            assert_eq!(metrics.malformed_total, 1);
            assert_eq!(assembly.audit_snapshot().records.len(), 1);

            shutdown.cancel();
            assert!(server_task.await.expect("server task").is_ok());
            upstream_task.await.expect("upstream task");
        });
    }

    // ---- the inline receive path against the task-per-datagram path ----

    use super::{DispatchMode, INJECTED_SENDS, PANIC_NEXT, YIELDS};
    use crate::assembly::HostOptions;
    use crate::cache::CacheTestClock;
    use crate::observer::TraceId;
    use std::cell::Cell;
    use std::net::SocketAddr;
    use std::rc::Rc;
    use std::time::Duration;

    fn named_query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
        let mut wire = vec![
            (id >> 8) as u8,
            id as u8,
            0x01,
            0x00,
            0x00,
            0x01,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
        ];
        for label in name.trim_end_matches('.').split('.') {
            wire.push(label.len() as u8);
            wire.extend_from_slice(label.as_bytes());
        }
        wire.push(0);
        wire.extend_from_slice(&qtype.to_be_bytes());
        wire.extend_from_slice(&1_u16.to_be_bytes());
        wire
    }

    /// An answer with `count` A records from 192.0.2.1 on.
    fn a_answer(query: &[u8], ttl: u32, count: usize) -> Vec<u8> {
        let (_, question) = parse_query(query).expect("upstream query");
        let end = 12 + question.qname_wire.len() + 4;
        let mut wire = query[..end].to_vec();
        wire[2] |= 0x80;
        wire[3] = 0x80;
        wire[6..8].copy_from_slice(&(count as u16).to_be_bytes());
        wire[8..12].fill(0);
        for index in 0..count {
            wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
            wire.extend_from_slice(&ttl.to_be_bytes());
            wire.extend_from_slice(&[0x00, 0x04, 192, 0, 2, 1 + index as u8]);
        }
        wire
    }

    /// Answers every query with one A record (TTL 10), `big.test` with forty,
    /// and never answers `silent.test`.
    async fn fake_upstream(socket: UdpSocket, received: Rc<Cell<usize>>) {
        let mut buffer = [0_u8; 2048];
        loop {
            let Ok((length, peer)) = socket.recv_from(&mut buffer).await else {
                return;
            };
            received.set(received.get() + 1);
            let query = &buffer[..length];
            let Ok((_, question)) = parse_query(query) else {
                continue;
            };
            let name = crate::matchers::wire_name_to_ascii_domain(&question.qname_wire)
                .unwrap_or_default();
            let reply = match name.trim_end_matches('.') {
                "silent.test" => continue,
                "big.test" => a_answer(query, 10, 40),
                _ => a_answer(query, 10, 1),
            };
            let _ = socket.send_to(&reply, peer).await;
        }
    }

    fn twin_yaml(upstream: SocketAddr, rules: &std::path::Path) -> String {
        format!(
            "log: {{level: error}}\nplugins:\n  - tag: c\n    type: cache\n    args: {{size: 64, lazy_cache_ttl: 300}}\n  - tag: f\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://{upstream}'}}]}}\n  - tag: rw\n    type: rewrite\n    args: {{files: ['{}'], dns: '{upstream}'}}\n  - tag: main\n    type: sequence\n    args:\n      - exec: $rw\n      - matches: qname blocked.test\n        exec: reject 3\n      - exec: $c\n      - exec: $f\n  - tag: dns\n    type: udp_server\n    args: {{listen: '127.0.0.1:15999', entry: main, enable_audit: true}}\n",
            rules.display()
        )
    }

    enum TwinStep {
        Query(Vec<u8>),
        /// A query whose reply is not awaited (the upstream stays silent).
        Unanswered(Vec<u8>),
        AdvanceClock(u64),
        /// Waits until the cache has no refresh pending.
        RefreshDone,
        /// The next inline send reports this error (the task-per-datagram
        /// path has no inline send, so it ignores the step).
        InjectSend(std::io::ErrorKind),
        /// Cancels the listener and waits for it to finish.
        Shutdown,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct TwinOutput {
        replies: Vec<Option<Vec<u8>>>,
        audit: Vec<String>,
        metrics: String,
        upstream_queries: usize,
        serve_result: String,
    }

    fn normalized_audit(assembly: &HostAssembly) -> Vec<String> {
        assembly
            .audit_snapshot()
            .records
            .into_iter()
            .map(|mut record| {
                record.timestamp = std::time::UNIX_EPOCH;
                record.elapsed = Duration::ZERO;
                record.trace_id = TraceId::Text(String::new());
                format!("{record:?}")
            })
            .collect()
    }

    fn normalized_metrics(assembly: &HostAssembly) -> String {
        let mut metrics = assembly.metrics_snapshot();
        metrics.duration = Default::default();
        for upstream in metrics.forward_attempts_by_upstream.values_mut() {
            upstream.latency_buckets = Default::default();
            upstream.latency_sum_ms = 0;
        }
        format!("{metrics:?}")
    }

    fn run_twin(
        mode: DispatchMode,
        upstream: &std::net::UdpSocket,
        client: &std::net::UdpSocket,
        rules: &std::path::Path,
        steps: Vec<TwinStep>,
    ) -> TwinOutput {
        let upstream_addr = upstream.local_addr().expect("upstream address");
        let clock = CacheTestClock::new(1000);
        let assembly = HostAssembly::with_options(
            compile_yaml(&twin_yaml(upstream_addr, rules)).expect("twin config"),
            HostOptions::default().with_cache_clock(Rc::new(clock.clone())),
        )
        .expect("twin host");
        let upstream = upstream.try_clone().expect("upstream clone");
        let client = client.try_clone().expect("client clone");
        assembly.block_on(async {
            let received = Rc::new(Cell::new(0));
            let upstream = UdpSocket::from_std(upstream).expect("tokio upstream");
            let upstream_task =
                tokio::task::spawn_local(fake_upstream(upstream, Rc::clone(&received)));
            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener")
                    .with_mode(mode);
            let server_addr = server.local_addr().expect("bound listener");
            let shutdown = TransportCancellation::new();
            let mut server_task = Some(tokio::task::spawn_local(server.serve(shutdown.clone())));
            let client = UdpSocket::from_std(client).expect("tokio client");
            let mut replies = Vec::new();
            let mut serve_result = String::new();
            for step in steps {
                match step {
                    TwinStep::Query(query) => {
                        let completed = assembly.metrics_snapshot().completed_total
                            + assembly.metrics_snapshot().malformed_total;
                        client
                            .send_to(&query, server_addr)
                            .await
                            .expect("send query");
                        let mut reply = [0_u8; 4096];
                        let received = tokio::time::timeout(
                            Duration::from_millis(500),
                            client.recv_from(&mut reply),
                        )
                        .await
                        .ok()
                        .map(|result| reply[..result.expect("receive").0].to_vec());
                        replies.push(received);
                        // The terminal record follows the send.
                        while assembly.metrics_snapshot().completed_total
                            + assembly.metrics_snapshot().malformed_total
                            == completed
                        {
                            tokio::task::yield_now().await;
                        }
                    }
                    TwinStep::Unanswered(query) => {
                        let admitted = assembly.metrics_snapshot().admitted_total;
                        client
                            .send_to(&query, server_addr)
                            .await
                            .expect("send query");
                        while assembly.metrics_snapshot().admitted_total == admitted {
                            tokio::task::yield_now().await;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    TwinStep::AdvanceClock(seconds) => clock.advance(seconds),
                    TwinStep::RefreshDone => {
                        let store = assembly
                            .cache()
                            .get(crate::config::CacheId(0))
                            .expect("cache store")
                            .clone();
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        while store.pending_refreshes() != 0 {
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                    }
                    TwinStep::InjectSend(kind) => {
                        if mode != DispatchMode::Legacy {
                            INJECTED_SENDS.with(|sends| sends.borrow_mut().push_back(kind));
                        }
                    }
                    TwinStep::Shutdown => {
                        shutdown.cancel();
                        if let Some(task) = server_task.take() {
                            serve_result = format!("{:?}", task.await.expect("server task"));
                        }
                    }
                }
            }
            if let Some(task) = server_task.take() {
                shutdown.cancel();
                serve_result = format!("{:?}", task.await.expect("server task"));
            }
            upstream_task.abort();
            INJECTED_SENDS.with(|sends| sends.borrow_mut().clear());
            TwinOutput {
                replies,
                audit: normalized_audit(&assembly),
                metrics: normalized_metrics(&assembly),
                upstream_queries: received.get(),
                serve_result,
            }
        })
    }

    struct Twins {
        upstream: std::net::UdpSocket,
        client: std::net::UdpSocket,
        rules: std::path::PathBuf,
    }

    impl Twins {
        fn new(label: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("udp-inline-{label}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("rule directory");
            let rules = dir.join("rewrite.txt");
            std::fs::write(
                &rules,
                "domain:ip.test 192.0.2.9\ndomain:alias.test target.test\n",
            )
            .expect("rewrite rules");
            let upstream = std::net::UdpSocket::bind("127.0.0.1:0").expect("upstream socket");
            upstream
                .set_nonblocking(true)
                .expect("nonblocking upstream");
            let client = std::net::UdpSocket::bind("127.0.0.1:0").expect("client socket");
            client.set_nonblocking(true).expect("nonblocking client");
            Self {
                upstream,
                client,
                rules,
            }
        }

        /// Runs the same steps on the inline path and on the
        /// task-per-datagram path (plus per-datagram inline on Linux) and
        /// returns the inline output after checking that all are identical.
        fn run(&self, steps: impl Fn() -> Vec<TwinStep>) -> TwinOutput {
            let inline = run_twin(
                DispatchMode::Inline,
                &self.upstream,
                &self.client,
                &self.rules,
                steps(),
            );
            #[cfg(target_os = "linux")]
            {
                let single = run_twin(
                    DispatchMode::SingleInline,
                    &self.upstream,
                    &self.client,
                    &self.rules,
                    steps(),
                );
                assert_eq!(inline, single, "batched and single-inline paths differ");
            }
            let legacy = run_twin(
                DispatchMode::Legacy,
                &self.upstream,
                &self.client,
                &self.rules,
                steps(),
            );
            assert_eq!(inline, legacy, "inline and task-per-datagram paths differ");
            inline
        }
    }

    fn header_only_query(id: u16) -> Vec<u8> {
        vec![
            (id >> 8) as u8,
            id as u8,
            0x01,
            0x00,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ]
    }

    #[test]
    fn the_inline_path_answers_and_records_exactly_like_one_task_per_datagram() {
        let twins = Twins::new("script");
        let output = twins.run(|| {
            vec![
                // Miss: waits for the upstream in a task.
                TwinStep::Query(named_query(1, "cached.test", 1)),
                // Fresh hit: answered inline.
                TwinStep::Query(named_query(2, "cached.test", 1)),
                // Stale hit: answered inline and starts one refresh.
                TwinStep::AdvanceClock(11),
                TwinStep::Query(named_query(3, "cached.test", 1)),
                TwinStep::RefreshDone,
                // Local answers.
                TwinStep::Query(named_query(4, "blocked.test", 1)),
                TwinStep::Query(named_query(5, "ip.test", 1)),
                // A rewrite to a domain labels inline and resolves in a task.
                TwinStep::Query(named_query(6, "alias.test", 1)),
                // Malformed: FORMERR without admission.
                TwinStep::Query(header_only_query(7)),
                // Truncated to 512 bytes.
                TwinStep::Query(named_query(8, "big.test", 1)),
                TwinStep::Query(named_query(9, "big.test", 1)),
                // A full socket buffer: the send moves to a task.
                TwinStep::InjectSend(std::io::ErrorKind::WouldBlock),
                TwinStep::Query(named_query(10, "cached.test", 1)),
                // Shutdown while a handed-off query waits for its upstream.
                TwinStep::Unanswered(named_query(11, "silent.test", 1)),
                TwinStep::Shutdown,
            ]
        });
        let replies = &output.replies;
        assert_eq!(replies.len(), 10);
        assert!(replies.iter().all(Option::is_some), "{replies:?}");
        assert_eq!(replies[3].as_ref().expect("blocked")[3] & 0x0f, 3);
        let formerr = replies[6].as_ref().expect("formerr");
        assert_eq!(&formerr[..2], &[0, 7]);
        assert_eq!(formerr[3] & 0x0f, 1);
        for truncated in &replies[7..9] {
            let truncated = truncated.as_ref().expect("truncated");
            assert!(truncated.len() <= 512 && truncated[2] & 0x02 != 0);
        }
        // Miss, refresh, rewrite lookup, big miss, silent: five upstream queries.
        assert_eq!(output.upstream_queries, 5, "{output:?}");
        assert_eq!(output.audit.len(), 10, "{:#?}", output.audit);
        assert!(output.audit[9].contains("terminal_outcome: Canceled"));
        assert!(output.audit[5].contains("domain_set: Some(\"重定向\")"));
        assert_eq!(output.serve_result, "Ok(())");
    }

    #[test]
    fn a_failed_inline_send_is_recorded_as_send_failed() {
        let twins = Twins::new("send-failed");
        let output = run_twin(
            DispatchMode::Inline,
            &twins.upstream,
            &twins.client,
            &twins.rules,
            vec![
                TwinStep::InjectSend(std::io::ErrorKind::ConnectionRefused),
                TwinStep::Query(named_query(1, "blocked.test", 1)),
                TwinStep::InjectSend(std::io::ErrorKind::ConnectionRefused),
                TwinStep::Query(header_only_query(2)),
            ],
        );
        assert_eq!(output.replies, vec![None, None]);
        assert_eq!(output.audit.len(), 1);
        assert!(output.audit[0].contains("terminal_outcome: SendFailed"));
        assert!(output.metrics.contains("send_failed_total: 1"));
        assert!(output.metrics.contains("malformed_total: 1"));
    }

    #[test]
    fn an_inline_query_after_cancellation_is_canceled_and_not_sent() {
        let twins = Twins::new("canceled");
        let config = compile_yaml(&twin_yaml(
            twins.upstream.local_addr().expect("upstream address"),
            &twins.rules,
        ))
        .expect("twin config");
        let assembly = HostAssembly::with_options(config, HostOptions::default()).expect("host");
        assembly.block_on(async {
            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let receiver = UdpSocket::bind("127.0.0.1:0").await.expect("receiver");
            let peer = receiver.local_addr().expect("receiver address");
            let canceled = TransportCancellation::new();
            canceled.cancel();
            let mut tasks = JoinSet::new();
            server.handle_inline(
                &mut tasks,
                &named_query(1, "blocked.test", 1),
                peer,
                &canceled,
            );
            assert!(tasks.is_empty(), "a local answer needs no task");
            let audit = normalized_audit(&assembly);
            assert_eq!(audit.len(), 1);
            assert!(audit[0].contains("terminal_outcome: Canceled"));
            assert!(
                tokio::time::timeout(Duration::from_millis(20), receiver.recv_from(&mut [0; 64]))
                    .await
                    .is_err()
            );
        });
    }

    #[test]
    fn a_panic_while_handling_a_datagram_ends_the_listener_and_closes_upstreams() {
        let twins = Twins::new("panic");
        let config = compile_yaml(&twin_yaml(
            twins.upstream.local_addr().expect("upstream address"),
            &twins.rules,
        ))
        .expect("twin config");
        let assembly = HostAssembly::with_options(config, HostOptions::default()).expect("host");
        assembly.block_on(async {
            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let serving = tokio::task::spawn_local(server.serve(TransportCancellation::new()));
            PANIC_NEXT.with(|flag| flag.set(true));
            let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");
            client
                .send_to(&named_query(1, "blocked.test", 1), server_addr)
                .await
                .expect("send query");
            let result = tokio::time::timeout(Duration::from_secs(2), serving)
                .await
                .expect("listener ends")
                .expect("server task");
            match result {
                Err(super::UdpServerError::Task(message)) => {
                    assert!(message.contains("injected request panic"), "{message}");
                }
                other => panic!("unexpected listener result {other:?}"),
            }
            assert_eq!(
                assembly
                    .forward()
                    .expect("forward")
                    .upstream()
                    .lifecycle_state(),
                LifecycleState::Closed
            );
        });
    }

    #[test]
    fn the_receive_loop_yields_after_each_burst_of_inline_datagrams() {
        let twins = Twins::new("yield");
        for (mode, expected) in [(DispatchMode::Inline, 2), (DispatchMode::Legacy, 0)] {
            YIELDS.with(|count| count.set(0));
            let steps = (0..64)
                .map(|id| TwinStep::Query(header_only_query(id)))
                .collect();
            let output = run_twin(mode, &twins.upstream, &twins.client, &twins.rules, steps);
            assert!(output.metrics.contains("malformed_total: 64"));
            assert_eq!(YIELDS.with(Cell::get), expected, "{mode:?}");
        }
    }

    #[test]
    fn a_handed_off_query_holds_its_admitted_snapshot_until_it_ends() {
        let twins = Twins::new("drain");
        let config = compile_yaml(&twin_yaml(
            twins.upstream.local_addr().expect("upstream address"),
            &twins.rules,
        ))
        .expect("twin config");
        let assembly = HostAssembly::with_options(config, HostOptions::default()).expect("host");
        let upstream = twins.upstream.try_clone().expect("upstream clone");
        assembly.block_on(async {
            let received = Rc::new(Cell::new(0));
            let upstream = UdpSocket::from_std(upstream).expect("tokio upstream");
            let upstream_task =
                tokio::task::spawn_local(fake_upstream(upstream, Rc::clone(&received)));
            let control = assembly.control();
            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let shutdown = TransportCancellation::new();
            let serving = tokio::task::spawn_local(server.serve(shutdown.clone()));
            let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");
            assert_eq!(control.admitted_requests(), 0);
            client
                .send_to(&named_query(1, "silent.test", 1), server_addr)
                .await
                .expect("send query");
            while received.get() == 0 {
                tokio::task::yield_now().await;
            }
            // The query waits for its upstream in a task and still holds the
            // snapshot, so a retiring generation would wait for it.
            assert_eq!(control.admitted_requests(), 1);
            // An inline answer takes and releases the snapshot within the
            // receive loop.
            client
                .send_to(&named_query(2, "blocked.test", 1), server_addr)
                .await
                .expect("send query");
            let mut reply = [0_u8; 512];
            client.recv_from(&mut reply).await.expect("blocked reply");
            assert_eq!(control.admitted_requests(), 1);
            shutdown.cancel();
            assert!(serving.await.expect("server task").is_ok());
            assert_eq!(control.admitted_requests(), 0);
            let audit = normalized_audit(&assembly);
            assert_eq!(audit.len(), 2);
            assert!(audit[1].contains("terminal_outcome: Canceled"), "{audit:?}");
            upstream_task.abort();
        });
    }

    #[cfg(target_os = "linux")]
    fn edns_query(id: u16, name: &str, qtype: u16, udp_size: u16) -> Vec<u8> {
        let mut wire = named_query(id, name, qtype);
        wire[10..12].copy_from_slice(&1_u16.to_be_bytes());
        // Root owner (0), OPT (41), class = udp_size, TTL = 0, RDLEN = 0.
        wire.push(0);
        wire.extend_from_slice(&41_u16.to_be_bytes());
        wire.extend_from_slice(&udp_size.to_be_bytes());
        wire.extend_from_slice(&0_u32.to_be_bytes());
        wire.extend_from_slice(&0_u16.to_be_bytes());
        wire
    }

    #[cfg(target_os = "linux")]
    fn reset_batch_test_hooks() {
        super::INJECTED_BATCH_SENDS.with(|sends| sends.borrow_mut().clear());
        super::BATCH_SEND_CALLS.with(|count| count.set(0));
        super::PANIC_COUNTDOWN.with(|countdown| countdown.set(None));
        PANIC_NEXT.with(|flag| flag.set(false));
        INJECTED_SENDS.with(|sends| sends.borrow_mut().clear());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn batched_and_single_inline_paths_match_audit_and_trace_order_on_mixed_burst() {
        use std::collections::BTreeMap;

        let twins = Twins::new("batch-mixed-parity");
        let run_mode = |mode: DispatchMode| {
            reset_batch_test_hooks();
            let upstream_addr = twins.upstream.local_addr().expect("upstream address");
            let clock = CacheTestClock::new(1000);
            let assembly = HostAssembly::with_options(
                compile_yaml(&twin_yaml(upstream_addr, &twins.rules)).expect("twin config"),
                HostOptions::default().with_cache_clock(Rc::new(clock)),
            )
            .expect("twin host");
            let upstream = twins.upstream.try_clone().expect("upstream clone");
            let client = twins.client.try_clone().expect("client clone");
            assembly.block_on(async {
                let received = Rc::new(Cell::new(0));
                let upstream = UdpSocket::from_std(upstream).expect("tokio upstream");
                let upstream_task =
                    tokio::task::spawn_local(fake_upstream(upstream, Rc::clone(&received)));
                let server = super::UdpServer::bind(
                    &assembly,
                    "127.0.0.1:0".parse().expect("listen address"),
                )
                .await
                .expect("UDP listener")
                .with_mode(mode);
                let server_addr = server.local_addr().expect("bound listener");
                let client = UdpSocket::from_std(client).expect("tokio client");

                // Warm the cache for `cached.test` and `big.test` before the burst.
                let shutdown = TransportCancellation::new();
                let server_task = tokio::task::spawn_local(server.serve(shutdown.clone()));
                for (id, name) in [(1_u16, "cached.test"), (2, "big.test")] {
                    client
                        .send_to(&named_query(id, name, 1), server_addr)
                        .await
                        .expect("warm query");
                    let mut buf = [0_u8; 4096];
                    tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
                        .await
                        .expect("warm reply")
                        .expect("recv");
                }
                while assembly.metrics_snapshot().completed_total != 2 {
                    tokio::task::yield_now().await;
                }

                // Queue a full mixed burst back-to-back before yielding so Linux
                // `recvmmsg` pulls them together.
                let burst = vec![
                    named_query(10, "cached.test", 1),
                    named_query(11, "blocked.test", 1),
                    named_query(12, "ip.test", 1),
                    named_query(13, "big.test", 1),
                    edns_query(14, "big.test", 1, 4096),
                    header_only_query(15),
                    named_query(16, "alias.test", 1),
                    named_query(17, "fresh.test", 1),
                ];
                for query in &burst {
                    client
                        .send_to(query, server_addr)
                        .await
                        .expect("burst send");
                }

                let mut replies = BTreeMap::new();
                let mut buf = [0_u8; 4096];
                while replies.len() < burst.len() {
                    let (len, _) =
                        tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
                            .await
                            .expect("burst reply")
                            .expect("recv");
                    let id = u16::from_be_bytes([buf[0], buf[1]]);
                    replies.insert(id, buf[..len].to_vec());
                }
                while assembly.metrics_snapshot().completed_total
                    + assembly.metrics_snapshot().malformed_total
                    != 2 + burst.len() as u64
                {
                    tokio::task::yield_now().await;
                }
                shutdown.cancel();
                assert!(server_task.await.expect("server task").is_ok());
                upstream_task.abort();

                // Verify trace IDs in the audit snapshot are strictly ordered by
                // admission sequence, and extract normalized rows in trace order.
                let mut records = assembly.audit_snapshot().records;
                records.sort_by(|a, b| a.trace_id.cmp(&b.trace_id));
                for pair in records.windows(2) {
                    assert!(
                        pair[0].trace_id < pair[1].trace_id,
                        "trace IDs must strictly increase"
                    );
                }
                let ordered_audit: Vec<String> = records
                    .into_iter()
                    .map(|mut record| {
                        record.timestamp = std::time::UNIX_EPOCH;
                        record.elapsed = Duration::ZERO;
                        record.trace_id = TraceId::Text(String::new());
                        format!("{record:?}")
                    })
                    .collect();
                let batch_calls = super::BATCH_SEND_CALLS.with(Cell::get);
                (
                    replies,
                    ordered_audit,
                    normalized_metrics(&assembly),
                    received.get(),
                    batch_calls,
                )
            })
        };

        let (batched_replies, batched_audit, batched_metrics, batched_up, batched_calls) =
            run_mode(DispatchMode::Inline);
        let (single_replies, single_audit, single_metrics, single_up, single_calls) =
            run_mode(DispatchMode::SingleInline);

        assert!(batched_calls >= 1, "burst must exercise batched sendmmsg");
        assert_eq!(single_calls, 0);
        assert_eq!(batched_replies, single_replies);
        assert_eq!(batched_audit, single_audit);
        assert_eq!(batched_metrics, single_metrics);
        assert_eq!(batched_up, single_up);
        // Verify 512-byte truncation vs 4096-byte EDNS big reply in the same batch.
        assert!(batched_replies[&13].len() <= 512 && batched_replies[&13][2] & 0x02 != 0);
        assert!(batched_replies[&14].len() > 512 && batched_replies[&14][2] & 0x02 == 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn batched_partial_send_failure_isolates_only_the_failed_query() {
        reset_batch_test_hooks();
        let twins = Twins::new("batch-partial-fail");
        let config = compile_yaml(&twin_yaml(
            twins.upstream.local_addr().expect("upstream address"),
            &twins.rules,
        ))
        .expect("twin config");
        let assembly = HostAssembly::with_options(config, HostOptions::default()).expect("host");
        assembly.block_on(async {
            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");

            // Queue 5 local-answer queries before starting the server so all 5
            // land in one `recvmmsg` / `sendmmsg` batch.
            for id in 1..=5_u16 {
                client
                    .send_to(&named_query(id, "blocked.test", 1), server_addr)
                    .await
                    .expect("send");
            }
            super::INJECTED_BATCH_SENDS.with(|sends| {
                let mut q = sends.borrow_mut();
                q.push_back(super::InjectedBatchSend::RealThenStop(2));
                q.push_back(super::InjectedBatchSend::Error(
                    std::io::ErrorKind::ConnectionRefused,
                ));
            });

            let shutdown = TransportCancellation::new();
            let serving = tokio::task::spawn_local(server.serve(shutdown.clone()));

            let mut received_ids = Vec::new();
            let mut buf = [0_u8; 512];
            for _ in 0..4 {
                let (len, _) =
                    tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
                        .await
                        .expect("reply")
                        .expect("recv");
                assert!(len >= 12);
                received_ids.push(u16::from_be_bytes([buf[0], buf[1]]));
            }
            assert_eq!(received_ids, vec![1, 2, 4, 5]);
            assert!(
                tokio::time::timeout(Duration::from_millis(30), client.recv_from(&mut buf))
                    .await
                    .is_err(),
                "failed query 3 must not be sent"
            );

            shutdown.cancel();
            assert!(serving.await.expect("server task").is_ok());
            assert_eq!(assembly.control().admitted_requests(), 0);
            let metrics = assembly.metrics_snapshot();
            assert_eq!(metrics.send_succeeded_total, 4);
            assert_eq!(metrics.send_failed_total, 1);
            let mut records = assembly.audit_snapshot().records;
            records.sort_by(|a, b| a.trace_id.cmp(&b.trace_id));
            let outcomes: Vec<_> = records.iter().map(|r| r.terminal_outcome).collect();
            assert_eq!(
                outcomes,
                vec![
                    QueryTerminalOutcome::SendSucceeded,
                    QueryTerminalOutcome::SendSucceeded,
                    QueryTerminalOutcome::SendFailed,
                    QueryTerminalOutcome::SendSucceeded,
                    QueryTerminalOutcome::SendSucceeded,
                ]
            );
        });
        reset_batch_test_hooks();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn batched_dual_stack_first_send_failure_still_delivers_ipv4_mapped_and_ipv6_peers() {
        reset_batch_test_hooks();
        let twins = Twins::new("batch-dual-stack-first-fail");
        let config = compile_yaml(&twin_yaml(
            twins.upstream.local_addr().expect("upstream address"),
            &twins.rules,
        ))
        .expect("twin config");
        let assembly = HostAssembly::with_options(config, HostOptions::default()).expect("host");
        assembly.block_on(async {
            let Ok(server) =
                super::UdpServer::bind(&assembly, "[::]:0".parse().expect("listen address")).await
            else {
                return;
            };
            let port = server.local_addr().expect("bound listener").port();
            let Ok(v6_first) = UdpSocket::bind("[::1]:0").await else {
                return;
            };
            let v4_client = UdpSocket::bind("127.0.0.1:0").await.expect("v4 client");
            let v4_addr = v4_client.local_addr().expect("v4 addr");
            let v6_second = UdpSocket::bind("[::1]:0").await.expect("v6 client");
            let v6_second_addr = v6_second.local_addr().expect("v6 addr");

            // Queue 3 datagrams before starting the server: the first fails in
            // `sendmmsg`, and the subsequent IPv4-mapped and native IPv6 replies
            // must still be sent cleanly using their raw peer addresses.
            v6_first
                .send_to(
                    &named_query(1, "blocked.test", 1),
                    SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port)),
                )
                .await
                .expect("send 1");
            v4_client
                .send_to(
                    &named_query(2, "ip.test", 1),
                    SocketAddr::from(([127, 0, 0, 1], port)),
                )
                .await
                .expect("send 2");
            v6_second
                .send_to(
                    &named_query(3, "ip.test", 1),
                    SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port)),
                )
                .await
                .expect("send 3");

            super::INJECTED_BATCH_SENDS.with(|sends| {
                sends
                    .borrow_mut()
                    .push_back(super::InjectedBatchSend::Error(
                        std::io::ErrorKind::ConnectionRefused,
                    ));
            });

            let shutdown = TransportCancellation::new();
            let serving = tokio::task::spawn_local(server.serve(shutdown.clone()));

            let mut buf = [0_u8; 512];
            let (n4, _) =
                tokio::time::timeout(Duration::from_secs(2), v4_client.recv_from(&mut buf))
                    .await
                    .expect("v4 reply")
                    .expect("recv v4");
            assert_eq!(u16::from_be_bytes([buf[0], buf[1]]), 2);
            assert!(n4 >= 12);

            let (n6, _) =
                tokio::time::timeout(Duration::from_secs(2), v6_second.recv_from(&mut buf))
                    .await
                    .expect("v6 reply")
                    .expect("recv v6");
            assert_eq!(u16::from_be_bytes([buf[0], buf[1]]), 3);
            assert!(n6 >= 12);

            assert!(
                tokio::time::timeout(Duration::from_millis(30), v6_first.recv_from(&mut buf))
                    .await
                    .is_err(),
                "first failed datagram must not be delivered"
            );

            shutdown.cancel();
            assert!(serving.await.expect("server task").is_ok());
            let mut records = assembly.audit_snapshot().records;
            records.sort_by(|a, b| a.trace_id.cmp(&b.trace_id));
            assert_eq!(records.len(), 3);
            assert_eq!(
                records[0].terminal_outcome,
                QueryTerminalOutcome::SendFailed
            );
            assert_eq!(
                records[1].terminal_outcome,
                QueryTerminalOutcome::SendSucceeded
            );
            // Audit must record the unmapped plain IPv4 address while sending
            // used the raw IPv4-mapped IPv6 peer so `v4_client` received it.
            assert_eq!(records[1].client_addr, v4_addr);
            assert_eq!(
                records[2].terminal_outcome,
                QueryTerminalOutcome::SendSucceeded
            );
            assert_eq!(records[2].client_addr, v6_second_addr);
        });
        reset_batch_test_hooks();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn batched_partial_send_then_would_block_hands_remainder_to_async_send() {
        reset_batch_test_hooks();
        let twins = Twins::new("batch-wouldblock");
        let config = compile_yaml(&twin_yaml(
            twins.upstream.local_addr().expect("upstream address"),
            &twins.rules,
        ))
        .expect("twin config");
        let assembly = HostAssembly::with_options(config, HostOptions::default()).expect("host");
        assembly.block_on(async {
            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");

            for id in 1..=5_u16 {
                client
                    .send_to(&named_query(id, "blocked.test", 1), server_addr)
                    .await
                    .expect("send");
            }
            super::INJECTED_BATCH_SENDS.with(|sends| {
                let mut q = sends.borrow_mut();
                q.push_back(super::InjectedBatchSend::RealThenStop(2));
                q.push_back(super::InjectedBatchSend::Error(
                    std::io::ErrorKind::WouldBlock,
                ));
            });

            let shutdown = TransportCancellation::new();
            let serving = tokio::task::spawn_local(server.serve(shutdown.clone()));

            let mut received_ids = Vec::new();
            let mut buf = [0_u8; 512];
            for _ in 0..5 {
                let (len, _) =
                    tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
                        .await
                        .expect("reply")
                        .expect("recv");
                assert!(len >= 12);
                received_ids.push(u16::from_be_bytes([buf[0], buf[1]]));
            }
            received_ids.sort_unstable();
            assert_eq!(received_ids, vec![1, 2, 3, 4, 5]);
            while assembly.metrics_snapshot().completed_total != 5 {
                tokio::task::yield_now().await;
            }
            shutdown.cancel();
            assert!(serving.await.expect("server task").is_ok());
            assert_eq!(assembly.control().admitted_requests(), 0);
            assert_eq!(assembly.metrics_snapshot().send_succeeded_total, 5);
        });
        reset_batch_test_hooks();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn batched_zero_send_result_falls_back_to_async_send_without_spinning() {
        reset_batch_test_hooks();
        let twins = Twins::new("batch-zero");
        let config = compile_yaml(&twin_yaml(
            twins.upstream.local_addr().expect("upstream address"),
            &twins.rules,
        ))
        .expect("twin config");
        let assembly = HostAssembly::with_options(config, HostOptions::default()).expect("host");
        assembly.block_on(async {
            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");

            for id in 1..=3_u16 {
                client
                    .send_to(&named_query(id, "blocked.test", 1), server_addr)
                    .await
                    .expect("send");
            }
            super::INJECTED_BATCH_SENDS.with(|sends| {
                sends.borrow_mut().push_back(super::InjectedBatchSend::Zero);
            });

            let shutdown = TransportCancellation::new();
            let serving = tokio::task::spawn_local(server.serve(shutdown.clone()));

            let mut received_ids = Vec::new();
            let mut buf = [0_u8; 512];
            for _ in 0..3 {
                let (_, _) =
                    tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
                        .await
                        .expect("reply")
                        .expect("recv");
                received_ids.push(u16::from_be_bytes([buf[0], buf[1]]));
            }
            received_ids.sort_unstable();
            assert_eq!(received_ids, vec![1, 2, 3]);
            while assembly.metrics_snapshot().completed_total != 3 {
                tokio::task::yield_now().await;
            }
            shutdown.cancel();
            assert!(serving.await.expect("server task").is_ok());
            assert_eq!(assembly.control().admitted_requests(), 0);
            assert_eq!(assembly.metrics_snapshot().send_succeeded_total, 3);
        });
        reset_batch_test_hooks();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn batched_mid_flush_cancellation_records_canceled_for_unsent_queries() {
        reset_batch_test_hooks();
        let twins = Twins::new("batch-mid-cancel");
        let config = compile_yaml(&twin_yaml(
            twins.upstream.local_addr().expect("upstream address"),
            &twins.rules,
        ))
        .expect("twin config");
        let assembly = HostAssembly::with_options(config, HostOptions::default()).expect("host");
        assembly.block_on(async {
            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");

            for id in 1..=5_u16 {
                client
                    .send_to(&named_query(id, "blocked.test", 1), server_addr)
                    .await
                    .expect("send");
            }
            super::INJECTED_BATCH_SENDS.with(|sends| {
                sends
                    .borrow_mut()
                    .push_back(super::InjectedBatchSend::RealThenCancel(2));
            });

            let shutdown = TransportCancellation::new();
            let serving = tokio::task::spawn_local(server.serve(shutdown.clone()));

            let mut received_ids = Vec::new();
            let mut buf = [0_u8; 512];
            for _ in 0..2 {
                let (_, _) =
                    tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
                        .await
                        .expect("reply")
                        .expect("recv");
                received_ids.push(u16::from_be_bytes([buf[0], buf[1]]));
            }
            assert_eq!(received_ids, vec![1, 2]);
            assert!(
                tokio::time::timeout(Duration::from_millis(30), client.recv_from(&mut buf))
                    .await
                    .is_err(),
                "canceled replies must not be sent"
            );

            assert!(serving.await.expect("server task").is_ok());
            assert_eq!(assembly.control().admitted_requests(), 0);
            let mut records = assembly.audit_snapshot().records;
            records.sort_by(|a, b| a.trace_id.cmp(&b.trace_id));
            let outcomes: Vec<_> = records.iter().map(|r| r.terminal_outcome).collect();
            assert_eq!(
                outcomes,
                vec![
                    QueryTerminalOutcome::SendSucceeded,
                    QueryTerminalOutcome::SendSucceeded,
                    QueryTerminalOutcome::Canceled,
                    QueryTerminalOutcome::Canceled,
                    QueryTerminalOutcome::Canceled,
                ]
            );
        });
        reset_batch_test_hooks();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn batched_mid_batch_panic_flushes_earlier_replies_and_closes_upstreams() {
        reset_batch_test_hooks();
        let twins = Twins::new("batch-mid-panic");
        let config = compile_yaml(&twin_yaml(
            twins.upstream.local_addr().expect("upstream address"),
            &twins.rules,
        ))
        .expect("twin config");
        let assembly = HostAssembly::with_options(config, HostOptions::default()).expect("host");
        assembly.block_on(async {
            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");

            for id in 1..=4_u16 {
                client
                    .send_to(&named_query(id, "blocked.test", 1), server_addr)
                    .await
                    .expect("send");
            }
            // Let the first 2 datagrams finish, then panic on the 3rd.
            super::PANIC_COUNTDOWN.with(|c| c.set(Some(2)));

            let serving = tokio::task::spawn_local(server.serve(TransportCancellation::new()));
            let result = tokio::time::timeout(Duration::from_secs(2), serving)
                .await
                .expect("listener ends")
                .expect("server task");
            match result {
                Err(super::UdpServerError::Task(message)) => {
                    assert!(message.contains("injected request panic"), "{message}");
                }
                other => panic!("unexpected listener result {other:?}"),
            }

            let mut received_ids = Vec::new();
            let mut buf = [0_u8; 512];
            for _ in 0..2 {
                let (_, _) =
                    tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
                        .await
                        .expect("pre-panic reply")
                        .expect("recv");
                received_ids.push(u16::from_be_bytes([buf[0], buf[1]]));
            }
            assert_eq!(received_ids, vec![1, 2]);
            assert_eq!(assembly.control().admitted_requests(), 0);
            assert_eq!(assembly.metrics_snapshot().send_succeeded_total, 2);
            assert_eq!(
                assembly
                    .forward()
                    .expect("forward")
                    .upstream()
                    .lifecycle_state(),
                LifecycleState::Closed
            );
        });
        reset_batch_test_hooks();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn low_load_single_datagram_uses_single_send_fast_path() {
        reset_batch_test_hooks();
        let twins = Twins::new("batch-low-load");
        let config = compile_yaml(&twin_yaml(
            twins.upstream.local_addr().expect("upstream address"),
            &twins.rules,
        ))
        .expect("twin config");
        let assembly = HostAssembly::with_options(config, HostOptions::default()).expect("host");
        assembly.block_on(async {
            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let shutdown = TransportCancellation::new();
            let serving = tokio::task::spawn_local(server.serve(shutdown.clone()));
            let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");

            let mut buf = [0_u8; 512];
            for id in 1..=3_u16 {
                client
                    .send_to(&named_query(id, "blocked.test", 1), server_addr)
                    .await
                    .expect("send");
                let (len, _) =
                    tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
                        .await
                        .expect("reply")
                        .expect("recv");
                assert!(len >= 12);
                assert_eq!(u16::from_be_bytes([buf[0], buf[1]]), id);
            }
            assert_eq!(
                super::BATCH_SEND_CALLS.with(Cell::get),
                0,
                "single-datagram wakeups must bypass sendmmsg"
            );
            shutdown.cancel();
            assert!(serving.await.expect("server task").is_ok());
        });
        reset_batch_test_hooks();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn batched_burst_of_64_datagrams_yields_exactly_twice() {
        reset_batch_test_hooks();
        let twins = Twins::new("batch-yield-burst");
        let config = compile_yaml(&twin_yaml(
            twins.upstream.local_addr().expect("upstream address"),
            &twins.rules,
        ))
        .expect("twin config");
        let assembly = HostAssembly::with_options(config, HostOptions::default()).expect("host");
        assembly.block_on(async {
            let server =
                super::UdpServer::bind(&assembly, "127.0.0.1:0".parse().expect("listen address"))
                    .await
                    .expect("UDP listener");
            let server_addr = server.local_addr().expect("bound listener");
            let client = UdpSocket::bind("127.0.0.1:0").await.expect("client");

            for id in 0..64_u16 {
                client
                    .send_to(&header_only_query(id), server_addr)
                    .await
                    .expect("send");
            }
            YIELDS.with(|count| count.set(0));
            let shutdown = TransportCancellation::new();
            let serving = tokio::task::spawn_local(server.serve(shutdown.clone()));
            let mut buf = [0_u8; 64];
            for _ in 0..64 {
                tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
                    .await
                    .expect("formerr reply")
                    .expect("recv");
            }
            while assembly.metrics_snapshot().malformed_total != 64 {
                tokio::task::yield_now().await;
            }
            assert_eq!(YIELDS.with(Cell::get), 2);
            assert!(super::BATCH_SEND_CALLS.with(Cell::get) >= 2);
            shutdown.cancel();
            assert!(serving.await.expect("server task").is_ok());
        });
        reset_batch_test_hooks();
    }
}
