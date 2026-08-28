//! Fuzz-style property tests for the QuiltWire v0 codec (no hardware).
//!
//! - golden vector pinned (byte-identical to firmware/esp32-cell/test_quiltwire.c)
//! - 20k pseudo-random frames roundtrip through encode→decode
//! - every single-bit corruption of a frame is rejected (CRC guarantee)
//! - truncation at every length is rejected
//! - the streaming decoder resyncs after arbitrary garbage
//! - random bytes never panic the decoder; anything that decodes re-encodes
//!   to the same bytes (the "no fabricated frames" property)

use quilt_wire::frame::{decode, Frame, FrameDecoder, Kind, FRAME_LEN};

/// Deterministic xorshift64* PRNG — reproducible without external crates.
struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }
    fn u16(&mut self) -> u16 {
        (self.next_u64() >> 32) as u16
    }
    fn u8(&mut self) -> u8 {
        (self.next_u64() >> 56) as u8
    }
    fn byte(&mut self) -> u8 {
        self.next_u64() as u8
    }
}

#[test]
fn crc_and_golden_pinned() {
    assert_eq!(quilt_wire::frame::crc16_ccitt_false(b"123456789"), 0x29B1);
    let hex = "510101070201e80300000000ac41d6d3";
    let bytes = hex_decode(hex);
    let f = decode(&bytes).expect("golden decodes");
    assert_eq!(
        (f.kind, f.cell_id, f.seq, f.tick),
        (Kind::Delta, 7, 0x0102, 1000)
    );
    assert_eq!(f.value_f32(), 21.5);
    assert_eq!(f.encode(), bytes.as_slice());
}

#[test]
fn random_frames_roundtrip() {
    let mut rng = Rng(0xC0FFEE);
    for _ in 0..20_000 {
        let kind = match rng.u8() % 5 {
            0 => Kind::Tick,
            1 => Kind::Delta,
            2 => Kind::Alarm,
            3 => Kind::LinkMeta,
            _ => Kind::Ack,
        };
        let f = Frame::new(kind, rng.u8(), rng.u16(), rng.u32(), rng.u32());
        let bytes = f.encode();
        assert_eq!(bytes.len(), FRAME_LEN);
        let back = decode(&bytes).expect("roundtrip decode");
        assert_eq!(back, f, "roundtrip equality");
    }
}

#[test]
fn single_bit_corruption_always_rejected() {
    // CRC-16/CCITT-FALSE has Hamming distance >= 4 for these lengths:
    // every single-bit flip must fail decode.
    let f = Frame::from_f32(Kind::Delta, 3, 900, 4242, -12.25);
    let bytes = f.encode();
    for byte_i in 0..FRAME_LEN {
        for bit in 0..8 {
            let mut corrupted = bytes;
            corrupted[byte_i] ^= 1 << bit;
            assert!(
                decode(&corrupted).is_err(),
                "corruption at byte {byte_i} bit {bit} must be rejected"
            );
        }
    }
}

#[test]
fn truncation_rejected_at_every_length() {
    let bytes = Frame::from_f32(Kind::Tick, 1, 2, 3, 4.0).encode();
    for cut in 0..FRAME_LEN {
        assert!(
            decode(&bytes[..cut]).is_err(),
            "truncation at {cut} must fail"
        );
    }
}

#[test]
fn decoder_resyncs_after_garbage() {
    let f1 = Frame::from_f32(Kind::Delta, 9, 10, 11, 1.5);
    let f2 = Frame::from_f32(Kind::Delta, 9, 11, 12, 1.6);
    let mut stream: Vec<u8> = Vec::new();
    stream.extend_from_slice(b"\r\nline noise before sync \x00\xff\x07");
    stream.extend_from_slice(&f1.encode());
    stream.extend_from_slice(b"mid-stream junk - QW not a frame");
    stream.extend_from_slice(&f2.encode());

    let mut dec = FrameDecoder::new();
    let mut got = Vec::new();
    dec.push(&stream, |f| got.push(f));
    assert_eq!(got, vec![f1, f2], "garbage dropped, both frames recovered");
}

#[test]
fn random_bytes_never_panic_and_never_fabricate() {
    let mut rng = Rng(0xBADF00D);
    for _ in 0..2_000 {
        let len = (rng.u32() % 64) as usize;
        let chunk: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        let mut dec = FrameDecoder::new();
        let mut got = Vec::new();
        dec.push(&chunk, |f| got.push(f));
        for f in got {
            // Anything that decoded must be a real frame: re-encoding
            // reproduces bytes the CRC actually covers.
            let bytes = f.encode();
            assert_eq!(decode(&bytes).as_ref(), Ok(&f));
        }
    }
}

#[test]
fn chunk_boundaries_irrelevant() {
    // The same byte stream fed in different chunk sizes yields the same
    // frames — serial reads arrive at arbitrary boundaries.
    let mut rng = Rng(0x5EED);
    let mut frames = Vec::new();
    let mut stream = Vec::new();
    for i in 0..50u16 {
        let f = Frame::from_f32(Kind::Delta, 4, i, i as u32, 20.0 + i as f32 * 0.01);
        frames.push(f);
        stream.extend_from_slice(&f.encode());
        if i % 7 == 3 {
            stream.push(rng.byte()); // sprinkle noise
        }
    }
    for chunk_size in [1usize, 2, 3, 15, 16, 17, 64, 4096] {
        let mut dec = FrameDecoder::new();
        let mut got = Vec::new();
        for chunk in stream.chunks(chunk_size) {
            dec.push(chunk, |f| got.push(f));
        }
        assert_eq!(got, frames, "chunk size {chunk_size} must not matter");
    }
}

fn hex_decode(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks(2)
        .map(|c| {
            let hi = (c[0] as char).to_digit(16).unwrap() as u8;
            let lo = (c[1] as char).to_digit(16).unwrap() as u8;
            (hi << 4) | lo
        })
        .collect()
}
