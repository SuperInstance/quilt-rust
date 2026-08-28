//! QuiltWire v0 frame: layout, CRC16-CCITT-FALSE codec, streaming decoder
//! with resync. Pure core: no alloc, no std. See `lib.rs` for the pinned
//! byte layout.

pub const MAGIC: u8 = 0x51; // 'Q'
pub const VERSION: u8 = 0x01;
/// Fixed frame size in bytes.
pub const FRAME_LEN: usize = 16;
/// CRC covers bytes 0..=13.
const CRC_SPAN: usize = 14;

/// Frame kinds (byte 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Kind {
    /// Heartbeat: "I am alive", value may be unchanged.
    Tick = 0x00,
    /// Value changed by more than the sender's epsilon.
    Delta = 0x01,
    /// Alarm: cost no object, redundant fire expected.
    Alarm = 0x02,
    /// Link metadata observation from the sender's side.
    LinkMeta = 0x03,
    /// Transport-layer acknowledgement.
    Ack = 0x04,
}

impl Kind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x00 => Some(Kind::Tick),
            0x01 => Some(Kind::Delta),
            0x02 => Some(Kind::Alarm),
            0x03 => Some(Kind::LinkMeta),
            0x04 => Some(Kind::Ack),
            _ => None,
        }
    }

    /// Lowercase name used in walks/2 payloads (stable string form).
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Tick => "tick",
            Kind::Delta => "delta",
            Kind::Alarm => "alarm",
            Kind::LinkMeta => "linkmeta",
            Kind::Ack => "ack",
        }
    }
}

/// A decoded QuiltWire v0 frame. `value` keeps raw bits — it may be any
/// IEEE-754 bit pattern the sender put there (including non-finite); no
/// interpretation happens at the link layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub kind: Kind,
    pub cell_id: u8,
    /// Wrapping sequence number (gap detection = reliability observation).
    pub seq: u16,
    /// Sender-local tick. No cross-clock claims.
    pub tick: u32,
    /// Raw f32 payload bits.
    pub value_bits: u32,
}

impl Frame {
    pub fn new(kind: Kind, cell_id: u8, seq: u16, tick: u32, value_bits: u32) -> Self {
        Frame {
            kind,
            cell_id,
            seq,
            tick,
            value_bits,
        }
    }

    /// Convenience constructor taking the value as f32.
    pub fn from_f32(kind: Kind, cell_id: u8, seq: u16, tick: u32, value: f32) -> Self {
        Self::new(kind, cell_id, seq, tick, value.to_bits())
    }

    /// The f32 value (bits passed through unchanged).
    pub fn value_f32(&self) -> f32 {
        f32::from_bits(self.value_bits)
    }

    /// Encode into exactly [`FRAME_LEN`] bytes.
    pub fn encode_to(&self, out: &mut [u8; FRAME_LEN]) {
        out[0] = MAGIC;
        out[1] = VERSION;
        out[2] = self.kind as u8;
        out[3] = self.cell_id;
        out[4] = self.seq as u8;
        out[5] = (self.seq >> 8) as u8;
        out[6] = self.tick as u8;
        out[7] = (self.tick >> 8) as u8;
        out[8] = (self.tick >> 16) as u8;
        out[9] = (self.tick >> 24) as u8;
        out[10] = self.value_bits as u8;
        out[11] = (self.value_bits >> 8) as u8;
        out[12] = (self.value_bits >> 16) as u8;
        out[13] = (self.value_bits >> 24) as u8;
        let crc = crc16_ccitt_false(&out[..CRC_SPAN]);
        out[14] = crc as u8;
        out[15] = (crc >> 8) as u8;
    }

    /// Encode into a fresh array.
    pub fn encode(&self) -> [u8; FRAME_LEN] {
        let mut buf = [0u8; FRAME_LEN];
        self.encode_to(&mut buf);
        buf
    }
}

/// CRC-16/CCITT-FALSE: poly 0x1021, init 0xFFFF, no reflection, xorout 0.
/// Bitwise implementation — no table, no_std, embedded-friendly.
/// Check value: `crc16_ccitt_false(b"123456789") == 0x29B1`.
pub fn crc16_ccitt_false(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Fewer than FRAME_LEN bytes.
    TooShort,
    /// Magic/version mismatch.
    BadHeader,
    /// Kind byte is not a known kind.
    BadKind(u8),
    /// CRC mismatch (torn or corrupted frame).
    BadCrc { got: u16, want: u16 },
}

