//! Optional TLV tail (after byte 16, when the transport MTU allows).
//!
//! Pure core, no alloc: TLVs are walked in place via an iterator.

pub const TLV_REASON: u8 = 0x01;

/// One decoded TLV pointing into the source buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub kind: u8,
    pub value: &'a [u8],
}

/// Walk the TLV tail of a buffer whose first [`crate::FRAME_LEN`] bytes are a
/// QuiltWire frame. Returns `None` on malformed tail (truncated header or
/// value overrun) — strict, because a tail that doesn't parse is a framing
/// fact the caller should know, not silently skip.
pub fn tlvs(tail: &[u8]) -> Option<impl Iterator<Item = Tlv<'_>>> {
    // Validate the whole tail structure up front so the iterator can yield
    // safely (a malformed tail yields nothing after validation fails).
    let mut i = 0usize;
    while i < tail.len() {
        if i + 2 > tail.len() {
            return None; // truncated type/len header
        }
        let len = tail[i + 1] as usize;
        let end = i + 2 + len;
        if end > tail.len() {
            return None; // value overruns buffer
        }
        i = end;
    }
    let mut pos = 0usize;
    Some(core::iter::from_fn(move || {
        if pos >= tail.len() {
            return None;
        }
        let kind = tail[pos];
        let len = tail[pos + 1] as usize;
        let value = &tail[pos + 2..pos + 2 + len];
        pos += 2 + len;
        Some(Tlv { kind, value })
    }))
}

/// The declared half of subtext: reason + considered-mask (TLV 0x01).
/// Present ONLY when the sender had >= 2 live links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteReason {
    pub reason: Reason,
    pub considered: u8,
}

/// Why the sender's policy chose this link (1 byte). Matches
/// LINK-LAYER-FEASIBILITY.md §2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Reason {
    Cheapest = 0,
    Fastest = 1,
    Only = 2,
    Reliable = 3,
    Urgent = 4,
    Bulk = 5,
}

impl Reason {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Reason::Cheapest),
            1 => Some(Reason::Fastest),
            2 => Some(Reason::Only),
            3 => Some(Reason::Reliable),
            4 => Some(Reason::Urgent),
            5 => Some(Reason::Bulk),
            _ => None,
        }
    }
}

impl RouteReason {
    /// Decode a TLV 0x01 value.
    pub fn parse(value: &[u8]) -> Option<Self> {
        if value.len() != 2 {
            return None;
        }
        Some(RouteReason {
            reason: Reason::from_u8(value[0])?,
            considered: value[1],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_tlvs() {
        let tail: &[u8] = &[0x01, 0x02, 0x03, 0x07, 0x05, 0x01, b'x'];
        let list: Vec<Tlv> = tlvs(tail).unwrap().collect();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].kind, 0x01);
        assert_eq!(list[0].value, &[0x03, 0x07]);
        let rr = RouteReason::parse(list[0].value).unwrap();
        assert_eq!(rr.reason, Reason::Reliable);
        assert_eq!(rr.considered, 0x07);
        assert_eq!(list[1].value, b"x");
    }

    #[test]
    fn malformed_tail_is_strict() {
        assert!(tlvs(&[0x01, 0x05, 0x00]).is_none()); // overrun
        assert!(tlvs(&[0x01]).is_none()); // truncated header
        assert_eq!(tlvs(&[]).unwrap().count(), 0);
    }
}
