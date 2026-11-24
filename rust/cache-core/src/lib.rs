//! `AtomDNS` Rust cache core: the safe [`NativeCache`] used by the native host.
//! (The former C ABI and its handle registry were removed with the Go bridge in Phase 6a.)

#![allow(clippy::missing_errors_doc)]

use quick_cache::sync::Cache as QuickCache;
use std::cell::Cell;
use std::sync::{Arc, Weak};

mod wire;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CacheConfig {
    pub capacity: u64,
    pub lazy_cache_ttl_secs: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LookupState {
    Miss = 0,
    Fresh = 1,
    Lazy = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheError {
    InvalidConfig,
    InvalidKey,
    InvalidResponse,
    InvalidExpiry,
    Internal,
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidConfig => "invalid cache configuration",
            Self::InvalidKey => "invalid cache key",
            Self::InvalidResponse => "invalid cache response",
            Self::InvalidExpiry => "invalid cache expiry",
            Self::Internal => "cache operation failed",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for CacheError {}

struct CacheState {
    config: CacheConfig,
    /// A sharded CLOCK-PRO store (`quick_cache`): about 70-90 B of bookkeeping
    /// per entry, against about 250 B for the former moka store
    /// (docs/rust/validation-records/11-24-quick-cache.md). Expiry is ours:
    /// lookups drop dead entries, the store only bounds the entry count.
    entries: QuickCache<Box<[u8]>, Arc<CacheEntry>>,
}

/// An owned, safe cache instance for Rust-native callers.
///
/// This object owns one bounded store and accepts ordinary Rust slices.
pub struct NativeCache {
    state: CacheState,
}

/// An optional native-only immutable attachment. It shares the exact entry's
/// replacement/eviction/flush lifetime. Native snapshots retain it; codecs choose
/// supported typed metadata.
#[derive(Clone)]
pub struct NativeAttachment(Arc<dyn std::any::Any + Send + Sync>);
impl NativeAttachment {
    pub fn new<T: std::any::Any + Send + Sync>(value: T) -> Self {
        Self(Arc::new(value))
    }
    #[must_use]
    pub fn downcast_ref<T: std::any::Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
    /// A non-owning handle, so a caller can share one attachment between many
    /// entries without keeping it alive after the last entry is gone.
    #[must_use]
    pub fn downgrade(&self) -> WeakAttachment {
        WeakAttachment(Arc::downgrade(&self.0))
    }
}

/// A non-owning [`NativeAttachment`] handle.
#[derive(Clone)]
pub struct WeakAttachment(Weak<dyn std::any::Any + Send + Sync>);
impl WeakAttachment {
    #[must_use]
    pub fn upgrade(&self) -> Option<NativeAttachment> {
        self.0.upgrade().map(NativeAttachment)
    }
}
impl std::fmt::Debug for NativeAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeAttachment")
    }
}
impl PartialEq for NativeAttachment {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for NativeAttachment {}

/// One stored answer. The response and the domain set share one allocation
/// (`data` = response bytes, then domain-set bytes): a cache holds up to
/// hundreds of thousands of these, so every allocation per entry counts
/// (docs/rust/validation-records/11-24-memory-composition.md).
#[derive(Debug)]
struct CacheEntry {
    attachment: Option<NativeAttachment>,
    data: Box<[u8]>,
    response_len: usize,
    stored_at_unix: i64,
    message_expires_at_unix: i64,
    cache_expires_at_unix: i64,
    wall_times: Option<[i64; 3]>,
}

impl CacheEntry {
    fn new(
        attachment: Option<NativeAttachment>,
        response: &[u8],
        domain_set: &[u8],
        times: [i64; 3],
        wall_times: Option<[i64; 3]>,
    ) -> Self {
        let mut data = Vec::with_capacity(response.len() + domain_set.len());
        data.extend_from_slice(response);
        data.extend_from_slice(domain_set);
        Self {
            attachment,
            data: data.into_boxed_slice(),
            response_len: response.len(),
            stored_at_unix: times[0],
            message_expires_at_unix: times[1],
            cache_expires_at_unix: times[2],
            wall_times,
        }
    }

