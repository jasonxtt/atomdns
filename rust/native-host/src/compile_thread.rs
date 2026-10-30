//! Compiling a hot-apply candidate off the DNS thread (M2-3).
//!
//! A candidate compile reads the configuration, loads every rule source and
//! builds the matchers: about half a second with the production rule set,
//! during which the single DNS thread used to answer nothing. Its result,
//! `CompiledConfig`, is a single-threaded graph (`Rc`, `RefCell`, `Cell`), so it
//! is not `Send` and cannot simply be returned from another thread. This module
//! moves it anyway, for one reason that holds by construction:
//!
//! A value built on a thread that has terminated, and that is handed over only
//! after that thread was joined, has no owner left on that thread. Every `Rc`
//! (and every `Weak`, `RefCell`, `Cell`) inside it can only be shared with:
//!
//! 1. the inputs of the closure: they are `Send + 'static` (checked by the
//!    compiler), so no `Rc` from any other thread can enter the value;
//! 2. statics: a `static` must be `Sync`, so it cannot hold an `Rc`;
//! 3. the compile thread's own thread-locals: they are destroyed before the
//!    thread terminates, and `join` returns only after it terminated.
//!
//! So once `join` has returned, all owners of everything inside the value are
//! inside the value itself, and moving it to the DNS thread moves all of them
//! together. That covers values that are not `Send` only because of shared
//! single-thread ownership (`Rc`, `Weak`, and `RefCell`/`Cell` around them). It
//! would not cover thread-affine state (a lock guard, a handle that must be
//! used or dropped on the thread that created it), which is why only the
//! compile result can be moved: `CompiledConfig` and `ConfigError` hold plain
//! data, `Rc`/`Weak`/`RefCell`/`Cell`, `Arc`s and tokio sync primitives and
//! `JoinSet`s, none of them tied to the creating thread once unshared.
//!
//! `join`, and the blocking task's completion that carries the value on, order
//! every access on the compile thread before every access after the handover;
//! the value is never touched by two threads at the same time. This does not
//! depend on what the compiler does internally (whether it uses thread-locals,
//! say), which is why every compile gets a fresh thread instead of the
//! long-lived rule build thread.
//!
//! The same holds when nobody takes the value: whichever thread drops it
//! (the blocking thread, or the DNS thread dropping the task handle) is then
//! its only owner.
#![allow(unsafe_code)]

use crate::config::{CompiledConfig, ConfigError};

/// A value produced on a thread that has since been joined (see the module
/// documentation). Only [`on_joined_thread`] creates one.
struct Joined<T>(T);

// SAFETY: a `Joined` is created on a fresh thread as that thread's result and
// is moved out only through `JoinHandle::join`, after the thread has
// terminated. As argued in the module documentation, no owner of anything
// inside it remains outside it at that point, so moving it to another thread
// cannot let two threads touch one `Rc` count or `RefCell` flag. Only values
// that are `!Send` through shared ownership alone may be wrapped; the one
// crate-visible entry, `compile_off_thread`, admits only the compile result.
unsafe impl<T> Send for Joined<T> {}

/// Compiles a hot-apply candidate on a new thread (see the module
/// documentation), waiting for it without blocking the caller's runtime
/// thread. A panic in `work` is returned as an error.
pub(crate) async fn compile_off_thread(
    work: impl FnOnce() -> Result<CompiledConfig, ConfigError> + Send + 'static,
) -> Result<Result<CompiledConfig, ConfigError>, String> {
    on_joined_thread(work).await
}

/// Runs `work` on a new thread and hands its result over after joining it.
/// `T` may be `!Send` only through shared single-thread ownership, never
/// through thread affinity (module documentation).
async fn on_joined_thread<T: 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let joined = tokio::task::spawn_blocking(move || {
        std::thread::Builder::new()
            .name("atomdns-apply-compile".into())
            .spawn(move || Joined(work()))
            .map_err(|error| format!("cannot start the compile thread: {error}"))?
            .join()
            .map_err(|_| "the configuration compile panicked".to_owned())
    })
    .await
    .map_err(|error| format!("the configuration compile failed: {error}"))??;
    Ok(joined.0)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn a_single_threaded_graph_comes_back_whole() {
        let graph = block_on(super::on_joined_thread(|| {
            let shared = Rc::new(RefCell::new(vec![1, 2]));
            (Rc::clone(&shared), shared)
        }))
        .unwrap();
        graph.0.borrow_mut().push(3);
        assert_eq!(*graph.1.borrow(), vec![1, 2, 3]);
        assert_eq!(Rc::strong_count(&graph.0), 2);
    }

    #[test]
    fn work_runs_on_its_own_thread() {
        let name = block_on(super::on_joined_thread(|| {
            std::thread::current().name().map(str::to_owned)
        }))
        .unwrap();
        assert_eq!(name.as_deref(), Some("atomdns-apply-compile"));
    }

    #[test]
    fn a_panic_is_an_error() {
        let result = block_on(super::on_joined_thread(|| -> u8 {
            panic!("compile bug");
        }));
        assert_eq!(result.unwrap_err(), "the configuration compile panicked");
    }

    #[test]
    fn an_abandoned_result_is_dropped_exactly_once() {
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&drops);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let compile = super::on_joined_thread(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                Rc::new(Counted(counter))
            });
            // The caller gives up (as a hot apply does on shutdown) before the result exists.
            let abandoned =
                tokio::time::timeout(std::time::Duration::from_millis(1), compile).await;
            assert!(abandoned.is_err());
        });
        // Dropping the runtime waits for the blocking task, which drops the result.
        drop(runtime);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
