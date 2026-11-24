//! Compact, lossless storage for terminal audit records.

use std::cmp::Ordering;
use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};
use std::net::{SocketAddr, SocketAddrV6};
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{AuditRecord, ResponseDetails, TraceId};

const SHARING_SLOTS: usize = 4096;
const SHARING_SLOT_BITS: u32 = SHARING_SLOTS.trailing_zeros();
const _: () = assert!(SHARING_SLOTS.is_power_of_two());

/// A multiply-rotate hash (the FxHash scheme) for picking a sharing slot. The
/// slot is only a hint: a collision costs one missed share, never a wrong
/// record, so a keyed hash buys nothing here and SipHash cost about 2% of the
/// cache-hit path (docs/rust/validation-records/11-24-profile-query-cpu.md).
#[derive(Default)]
struct SlotHasher(u64);

impl SlotHasher {
    const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(Self::SEED);
    }
}

impl Hasher for SlotHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().expect("8-byte chunk")));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut word = [0_u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// A fixed-size weak cache. Collisions replace only the cache hint; existing
/// records/snapshots keep their own strong references. It never owns strings
/// after their final record has been released.
struct SharingCache<T> {
    slots: Vec<Option<Weak<T>>>,
    hash: BuildHasherDefault<SlotHasher>,
}

impl<T> Default for SharingCache<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            hash: BuildHasherDefault::default(),
        }
    }
}

impl<T: Hash + Eq> SharingCache<T> {
    fn share(&mut self, mut value: T, compact: impl FnOnce(&mut T)) -> Arc<T> {
        if self.slots.is_empty() {
            if self.slots.try_reserve_exact(SHARING_SLOTS).is_err() {
                // Sharing is optional: a recoverable hint-table allocation
                // failure must not become a DNS failure or lose this record.
                compact(&mut value);
                return Arc::new(value);
            }
            self.slots.resize_with(SHARING_SLOTS, || None);
        }
        // The top bits: the final multiply mixes them best.
        let slot = usize::try_from(self.hash.hash_one(&value) >> (u64::BITS - SHARING_SLOT_BITS))
            .expect("sharing slot fits usize");
        if let Some(existing) = self.slots[slot].as_ref().and_then(Weak::upgrade) {
            if *existing == value {
                return existing;
            }
        }
        compact(&mut value);
        let shared = Arc::new(value);
        self.slots[slot] = Some(Arc::downgrade(&shared));
        shared
    }
}

#[derive(Default)]
pub(super) struct AuditRetention {
    templates: SharingCache<AuditRecord>,
    responses: SharingCache<ResponseDetails>,
    /// The process nonce of native trace ids, shared by every record.
    native_prefix: Option<Arc<TracePrefix>>,
}

/// The original record is the shared template, with only its per-row fields
/// replaced by empty placeholders. Filters/ranks explicitly read common facts
/// without allocating a full record for every inspected row.
pub(crate) struct RetainedAuditRecord {
    /// Wall time as signed nanoseconds from the Unix epoch.
    unix_nanos: i64,
    elapsed_nanos: u64,
    pub trace_id: RetainedTraceId,
    client: RetainedClient,
    pub qname: Box<str>,
    pub response_details: Arc<ResponseDetails>,
    template: Arc<AuditRecord>,
}

impl RetainedAuditRecord {
    /// Common facts only. Per-row fields must be read from the retained record;
    /// expansion is required before passing a full AuditRecord to a projector.
    pub(crate) fn common(&self) -> &AuditRecord {
        &self.template
    }

    pub(crate) fn timestamp(&self) -> SystemTime {
        system_time(self.unix_nanos)
    }

    pub(crate) fn elapsed(&self) -> Duration {
        Duration::from_nanos(self.elapsed_nanos)
    }

    pub(crate) fn client_addr(&self) -> SocketAddr {
        self.client.address()
    }

    pub(crate) fn expand(&self) -> AuditRecord {
        AuditRecord {
            timestamp: self.timestamp(),
            client_addr: self.client_addr(),
            qname: self.qname.to_string(),
            trace_id: self.trace_id.to_trace_id(),
            elapsed: self.elapsed(),
            response_details: self.response_details.as_ref().clone(),
            ..self.template.as_ref().clone()
        }
    }
}

/// A client socket address in 24 bytes instead of `SocketAddr`'s 32. IPv6
/// addresses with a flow label or scope id (link-local clients) keep the full
/// value out of line, so every address round-trips exactly.
enum RetainedClient {
    V4 { ip: [u8; 4], port: u16 },
    V6 { ip: [u8; 16], port: u16 },
    Full(Box<SocketAddrV6>),
}

