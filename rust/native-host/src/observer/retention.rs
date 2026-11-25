//! Compact, lossless storage for terminal audit records.

use std::cmp::Ordering;
use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use atomdns_dns_core::{
    ResponseError, format_raw_rdata_into, format_txt_rdata_into, render_wire_name_into,
};

use super::{
    AnswerDetailsStatus, AuditAnswer, AuditRecord, ResponseDetails, ResponseFlags, TraceId,
};

pub(crate) const FLAG_AA: u8 = 0x01;
pub(crate) const FLAG_TC: u8 = 0x02;
pub(crate) const FLAG_RA: u8 = 0x04;
const STATUS_SHIFT: u8 = 3;
const STATUS_MASK: u8 = 0x18;

/// Static wire-decode failure codes produced by live response diagnosis.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[repr(u8)]
pub(crate) enum DecodeErrorCode {
    TruncatedMessage = 0,
    InvalidResponse = 1,
    BadName = 2,
    InvalidRdata = 3,
}

impl DecodeErrorCode {
    pub(crate) const fn from_response_error(error: ResponseError) -> Self {
        match error {
            ResponseError::TooShort
            | ResponseError::TruncatedQuestion
            | ResponseError::TruncatedRecord => Self::TruncatedMessage,
            ResponseError::NotResponse => Self::InvalidResponse,
            ResponseError::BadName => Self::BadName,
            ResponseError::InvalidRecordData => Self::InvalidRdata,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::TruncatedMessage => "truncated_message",
            Self::InvalidResponse => "invalid_response",
            Self::BadName => "bad_name",
            Self::InvalidRdata => "invalid_rdata",
        }
    }
}

pub(crate) const fn encode_flags(flags: &ResponseFlags) -> u8 {
    (if flags.aa { FLAG_AA } else { 0 })
        | (if flags.tc { FLAG_TC } else { 0 })
        | (if flags.ra { FLAG_RA } else { 0 })
}

pub(crate) const fn decode_flags(bits: u8) -> ResponseFlags {
    ResponseFlags {
        aa: bits & FLAG_AA != 0,
        tc: bits & FLAG_TC != 0,
        ra: bits & FLAG_RA != 0,
    }
}

pub(crate) const fn encode_flags_and_status(flags: u8, status: AnswerDetailsStatus) -> u8 {
    let status_bits = match status {
        AnswerDetailsStatus::Complete => 0,
        AnswerDetailsStatus::RawRdata => 1,
        AnswerDetailsStatus::DecodeError => 2,
    };
    (flags & 0x07) | (status_bits << STATUS_SHIFT)
}

const fn decode_status(bits: u8) -> AnswerDetailsStatus {
    match (bits & STATUS_MASK) >> STATUS_SHIFT {
        0 => AnswerDetailsStatus::Complete,
        1 => AnswerDetailsStatus::RawRdata,
        _ => AnswerDetailsStatus::DecodeError,
    }
}

/// Compact payload stored inside `Arc<CompactResponse>` for responses that do
/// not fit inline in [`RetainedResponse`].
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) enum CompactPayload {
    /// One AAAA record stored inline (no secondary heap allocation).
    SingleAaaa { ttl: u32, ip: [u8; 16] },
    /// Two A records stored inline (no secondary heap allocation).
    TwoA {
        ttl1: u32,
        ip1: [u8; 4],
        ttl2: u32,
        ip2: [u8; 4],
    },
    /// Packed Answer-section records in wire order:
    /// `[rrtype: u16 LE][ttl: u32 LE][len: u16 LE][payload: len bytes]`.
    ///
    /// Payload encoding by `rrtype`:
    /// - `1` (A): 4 raw IPv4 octets;
    /// - `28` (AAAA): 16 raw IPv6 octets;
    /// - `2 | 5 | 12 | 15` (NS/CNAME/PTR/MX): expanded uncompressed wire name
    ///   (MX 2-byte preference already stripped);
    /// - `16` (TXT): validated raw TXT RDATA (length-prefixed chunks);
    /// - all other types: raw RDATA bytes (`RawRdata` status bit set).
    Packed(Box<[u8]>),
    /// Lossless fallback for arbitrary test/synthetic `ResponseDetails` values.
    Verbatim(Box<ResponseDetails>),
}

