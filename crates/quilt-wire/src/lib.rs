//! # QuiltWire v0 — the link-core
//!
//! Transport-blind 16-byte frame + (std-only) a desktop arrival peer that
//! stamps decoded frames into `walks/2`-compatible JSONL.
//!
//! - Frame layout, endianness, and CRC: [`frame`] (pinned below).
//! - Stream decoding with resync: [`frame::FrameDecoder`].
//! - Seq continuity / restart detection: [`seq`].
//! - Link-quality EWMA (alpha from half-life in frames): [`link`].
//! - walks/2 chain writer + verifier: [`walks`] (std).
//! - Desktop peer: [`peer`] (std).
//!
//! ## Frame (16 bytes fixed) — pinned spec
//!
//! ```text
//! byte  0    : magic 0x51 ('Q')
//! byte  1    : version 0x01
//! byte  2    : kind    0x00 TICK | 0x01 DELTA | 0x02 ALARM | 0x03 LINKMETA | 0x04 ACK
//! byte  3    : cell_id (u8 — the sending cell; portals map ids to addresses)
//! bytes 4-5  : seq     (u16 LE, wraps — gap detection = reliability observation)
//! bytes 6-9  : tick    (u32 LE — sender-local tick; no cross-clock claims)
//! bytes 10-13: value   (f32 LE, raw IEEE-754 bits)
//! bytes 14-15: CRC16-CCITT-FALSE over bytes 0..=13
//!              (poly 0x1021, init 0xFFFF, no reflection, xorout 0x0000;
//!               check("123456789") = 0x29B1)
//! ```
//!
//! All multi-byte integers are **little-endian** (ESP32/xtensa and the host
//! fleet are LE; no byte-swap on the metal end).
//!
//! ## Optional TLVs (after byte 16, only when the transport MTU allows)
//!
//! ```text
//! TLV := type u8 | len u8 | value (len bytes)
//!   0x01: len=2, value = reason u8 | considered-mask u8
//!         present ONLY when the sender had >= 2 live links (the one
//!         declared half of subtext — nothing else is ever declared)
//! ```
//!
//! Unknown TLV types are skipped by the decoder (forward compatibility).
//! Serial v0 firmware sends bare 16-byte frames, no TLVs.
//!
//! ## Deliberately absent
//!
//! Timestamps-in-µs (no cross-clock claims — latency is observed in tick
//! units and receiver-side inter-arrival only), sender-quality reports
//! (subtext is observed, not declared), routing headers (egocentric: no
//! global addressing in the frame), encryption (v0; ESP-Now pairing later).

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(unsafe_code)]

pub mod frame;
pub mod link;
pub mod seq;
pub mod tlv;

#[cfg(feature = "std")]
pub mod peer;
#[cfg(feature = "std")]
pub mod walks;

pub use frame::{Frame, FrameDecoder, Kind, FRAME_LEN, MAGIC, VERSION};
pub use link::LinkQualityEwma;
pub use seq::{SeqTracker, SeqVerdict};