impl From<SocketAddr> for RetainedClient {
    fn from(address: SocketAddr) -> Self {
        match address {
            SocketAddr::V4(v4) => Self::V4 {
                ip: v4.ip().octets(),
                port: v4.port(),
            },
            SocketAddr::V6(v6) if v6.flowinfo() == 0 && v6.scope_id() == 0 => Self::V6 {
                ip: v6.ip().octets(),
                port: v6.port(),
            },
            SocketAddr::V6(v6) => Self::Full(Box::new(v6)),
        }
    }
}

impl RetainedClient {
    fn address(&self) -> SocketAddr {
        match self {
            Self::V4 { ip, port } => SocketAddr::from((*ip, *port)),
            Self::V6 { ip, port } => SocketAddr::from((*ip, *port)),
            Self::Full(address) => SocketAddr::V6(**address),
        }
    }
}

fn system_time(unix_nanos: i64) -> SystemTime {
    let magnitude = Duration::from_nanos(unix_nanos.unsigned_abs());
    if unix_nanos >= 0 {
        UNIX_EPOCH + magnitude
    } else {
        UNIX_EPOCH - magnitude
    }
}

/// Signed nanoseconds from the Unix epoch; exact for 1677–2262, clamped outside.
fn unix_nanos(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_nanos()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_nanos()).map_or(i64::MIN, |n| -n),
    }
}

impl AuditRetention {
    pub(super) fn retain(&mut self, mut record: AuditRecord) -> RetainedAuditRecord {
        let response =
            std::mem::replace(&mut record.response_details, ResponseDetails::no_response());
        let elapsed = std::mem::replace(&mut record.elapsed, Duration::ZERO);
        RetainedAuditRecord {
            unix_nanos: unix_nanos(std::mem::replace(
                &mut record.timestamp,
                SystemTime::UNIX_EPOCH,
            )),
            elapsed_nanos: u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX),
            client: std::mem::replace(&mut record.client_addr, ([0, 0, 0, 0], 0).into()).into(),
            qname: std::mem::take(&mut record.qname).into_boxed_str(),
            trace_id: RetainedTraceId::new(
                std::mem::take(&mut record.trace_id),
                &mut self.native_prefix,
            ),
            response_details: self.responses.share(response, |response| {
                response.answers.shrink_to_fit();
                for answer in &mut response.answers {
                    answer.data.shrink_to_fit();
                }
                if let Some(error) = &mut response.answer_decode_error {
                    error.shrink_to_fit();
                }
            }),
            template: self.templates.share(record, |_| {}),
        }
    }
}

/// A trace id in 16 bytes. Native ids (`n-<nonce>-<counter>`) keep only the
/// counter per record; the nonce is the same for the whole process and is shared.
#[derive(Eq, PartialEq)]
pub(crate) struct RetainedTraceId {
    counter: u64,
    prefix: Arc<TracePrefix>,
}

#[derive(Eq, PartialEq)]
pub(super) enum TracePrefix {
    Native(u128),
    /// Any other spelling, kept verbatim (`counter` is then 0).
    Text(Box<str>),
}

impl RetainedTraceId {
    #[cfg(test)]
    fn from_string(value: String, native_prefix: &mut Option<Arc<TracePrefix>>) -> Self {
        Self::new(TraceId::parse(value), native_prefix)
    }

    fn new(value: TraceId, native_prefix: &mut Option<Arc<TracePrefix>>) -> Self {
        match value {
            TraceId::Native { nonce, counter } => {
                let prefix = match native_prefix {
                    Some(shared) if **shared == TracePrefix::Native(nonce) => Arc::clone(shared),
                    _ => {
                        let shared = Arc::new(TracePrefix::Native(nonce));
                        *native_prefix = Some(Arc::clone(&shared));
                        shared
                    }
                };
                Self { counter, prefix }
            }
            TraceId::Text(value) => Self {
                counter: 0,
                prefix: Arc::new(TracePrefix::Text(value.into_boxed_str())),
            },
        }
    }

    pub(crate) fn to_trace_id(&self) -> TraceId {
        match self.prefix.as_ref() {
            TracePrefix::Native(nonce) => TraceId::Native {
                nonce: *nonce,
                counter: self.counter,
            },
            TracePrefix::Text(value) => TraceId::Text(value.to_string()),
        }
    }
}

impl std::fmt::Display for RetainedTraceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.prefix.as_ref() {
            TracePrefix::Native(nonce) => {
                write!(formatter, "n-{nonce:032x}-{:016x}", self.counter)
            }
            TracePrefix::Text(value) => formatter.write_str(value),
        }
    }
}

impl Ord for RetainedTraceId {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.prefix.as_ref(), other.prefix.as_ref()) {
            (TracePrefix::Native(left), TracePrefix::Native(right)) => {
                (left, self.counter).cmp(&(right, other.counter))
            }
            _ => self.to_string().cmp(&other.to_string()),
        }
    }
}