/// Shared compact response payload deduplicated in `AuditRetention::responses`.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct CompactResponse {
    pub rcode: u16,
    pub flags_and_status: u8,
    pub payload: CompactPayload,
}

impl CompactResponse {
    pub(crate) fn to_response_details(&self) -> ResponseDetails {
        match &self.payload {
            CompactPayload::SingleAaaa { ttl, ip } => ResponseDetails {
                rcode: self.rcode,
                flags: decode_flags(self.flags_and_status),
                answers: vec![AuditAnswer {
                    rrtype: 28,
                    ttl: *ttl,
                    data: Ipv6Addr::from(*ip).to_string(),
                }],
                answer_details_status: AnswerDetailsStatus::Complete,
                answer_decode_error: None,
            },
            CompactPayload::TwoA {
                ttl1,
                ip1,
                ttl2,
                ip2,
            } => ResponseDetails {
                rcode: self.rcode,
                flags: decode_flags(self.flags_and_status),
                answers: vec![
                    AuditAnswer {
                        rrtype: 1,
                        ttl: *ttl1,
                        data: Ipv4Addr::from(*ip1).to_string(),
                    },
                    AuditAnswer {
                        rrtype: 1,
                        ttl: *ttl2,
                        data: Ipv4Addr::from(*ip2).to_string(),
                    },
                ],
                answer_details_status: AnswerDetailsStatus::Complete,
                answer_decode_error: None,
            },
            CompactPayload::Packed(blob) => {
                let mut answers = Vec::new();
                for_each_packed_record(blob, |rrtype, ttl, payload| {
                    let data = format_packed_answer_data(rrtype, payload);
                    answers.push(AuditAnswer { rrtype, ttl, data });
                    false
                });
                ResponseDetails {
                    rcode: self.rcode,
                    flags: decode_flags(self.flags_and_status),
                    answers,
                    answer_details_status: decode_status(self.flags_and_status),
                    answer_decode_error: None,
                }
            }
            CompactPayload::Verbatim(details) => (**details).clone(),
        }
    }
}

/// Appends one Answer record into a packed binary buffer.
pub(crate) fn push_packed_record(buf: &mut Vec<u8>, rrtype: u16, ttl: u32, payload: &[u8]) {
    let len = u16::try_from(payload.len()).expect("DNS RR payload fits u16");
    buf.extend_from_slice(&rrtype.to_le_bytes());
    buf.extend_from_slice(&ttl.to_le_bytes());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(payload);
}

/// Visits every packed Answer record in wire order; stops early when `visitor`
/// returns `true`.
pub(crate) fn for_each_packed_record(
    blob: &[u8],
    mut visitor: impl FnMut(u16, u32, &[u8]) -> bool,
) -> bool {
    let mut pos = 0;
    while let Some(header) = blob.get(pos..pos + 8) {
        let rrtype = u16::from_le_bytes([header[0], header[1]]);
        let ttl = u32::from_le_bytes([header[2], header[3], header[4], header[5]]);
        let len = usize::from(u16::from_le_bytes([header[6], header[7]]));
        pos += 8;
        let Some(payload) = blob.get(pos..pos + len) else {
            break;
        };
        pos += len;
        if visitor(rrtype, ttl, payload) {
            return true;
        }
    }
    false
}