/// Decode one frame from exactly [`FRAME_LEN`] bytes.
pub fn decode(buf: &[u8]) -> Result<Frame, DecodeError> {
    if buf.len() < FRAME_LEN {
        return Err(DecodeError::TooShort);
    }
    if buf[0] != MAGIC || buf[1] != VERSION {
        return Err(DecodeError::BadHeader);
    }
    let kind = Kind::from_u8(buf[2]).ok_or(DecodeError::BadKind(buf[2]))?;
    let crc_stored = u16::from_le_bytes([buf[14], buf[15]]);
    let crc_calc = crc16_ccitt_false(&buf[..CRC_SPAN]);
    if crc_stored != crc_calc {
        return Err(DecodeError::BadCrc {
            got: crc_stored,
            want: crc_calc,
        });
    }
    Ok(Frame {
        kind,
        cell_id: buf[3],
        seq: u16::from_le_bytes([buf[4], buf[5]]),
        tick: u32::from_le_bytes([buf[6], buf[7], buf[8], buf[9]]),
        value_bits: u32::from_le_bytes([buf[10], buf[11], buf[12], buf[13]]),
    })
}

/// Streaming decoder for a serial-style byte stream.
///
/// Bytes are pushed in arbitrary chunks; frames pop out when a full, valid
/// 16-byte frame has been seen. On garbage (line noise, torn frames, baud
/// glitches) the decoder **resyncs**: it drops one byte and re-scans for the
/// magic+version+CRC pattern, so a stream never permanently desynchronises.
///
/// No alloc: an internal fixed buffer holds at most [`MAX_BUFFER`] bytes.
/// Frames longer than the buffer with TLVs attached are not handled here —
/// serial v0 sends bare 16-byte frames; TLV-bearing buffers (ESP-Now MTU)
/// are decoded by [`decode`] on the frame prefix plus [`crate::tlv`] walking
/// the tail.
pub struct FrameDecoder {
    buf: [u8; Self::MAX_BUFFER],
    len: usize,
}

impl FrameDecoder {
    /// Internal buffer size. Comfortably above one frame + resync slack.
    pub const MAX_BUFFER: usize = 128;

    pub fn new() -> Self {
        FrameDecoder {
            buf: [0u8; Self::MAX_BUFFER],
            len: 0,
        }
    }

    /// Push a chunk of bytes; call the visitor for every decoded frame, in
    /// arrival order. Visitor may not re-entrantly use this decoder.
    ///
    /// Returns the number of frames decoded from this chunk.
    pub fn push<F: FnMut(Frame)>(&mut self, chunk: &[u8], mut on_frame: F) -> usize {
        let mut n = 0usize;
        for &b in chunk {
            if self.len < Self::MAX_BUFFER {
                self.buf[self.len] = b;
                self.len += 1;
            } else {
                // Buffer full without a valid frame: shift left, losing the
                // oldest byte (resync semantics — noise is dropped, frames
                // are not fabricated).
                self.buf.copy_within(1.., 0);
                self.buf[Self::MAX_BUFFER - 1] = b;
            }
            if self.len >= FRAME_LEN {
                match decode(&self.buf[..FRAME_LEN]) {
                    Ok(frame) => {
                        on_frame(frame);
                        n += 1;
                        // Drain the consumed frame.
                        self.buf.copy_within(FRAME_LEN..self.len, 0);
                        self.len -= FRAME_LEN;
                    }
                    Err(_) => {
                        // Not a frame start: drop the oldest byte and rescan.
                        self.buf.copy_within(1..self.len, 0);
                        self.len -= 1;
                    }
                }
            }
        }
        n
    }
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned golden vector — byte-identical to firmware/esp32-cell golden
    /// (see test_quiltwire.c and tests/roundtrip.rs).
    pub const GOLDEN_HEX: &str = "510101070201e80300000000ac41d6d3";

    #[test]
    fn crc_check_value() {
        assert_eq!(crc16_ccitt_false(b"123456789"), 0x29B1);
    }

    #[test]
    fn golden_vector() {
        let bytes: [u8; FRAME_LEN] = {
            let mut b = [0u8; FRAME_LEN];
            let hex = GOLDEN_HEX.as_bytes();
            for i in 0..FRAME_LEN {
                let hi = (hex[i * 2] as char).to_digit(16).unwrap() as u8;
                let lo = (hex[i * 2 + 1] as char).to_digit(16).unwrap() as u8;
                b[i] = (hi << 4) | lo;
            }
            b
        };
        let f = decode(&bytes).expect("golden decodes");
        assert_eq!(f.kind, Kind::Delta);
        assert_eq!(f.cell_id, 7);
        assert_eq!(f.seq, 0x0102);
        assert_eq!(f.tick, 1000);
        assert_eq!(f.value_f32(), 21.5);
        // And re-encodes to the identical bytes.
        assert_eq!(f.encode(), bytes);
    }
}