impl PartialOrd for RetainedTraceId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;
    use std::sync::Arc;

    use std::net::{SocketAddr, SocketAddrV6};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        RetainedAuditRecord, RetainedClient, RetainedTraceId, SHARING_SLOTS, SharingCache,
        system_time, unix_nanos,
    };

    #[test]
    fn compact_record_fields_round_trip_exactly() {
        for address in [
            "192.0.2.7:53000",
            "[2001:db8::7]:53000",
            "[::ffff:192.0.2.7]:1",
            "0.0.0.0:0",
        ] {
            let address: SocketAddr = address.parse().unwrap();
            assert_eq!(RetainedClient::from(address).address(), address);
        }
        let scoped = SocketAddr::V6(SocketAddrV6::new("fe80::7".parse().unwrap(), 53, 9, 3));
        assert!(matches!(
            RetainedClient::from(scoped),
            RetainedClient::Full(_)
        ));
        assert_eq!(RetainedClient::from(scoped).address(), scoped);

        for time in [
            UNIX_EPOCH,
            UNIX_EPOCH + Duration::new(1_795_000_000, 123_456_789),
            UNIX_EPOCH - Duration::new(86_400, 1),
            SystemTime::now(),
        ] {
            assert_eq!(system_time(unix_nanos(time)), time);
        }
        assert!(std::mem::size_of::<RetainedAuditRecord>() <= 88);
    }

    #[test]
    fn native_trace_ids_share_one_prefix() {
        let mut prefix = None;
        let id = |counter: u32| format!("n-{:032x}-{counter:016x}", 0xabc_u32);
        let first = RetainedTraceId::from_string(id(1), &mut prefix);
        let second = RetainedTraceId::from_string(id(2), &mut prefix);
        assert!(Arc::ptr_eq(&first.prefix, &second.prefix));
        assert_eq!(second.to_string(), id(2));
        assert!(first < second);
    }

    #[derive(Eq, PartialEq, Debug)]
    struct Colliding(u64);

    impl Hash for Colliding {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            // Deliberately valid hash collisions exercise full equality.
            0_u8.hash(state);
        }
    }

    #[test]
    fn sharing_cache_collisions_never_change_live_records_or_retain_payloads() {
        let mut cache = SharingCache::default();
        let first = cache.share(Colliding(1), |_| {});
        let weak = Arc::downgrade(&first);
        let second = cache.share(Colliding(2), |_| {});
        assert_eq!(first.0, 1);
        assert_eq!(second.0, 2);
        let again = cache.share(Colliding(2), |_| {
            panic!("shared payload must not be compacted again")
        });
        assert!(Arc::ptr_eq(&second, &again));
        drop(first);
        assert!(weak.upgrade().is_none());
        for value in 3..10000 {
            assert_eq!(cache.share(Colliding(value), |_| {}).0, value);
        }
        assert_eq!(cache.slots.len(), SHARING_SLOTS);
        assert_eq!(second.0, 2);
    }

    #[test]
    fn trace_id_representation_preserves_all_bytes_and_lexical_order() {
        let mut values = vec![
            "n-11111111111111111111111111111111-0000000000000001".to_owned(),
            "n-11111111111111111111111111111111-ffffffffffffffff".to_owned(),
            "n-11111111111111111111111111111111-0000000000000000".to_owned(),
            "n-00000000000000000000000000000000-ffffffffffffffff".to_owned(),
            "n-ABCDEFABCDEFABCDEFABCDEFABCDEFAB-0000000000000001".to_owned(),
            "n-+1111111111111111111111111111111-0000000000000001".to_owned(),
            "legacy trace 零".to_owned(),
            String::new(),
        ];
        let mut prefix = None;
        let mut compact: Vec<_> = values
            .iter()
            .cloned()
            .map(|value| RetainedTraceId::from_string(value, &mut prefix))
            .collect();
        assert_eq!(
            values,
            compact.iter().map(ToString::to_string).collect::<Vec<_>>()
        );
        values.sort();
        compact.sort();
        assert_eq!(
            values,
            compact.iter().map(ToString::to_string).collect::<Vec<_>>()
        );
    }

    #[test]
    fn old_generation_strings_live_only_until_the_last_snapshot_reference() {
        let mut cache = SharingCache::default();
        let old_record = cache.share("generation-one rule source".to_owned(), |_| {});
        let weak = Arc::downgrade(&old_record);
        let read_snapshot = Arc::clone(&old_record);
        for generation in 2..10000 {
            let _ = cache.share(format!("generation-{generation} rule source"), |_| {});
        }
        drop(old_record);
        assert!(weak.upgrade().is_some());
        drop(read_snapshot);
        assert!(weak.upgrade().is_none());
        assert_eq!(cache.slots.len(), SHARING_SLOTS);
    }
}