/// Formats one packed Answer payload identically to `dns_core::observe_answer_records`.
pub(crate) fn format_packed_answer_data_into(rrtype: u16, payload: &[u8], out: &mut String) {
    use std::fmt::Write;
    match rrtype {
        1 => {
            if let Ok(octets) = <[u8; 4]>::try_from(payload) {
                let _ = write!(out, "{}", Ipv4Addr::from(octets));
            }
        }
        28 => {
            if let Ok(octets) = <[u8; 16]>::try_from(payload) {
                let _ = write!(out, "{}", Ipv6Addr::from(octets));
            }
        }
        2 | 5 | 12 | 15 => render_wire_name_into(payload, out),
        16 => {
            let _ = format_txt_rdata_into(payload, out);
        }
        _ => format_raw_rdata_into(payload, out),
    }
}

fn format_packed_answer_data(rrtype: u16, payload: &[u8]) -> String {
    let mut out = String::new();
    format_packed_answer_data_into(rrtype, payload, &mut out);
    out
}

/// Hot-path unshared response diagnosis carried on `ExecutionResult` and
/// `TerminalObservation` without formatting Answer text.
#[derive(Clone, Debug)]
pub(crate) enum AuditResponse {
    Empty {
        rcode: u16,
        flags: u8,
    },
    DecodeError {
        rcode: u16,
        flags: u8,
        error: DecodeErrorCode,
    },
    SingleA {
        rcode: u16,
        flags: u8,
        ttl: u32,
        ip: [u8; 4],
    },
    Compact(CompactResponse),
}

impl Default for AuditResponse {
    fn default() -> Self {
        Self::no_response()
    }
}

impl AuditResponse {
    pub(crate) const fn no_response() -> Self {
        Self::Empty { rcode: 0, flags: 0 }
    }

    pub(crate) fn from_details(details: ResponseDetails) -> Self {
        if details.answers.is_empty()
            && details.answer_details_status == AnswerDetailsStatus::Complete
            && details.answer_decode_error.is_none()
        {
            return Self::Empty {
                rcode: details.rcode,
                flags: encode_flags(&details.flags),
            };
        }
        let rcode = details.rcode;
        let flags_and_status =
            encode_flags_and_status(encode_flags(&details.flags), details.answer_details_status);
        Self::Compact(CompactResponse {
            rcode,
            flags_and_status,
            payload: CompactPayload::Verbatim(Box::new(details)),
        })
    }

    #[cfg(test)]
    pub(crate) fn to_response_details(&self) -> ResponseDetails {
        match self {
            Self::Empty { rcode, flags } => ResponseDetails {
                rcode: *rcode,
                flags: decode_flags(*flags),
                answers: Vec::new(),
                answer_details_status: AnswerDetailsStatus::Complete,
                answer_decode_error: None,
            },
            Self::DecodeError {
                rcode,
                flags,
                error,
            } => ResponseDetails {
                rcode: *rcode,
                flags: decode_flags(*flags),
                answers: Vec::new(),
                answer_details_status: AnswerDetailsStatus::DecodeError,
                answer_decode_error: Some(error.as_str().to_owned()),
            },
            Self::SingleA {
                rcode,
                flags,
                ttl,
                ip,
            } => ResponseDetails {
                rcode: *rcode,
                flags: decode_flags(*flags),
                answers: vec![AuditAnswer {
                    rrtype: 1,
                    ttl: *ttl,
                    data: Ipv4Addr::from(*ip).to_string(),
                }],
                answer_details_status: AnswerDetailsStatus::Complete,
                answer_decode_error: None,
            },
            Self::Compact(compact) => compact.to_response_details(),
        }
    }
}

/// Compact response stored inside [`RetainedAuditRecord`] (16 bytes).
///
/// The three hottest shapes (`Empty`, `DecodeError`, `SingleA`) live inline
/// with zero heap allocation and skip `SharingCache`; all other responses
/// share a deduplicated [`CompactResponse`].
#[derive(Clone, Debug)]
pub(crate) enum RetainedResponse {
    Empty {
        rcode: u16,
        flags: u8,
    },
    DecodeError {
        rcode: u16,
        flags: u8,
        error: DecodeErrorCode,
    },
    SingleA {
        rcode: u16,
        flags: u8,
        ttl: u32,
        ip: [u8; 4],
    },
    Shared(Arc<CompactResponse>),
}