    fn response(&self) -> &[u8] {
        &self.data[..self.response_len]
    }

    fn domain_set(&self) -> &[u8] {
        &self.data[self.response_len..]
    }

    fn snapshot(&self, key: Box<[u8]>) -> NativeSnapshotEntry {
        NativeSnapshotEntry {
            attachment: self.attachment.clone(),
            key: key.into_vec(),
            response: self.response().to_vec(),
            domain_set: self.domain_set().to_vec(),
            times: [
                self.stored_at_unix,
                self.message_expires_at_unix,
                self.cache_expires_at_unix,
            ],
            wall_times: self.wall_times,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct NativeLookup {
    pub attachment: Option<NativeAttachment>,
    pub state: LookupState,
    pub stored_at_unix: i64,
    pub message_expires_at_unix: i64,
    pub response: Vec<u8>,
    pub domain_set: Vec<u8>,
}

/// An owned native snapshot; runtime timestamps use the caller's monotonic epoch.
#[derive(Clone, Debug)]
pub struct NativeSnapshotEntry {
    pub attachment: Option<NativeAttachment>,
    pub key: Vec<u8>,
    pub response: Vec<u8>,
    pub domain_set: Vec<u8>,
    pub times: [i64; 3],
    pub wall_times: Option<[i64; 3]>,
}

/// Fully validated, owned native insertion prepared before a management commit.
pub struct PreparedNativeEntry {
    key: Box<[u8]>,
    entry: Arc<CacheEntry>,
}
impl PreparedNativeEntry {
    pub fn new(item: NativeSnapshotEntry) -> Result<Self, CacheError> {
        validate_store(
            &item.key,
            &item.response,
            item.times[0],
            item.times[1],
            item.times[2],
        )?;
        Ok(Self {
            entry: Arc::new(CacheEntry::new(
                item.attachment,
                &item.response,
                &item.domain_set,
                item.times,
                item.wall_times,
            )),
            key: item.key.into_boxed_slice(),
        })
    }
}

fn validate_store(
    key: &[u8],
    response: &[u8],
    stored_at_unix: i64,
    message_expires_at_unix: i64,
    cache_expires_at_unix: i64,
) -> Result<(), CacheError> {
    if key.is_empty() {
        return Err(CacheError::InvalidKey);
    }
    if response.is_empty() || wire::validate_response(response).is_err() {
        return Err(CacheError::InvalidResponse);
    }
    if message_expires_at_unix < stored_at_unix || cache_expires_at_unix < stored_at_unix {
        return Err(CacheError::InvalidExpiry);
    }
    Ok(())
}

fn lookup_data(
    cache: &NativeCache,
    key: &[u8],
    now_unix: i64,
) -> Result<Option<NativeLookup>, CacheError> {
    let Some(entry) = cache.state.entries.get(key) else {
        return Ok(None);
    };
    if now_unix >= entry.cache_expires_at_unix {
        cache.remove_entry(key, &entry);
        return Ok(None);
    }

    let (lookup_state, response) = if now_unix < entry.message_expires_at_unix {
        let elapsed = now_unix
            .saturating_sub(entry.stored_at_unix)
            .clamp(0, i64::from(u32::MAX));
        let elapsed = u32::try_from(elapsed).unwrap_or(u32::MAX);
        (LookupState::Fresh, wire::age_ttls(entry.response(), elapsed))
    } else if cache.state.config.lazy_cache_ttl_secs > 0 {
        (LookupState::Lazy, wire::set_ttls(entry.response(), 5))
    } else {
        return Ok(None);
    };
    let response = response.map_err(|_| {
        cache.remove_entry(key, &entry);
        CacheError::Internal
    })?;
    Ok(Some(NativeLookup {
        attachment: entry.attachment.clone(),
        state: lookup_state,
        stored_at_unix: entry.stored_at_unix,
        message_expires_at_unix: entry.message_expires_at_unix,
        response,
        domain_set: entry.domain_set().to_vec(),
    }))
}

impl NativeCache {
    pub fn new(config: CacheConfig) -> Result<Self, CacheError> {
        if config.capacity == 0 {
            return Err(CacheError::InvalidConfig);
        }
        let capacity = usize::try_from(config.capacity).unwrap_or(usize::MAX);
        Ok(Self {
            state: CacheState {
                config,
                entries: QuickCache::new(capacity),
            },
        })
    }

    /// Drops `key` only while it still holds `entry`, so a concurrent store of
    /// a fresh answer for the same key is never removed by a stale lookup.
    fn remove_entry(&self, key: &[u8], entry: &Arc<CacheEntry>) {
        // The removed value drops here, outside the shard lock.
        let _removed = self
            .state
            .entries
            .remove_if(key, |current| Arc::ptr_eq(current, entry));
    }

    pub fn store(
        &self,
        key: &[u8],
        response: &[u8],
        domain_set: &[u8],
        stored_at_unix: i64,
        message_expires_at_unix: i64,
        cache_expires_at_unix: i64,
    ) -> Result<(), CacheError> {
        self.store_timed(
            key,
            response,
            domain_set,
            [
                stored_at_unix,
                message_expires_at_unix,
                cache_expires_at_unix,
            ],
            None,
        )
    }

    /// Dual-clock insertion.
    pub fn store_timed(
        &self,
        key: &[u8],
        response: &[u8],
        domain_set: &[u8],
        times: [i64; 3],
        wall_times: Option<[i64; 3]>,
    ) -> Result<(), CacheError> {
        self.store_timed_with_attachment(key, response, domain_set, times, wall_times, None)
    }

    pub fn store_timed_with_attachment(
        &self,
        key: &[u8],
        response: &[u8],
        domain_set: &[u8],
        times: [i64; 3],
        wall_times: Option<[i64; 3]>,
        attachment: Option<NativeAttachment>,
    ) -> Result<(), CacheError> {
        validate_store(key, response, times[0], times[1], times[2])?;
        self.state.entries.insert(
            Box::from(key),
            Arc::new(CacheEntry::new(
                attachment, response, domain_set, times, wall_times,
            )),
        );
        Ok(())
    }

    /// Commits prevalidated entries without a recoverable partial-merge error.
    pub fn merge_prepared(&self, entries: Vec<PreparedNativeEntry>) {
        for item in entries {
            self.state.entries.insert(item.key, item.entry);
        }
    }

    /// Captures only entries whose complete retention window is still live.
    #[must_use]
    pub fn snapshot(&self, now: i64) -> Vec<NativeSnapshotEntry> {
        self.state
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                (entry.cache_expires_at_unix > now).then(|| entry.snapshot(key))
            })
            .collect()
    }

    /// Rejects oversized snapshots before duplicating live entry payloads.
    pub fn snapshot_bounded(
        &self,
        now: i64,
        max_entries: usize,
        max_bytes: usize,
    ) -> Result<Vec<NativeSnapshotEntry>, CacheError> {
        let mut snapshot = Vec::new();
        let mut bytes = 0_usize;
        for (key, entry) in self.state.entries.iter() {
            if entry.cache_expires_at_unix <= now {
                continue;
            }
            bytes = bytes
                .checked_add(key.len())
                .and_then(|n| n.checked_add(entry.data.len()))
                .ok_or(CacheError::InvalidConfig)?;
            if snapshot.len() >= max_entries || bytes > max_bytes {
                return Err(CacheError::InvalidConfig);
            }
            snapshot.push(entry.snapshot(key));
        }
        Ok(snapshot)
    }

    pub fn lookup(&self, key: &[u8], now_unix: i64) -> Result<Option<NativeLookup>, CacheError> {
        if key.is_empty() {
            return Err(CacheError::InvalidKey);
        }
        lookup_data(self, key, now_unix)
    }

    #[must_use]
    pub fn len(&self) -> u64 {
        u64::try_from(self.state.entries.len()).unwrap_or(u64::MAX)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Counts entries whose retention window is still live, dropping the
    /// dead ones on the way: they can never be served again, and counting in
    /// place avoids copying every key.
    #[must_use]
    pub fn live_len(&self, now: i64) -> usize {
        let live = Cell::new(0_usize);
        self.state.entries.retain(|_, entry| {
            let alive = entry.cache_expires_at_unix > now;
            if alive {
                live.set(live.get() + 1);
            }
            alive
        });
        live.get()
    }

    pub fn flush(&self) {
        self.state.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{CacheConfig, LookupState, NativeCache};

    const CONFIG: CacheConfig = CacheConfig {
        capacity: 4,
        lazy_cache_ttl_secs: 0,
        flags: 0,
    };

    fn response(ttl: u32, address: [u8; 4]) -> Vec<u8> {
        let mut packet = vec![
            0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x07, b'e',
            b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'o', b'r', b'g', 0x00, 0x00, 0x01, 0x00,
            0x01, 0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0x00, 0x04,
        ];
        packet.extend_from_slice(&address);
        packet[35..39].copy_from_slice(&ttl.to_be_bytes());
        packet
    }

    fn answer_ttl(packet: &[u8]) -> u32 {
        u32::from_be_bytes(packet[35..39].try_into().expect("answer TTL"))
    }

    #[test]
    fn native_attachment_tracks_entry_replacement_flush_and_import() {
        let cache = NativeCache::new(CONFIG).unwrap();
        let wire = response(60, [192, 0, 2, 1]);
        cache
            .store_timed_with_attachment(
                b"key",
                &wire,
                b"rules",
                [100, 160, 200],
                None,
                Some(super::NativeAttachment::new(7_u64)),
            )
            .unwrap();
        assert_eq!(
            cache
                .lookup(b"key", 101)
                .unwrap()
                .unwrap()
                .attachment
                .unwrap()
                .downcast_ref::<u64>(),
            Some(&7)
        );
        let snapshot = cache.snapshot(101);
        cache.merge_prepared(
            snapshot
                .into_iter()
                .map(|item| super::PreparedNativeEntry::new(item).unwrap())
                .collect(),
        );
        assert_eq!(
            cache
                .lookup(b"key", 101)
                .unwrap()
                .unwrap()
                .attachment
                .unwrap()
                .downcast_ref::<u64>(),
            Some(&7),
            "native snapshots preserve attachment"
        );
        cache
            .store_timed_with_attachment(
                b"key",
                &wire,
                b"rules",
                [100, 160, 200],
                None,
                Some(super::NativeAttachment::new(8_u64)),
            )
            .unwrap();
        cache.store(b"key", &wire, b"", 100, 160, 200).unwrap();
        assert!(
            cache
                .lookup(b"key", 101)
                .unwrap()
                .unwrap()
                .attachment
                .is_none()
        );
        cache.flush();
        assert!(cache.lookup(b"key", 101).unwrap().is_none());
    }

    #[test]
    fn native_cache_objects_are_isolated_and_return_owned_payloads() {
        let first = NativeCache::new(CONFIG).expect("first cache");
        let second = NativeCache::new(CONFIG).expect("second cache");
        let packet = response(60, [192, 0, 2, 1]);

        first
            .store(b"key", &packet, b"domain-a", 100, 160, 200)
            .expect("store");
        assert!(
            second
                .lookup(b"key", 101)
                .expect("isolated lookup")
                .is_none()
        );

        let mut hit = first.lookup(b"key", 101).expect("lookup").expect("hit");
        assert_eq!(hit.state, LookupState::Fresh);
        assert_eq!(hit.domain_set, b"domain-a");
        hit.response[0] = 0;
        hit.domain_set[0] = b'X';

        let fresh = first
            .lookup(b"key", 102)
            .expect("second lookup")
            .expect("hit");
        assert_eq!(answer_ttl(&fresh.response), 58);
        assert_eq!(fresh.response[41..45], packet[41..45]);
        assert_eq!(answer_ttl(&packet), 60);
        assert_eq!(fresh.domain_set, b"domain-a");
    }

    #[test]
    fn native_cache_overwrite_and_expiry_are_exact_without_cumulative_ttl_age() {
        let cache = NativeCache::new(CONFIG).expect("cache");
        let first = response(60, [192, 0, 2, 1]);
        let replacement = response(30, [192, 0, 2, 2]);

        cache
            .store(b"key", &first, &[], 100, 160, 200)
            .expect("first store");
        let first_hit = cache
            .lookup(b"key", 110)
            .expect("first lookup")
            .expect("hit");
        assert_eq!(answer_ttl(&first_hit.response), 50);
        let second_hit = cache
            .lookup(b"key", 111)
            .expect("second lookup")
            .expect("hit");
        assert_eq!(answer_ttl(&second_hit.response), 49);

        cache
            .store(b"key", &replacement, &[], 120, 150, 200)
            .expect("overwrite");
        let overwritten = cache
            .lookup(b"key", 121)
            .expect("overwrite lookup")
            .expect("hit");
        assert_eq!(overwritten.response[41..45], [192, 0, 2, 2]);
        assert_eq!(answer_ttl(&overwritten.response), 29);
        assert!(
            cache
                .lookup(b"key", 149)
                .expect("message expiry boundary predecessor")
                .is_some()
        );
        assert!(
            cache
                .lookup(b"key", 150)
                .expect("message expiry boundary")
                .is_none()
        );

        cache
            .store(b"cache-expiry", &replacement, &[], 200, 260, 205)
            .expect("cache expiry store");
        assert!(
            cache
                .lookup(b"cache-expiry", 204)
                .expect("cache expiry boundary predecessor")
                .is_some()
        );
        assert!(
            cache
                .lookup(b"cache-expiry", 205)
                .expect("cache expiry boundary")
                .is_none()
        );
    }

    #[test]
    fn stale_removal_keeps_a_newer_answer_for_the_same_key() {
        let cache = NativeCache::new(CONFIG).expect("cache");
        cache
            .store(b"key", &response(60, [192, 0, 2, 1]), &[], 100, 160, 200)
            .expect("first store");
        let stale = cache.state.entries.get(&b"key"[..]).expect("stored");
        cache
            .store(b"key", &response(60, [192, 0, 2, 2]), &[], 100, 160, 300)
            .expect("replacement");
        cache.remove_entry(b"key", &stale);
        let hit = cache.lookup(b"key", 150).expect("lookup").expect("kept");
        assert_eq!(hit.response[41..45], [192, 0, 2, 2]);

        let current = cache.state.entries.get(&b"key"[..]).expect("stored");
        cache.remove_entry(b"key", &current);
        assert!(cache.is_empty());
    }

    #[test]
    fn live_len_counts_live_entries_and_drops_dead_ones() {
        let cache = NativeCache::new(CONFIG).expect("cache");
        let packet = response(60, [192, 0, 2, 1]);
        cache.store(b"dead", &packet, &[], 100, 160, 200).unwrap();
        cache.store(b"live", &packet, &[], 100, 160, 300).unwrap();
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.live_len(199), 2);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.live_len(200), 1);
        assert_eq!(cache.len(), 1, "the expired entry is gone");
        assert!(cache.lookup(b"live", 150).unwrap().is_some());
    }

    #[test]
    fn native_cache_rejects_malformed_wire_and_keeps_capacity_bounded() {
        let cache = NativeCache::new(CONFIG).expect("cache");
        let packet = response(60, [192, 0, 2, 3]);
        let mut malformed = packet.clone();
        malformed.truncate(20);
        assert_eq!(
            cache.store(b"bad", &malformed, &[], 100, 160, 200),
            Err(super::CacheError::InvalidResponse)
        );

        for (key, address) in [
            (b"key-0", [192, 0, 2, 4]),
            (b"key-1", [192, 0, 2, 5]),
            (b"key-2", [192, 0, 2, 6]),
            (b"key-3", [192, 0, 2, 7]),
            (b"key-4", [192, 0, 2, 8]),
        ] {
            cache
                .store(key, &response(60, address), &[], 100, 160, 200)
                .expect("store");
        }
        assert!(cache.len() <= CONFIG.capacity);
    }
}