impl RetainedResponse {
    pub(crate) fn to_response_details(&self) -> ResponseDetails {
        match self {
            Self::Empty { rcode, flags } => ResponseDetails {
                rcode: *rcode,
                flags: decode_flags(*flags),
                answers: Vec::new(),
                answer_details_status: AnswerDetailsStatus::Complete,
                answer_decode_error: None,
            },
            Self::DecodeError {
                rcode,
                flags,
                error,
            } => ResponseDetails {
                rcode: *rcode,
                flags: decode_flags(*flags),
                answers: Vec::new(),
                answer_details_status: AnswerDetailsStatus::DecodeError,
                answer_decode_error: Some(error.as_str().to_owned()),
            },
            Self::SingleA {
                rcode,
                flags,
                ttl,
                ip,
            } => ResponseDetails {
                rcode: *rcode,
                flags: decode_flags(*flags),
                answers: vec![AuditAnswer {
                    rrtype: 1,
                    ttl: *ttl,
                    data: Ipv4Addr::from(*ip).to_string(),
                }],
                answer_details_status: AnswerDetailsStatus::Complete,
                answer_decode_error: None,
            },
            Self::Shared(compact) => compact.to_response_details(),
        }
    }
}

const _: () = assert!(std::mem::size_of::<RetainedResponse>() <= 16);

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
pub(crate) struct AuditRetention {
    templates: SharingCache<AuditRecord>,
    responses: SharingCache<CompactResponse>,
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
    response: RetainedResponse,
    template: Arc<AuditRecord>,
}

impl RetainedAuditRecord {
    /// Common facts only. Per-row fields must be read from the retained record;
    /// expansion is required before passing a full AuditRecord to a projector.
    pub(crate) fn common(&self) -> &AuditRecord {
        &self.template
    }

    pub(crate) fn response(&self) -> &RetainedResponse {
        &self.response
    }

    #[cfg(test)]
    pub(crate) fn template_arc(&self) -> &Arc<AuditRecord> {
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
            response_details: self.response.to_response_details(),
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
    #[cfg(test)]
    pub(super) fn retain(&mut self, mut record: AuditRecord) -> RetainedAuditRecord {
        let response = AuditResponse::from_details(std::mem::replace(
            &mut record.response_details,
            ResponseDetails::no_response(),
        ));
        self.retain_with_response(record, response)
    }

    pub(crate) fn retain_with_response(
        &mut self,
        mut record: AuditRecord,
        response: AuditResponse,
    ) -> RetainedAuditRecord {
        record.response_details = ResponseDetails::no_response();
        let elapsed = std::mem::replace(&mut record.elapsed, Duration::ZERO);
        let retained_response = match response {
            AuditResponse::Empty { rcode, flags } => RetainedResponse::Empty { rcode, flags },
            AuditResponse::DecodeError {
                rcode,
                flags,
                error,
            } => RetainedResponse::DecodeError {
                rcode,
                flags,
                error,
            },
            AuditResponse::SingleA {
                rcode,
                flags,
                ttl,
                ip,
            } => RetainedResponse::SingleA {
                rcode,
                flags,
                ttl,
                ip,
            },
            AuditResponse::Compact(compact) => {
                RetainedResponse::Shared(self.responses.share(compact, |compact| {
                    if let CompactPayload::Verbatim(details) = &mut compact.payload {
                        details.answers.shrink_to_fit();
                        for answer in &mut details.answers {
                            answer.data.shrink_to_fit();
                        }
                        if let Some(error) = &mut details.answer_decode_error {
                            error.shrink_to_fit();
                        }
                    }
                }))
            }
        };
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
            response: retained_response,
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
        // Plus 16 B Arc header = 112 B, same mimalloc 112 B size class as the
        // former 104 B (no production memory increase).
        assert!(std::mem::size_of::<RetainedAuditRecord>() <= 96);
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

    #[test]
    fn compact_and_verbatim_responses_round_trip_losslessly() {
        use super::{
            AuditResponse, CompactPayload, CompactResponse, DecodeErrorCode, FLAG_AA, FLAG_RA,
            FLAG_TC, RetainedResponse, encode_flags_and_status, push_packed_record,
        };
        use crate::observer::{AnswerDetailsStatus, AuditAnswer, ResponseDetails, ResponseFlags};

        let mut packed = Vec::new();
        push_packed_record(&mut packed, 5, 120, &[5, b'a', b'l', b'i', b'a', b's', 0]);
        push_packed_record(&mut packed, 1, 60, &[192, 0, 2, 1]);
        push_packed_record(
            &mut packed,
            16,
            30,
            &[3, b'f', b'o', b'o', 3, b'b', b'a', b'r'],
        );
        push_packed_record(&mut packed, 65000, 0, &[0xde, 0xad]);

        let cases = [
            AuditResponse::Empty {
                rcode: 3,
                flags: FLAG_AA | FLAG_RA,
            },
            AuditResponse::DecodeError {
                rcode: 16,
                flags: FLAG_TC,
                error: DecodeErrorCode::InvalidRdata,
            },
            AuditResponse::SingleA {
                rcode: 0,
                flags: FLAG_RA,
                ttl: 300,
                ip: [198, 51, 100, 7],
            },
            AuditResponse::Compact(CompactResponse {
                rcode: 0,
                flags_and_status: encode_flags_and_status(FLAG_RA, AnswerDetailsStatus::Complete),
                payload: CompactPayload::SingleAaaa {
                    ttl: 45,
                    ip: [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7],
                },
            }),
            AuditResponse::Compact(CompactResponse {
                rcode: 0,
                flags_and_status: encode_flags_and_status(FLAG_AA, AnswerDetailsStatus::Complete),
                payload: CompactPayload::TwoA {
                    ttl1: 10,
                    ip1: [192, 0, 2, 1],
                    ttl2: 20,
                    ip2: [192, 0, 2, 2],
                },
            }),
            AuditResponse::Compact(CompactResponse {
                rcode: 0,
                flags_and_status: encode_flags_and_status(FLAG_RA, AnswerDetailsStatus::RawRdata),
                payload: CompactPayload::Packed(packed.into_boxed_slice()),
            }),
        ];

        for response in cases {
            let expected = response.to_response_details();
            let retained = match response {
                AuditResponse::Empty { rcode, flags } => RetainedResponse::Empty { rcode, flags },
                AuditResponse::DecodeError {
                    rcode,
                    flags,
                    error,
                } => RetainedResponse::DecodeError {
                    rcode,
                    flags,
                    error,
                },
                AuditResponse::SingleA {
                    rcode,
                    flags,
                    ttl,
                    ip,
                } => RetainedResponse::SingleA {
                    rcode,
                    flags,
                    ttl,
                    ip,
                },
                AuditResponse::Compact(compact) => RetainedResponse::Shared(Arc::new(compact)),
            };
            assert_eq!(retained.to_response_details(), expected);
        }

        // Arbitrary synthetic ResponseDetails (custom error + non-empty answers + UTF-8 data)
        // must round-trip losslessly via Verbatim.
        let synthetic = ResponseDetails {
            rcode: 1,
            flags: ResponseFlags {
                aa: false,
                tc: true,
                ra: true,
            },
            answers: vec![
                AuditAnswer {
                    rrtype: 1,
                    ttl: 65,
                    data: "192.0.2.8".to_owned(),
                },
                AuditAnswer {
                    rrtype: 65000,
                    ttl: 0,
                    data: "\\000 arbitrary 原始".to_owned(),
                },
            ],
            answer_details_status: AnswerDetailsStatus::DecodeError,
            answer_decode_error: Some("truncated answer".to_owned()),
        };
        let from_details = AuditResponse::from_details(synthetic.clone());
        assert_eq!(from_details.to_response_details(), synthetic);
    }
}
