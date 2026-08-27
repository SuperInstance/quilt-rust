//! # journal.rs
//!
//! The crash-safe journal — the black-box recorder for the ledger.
//!
//! ## Role in the system
//!
//! `ledger.rs` is the append-only hash chain; this module is the thing
//! that makes it **survive**. A journal is a single append-only file
//! of length-prefixed, CRC32-checksummed frames. Every frame commits
//! to its predecessor through a SHA-256 chain (independent of, and in
//! addition to, the per-cell ledger seals inside the payloads), so
//! the file is verifiable end-to-end: every CRC, every frame link,
//! every ledger seal, every continuity edge.
//!
//! The design target is the vessel doctrine (`docs/VESSEL-FIT.md`
//! suggestion #2; `docs/JOURNAL.md` is the full spec): a fishing boat
//! 60 miles offshore loses power mid-write. On reboot the journal
//! either replays clean to the last complete frame or reports the
//! tear honestly — **no case produces a silent wrong state**. That
//! contract is pinned by the power-yank test
//! (`tests/journal_power_yank.rs`), which truncates a journal at
//! *every* byte offset and replays each prefix.
//!
//! ## Torn writes — the recovery contract
//!
//! A frame write interrupted by power loss leaves a partial frame at
//! end-of-file. Recovery detects it (the frame's length prefix
//! demands more bytes than exist) and treats that frame's write as
//! never having happened: the honest prefix is everything before it,
//! and [`recover_file`] truncates the file back to that boundary. A
//! partial frame at EOF is a *normal* power-loss outcome; corruption
//! *inside* a complete frame (CRC mismatch) or a break in the chain
//! linkage is **not** normal, is never truncated away, and is always
//! reported.
//!
//! ## fsync-before-ack
//!
//! [`JournalWriter`] with [`SyncPolicy::EveryFrame`] (the honest
//! default) calls `sync_all()` on the file before acknowledging a
//! frame: the ack is the promise "this frame is on the platter", the
//! only promise a black-box recorder is allowed to make.
//! [`SyncPolicy::Off`] skips the fsync — same bytes, weaker
//! durability — for tests and scratch journals.
//!
//! ## Depends on
//!
//! - `crate::ledger` — [`CellLedger`], [`LedgerEntry`], the inline
//!   SHA-256, and `canonical_json` (the payload preimage form).
//! - `std::fs` / `std::io` — and that is all. No tokio, no clock:
//!   the recorder's caller passes timestamps, the ledger's
//!   discipline, kept here. The format is deliberately trivial to
//!   port to the ESP32 tier (see `docs/JOURNAL.md` §7).
//!
//! ## Used by
//!
//! - `quilt journal <sheet> --out journal.bin` — the live recorder
//!   over `engine.subscribe_all`.
//! - `quilt replay <journal.bin>` / `quilt journal-verify` — the
//!   recovery and audit surfaces.
//! - Anything embedding quilt-core that wants the ledger durable.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ledger::{canonical_json, sha256, CellLedger, LedgerEntry};
use crate::types::CellId;

// ---------------------------------------------------------------------------
// CRC-32 (ISO-HDLC / zlib) — dependency-free, exactly specified
// ---------------------------------------------------------------------------

/// The reflected CRC-32 table (poly 0xEDB88320), computed at compile
/// time. Pinned here — like the ledger's SHA-256 — so any port
/// reproduces the checksums bit-for-bit.
const fn make_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = make_crc_table();

/// CRC-32/ISO-HDLC of `data` (init/xorout 0xFFFFFFFF, reflected) —
/// the same checksum zlib, gzip, and PNG use. Check value:
/// `crc32(b"123456789") == 0xCBF4_3926`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc = CRC_TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

// ---------------------------------------------------------------------------
// Format constants — pinned by docs/JOURNAL.md
// ---------------------------------------------------------------------------

/// File magic: `QUILTJNL`.
pub const MAGIC: [u8; 8] = *b"QUILTJNL";
/// Journal format version written by this module.
pub const FORMAT_VERSION: u16 = 1;
/// Header size in bytes: magic(8) + version(2) + flags(2) + crc(4).
pub const HEADER_LEN: usize = 16;
/// Fixed part of every frame body: frame_version(1) + entry_type(1)
/// + seq(8). The 32-byte chain head follows, then the payload.
pub const FRAME_BODY_FIXED: usize = 42;
/// Sanity ceiling for one frame body (guards against absurd length
/// prefixes in garbage bytes). Sheet sources are KBs; this is MBs.
pub const MAX_BODY: usize = 8 * 1024 * 1024;

/// Entry type: sheet metadata (must be frame seq 1, exactly once).
pub const ENTRY_SHEET_META: u8 = 1;
/// Entry type: a sealed `LedgerEntry` payload.
pub const ENTRY_LEDGER_ENTRY: u8 = 2;
/// Entry type: a checkpoint / heartbeat marker (skipped by replay,
/// still covered by the frame chain).
pub const ENTRY_CHECKPOINT: u8 = 3;

/// The frame chain's genesis message. Every journal's first frame
/// head commits to `sha256(this message)`, so the chain root is
/// deterministic and identity-independent.
pub const FRAME_GENESIS_MESSAGE: &[u8] = b"quilt-journal/frames/1";

/// The frame-chain genesis head. Pinned (hex) in `docs/JOURNAL.md`
/// §2: changing it rewrites every journal ever written.
pub fn frame_genesis() -> [u8; 32] {
    sha256::sha256(FRAME_GENESIS_MESSAGE)
}

/// The chained preimage of a frame: the body bytes the chain hash
/// covers — `frame_version | entry_type | seq | payload` (the stored
/// 32-byte head field itself is *not* part of the preimage; it is
/// the *output* over those bytes chained to the previous head).
fn chained_bytes(entry_type: u8, seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(10 + payload.len());
    v.push(1u8); // frame body version
    v.push(entry_type);
    v.extend_from_slice(&seq.to_le_bytes());
    v.extend_from_slice(payload);
    v
}

/// The next chain head: `sha256(prev_head || chained_bytes)`.
fn next_head(prev_head: &[u8; 32], chained: &[u8]) -> [u8; 32] {
    let mut preimage = Vec::with_capacity(32 + chained.len());
    preimage.extend_from_slice(prev_head);
    preimage.extend_from_slice(chained);
    sha256::sha256(&preimage)
}

// ---------------------------------------------------------------------------
// Payload types
// ---------------------------------------------------------------------------

/// Frame payload for [`ENTRY_SHEET_META`]: which sheet this journal
/// recorded, including the full YAML source, so the journal is
/// self-contained for replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SheetMetaFrame {
    /// The sheet id.
    pub id: String,
    /// The sheet format version string.
    pub version: String,
    /// The complete sheet source (YAML) as loaded at record time.
    pub source: String,
}

/// Frame payload for [`ENTRY_LEDGER_ENTRY`]: which cell's ledger the
/// sealed entry belongs to. The entry itself never names its cell
/// (cell identity lives on the ledger, and the ledger's seal
/// preimage is pinned across ports), so the journal — which
/// interleaves many cells' chains — wraps it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntryFrame {
    /// The cell whose ledger this entry belongs to.
    pub cell_id: CellId,
    /// The sealed entry.
    pub entry: LedgerEntry,
}

/// Frame payload for [`ENTRY_CHECKPOINT`]: a human-meaningful marker
/// (heartbeat, boot note). Replay skips checkpoints; the frame chain
/// still covers them, so they cannot be forged or removed silently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointFrame {
    /// Free-form note.
    pub note: String,
}

// ---------------------------------------------------------------------------
// Scan — raw frame extraction with tear/corruption detection
// ---------------------------------------------------------------------------

/// One raw frame as it sits in the file, with its location.
#[derive(Debug, Clone)]
pub struct RawFrame {
    /// Journal-global sequence number (1-based, contiguous).
    pub seq: u64,
    /// The entry type byte (see the `ENTRY_*` constants).
    pub entry_type: u8,
    /// The frame-chain head *after* this frame.
    pub head: [u8; 32],
    /// The payload bytes.
    pub payload: Vec<u8>,
    /// Byte offset of this frame's length prefix within the file.
    pub offset: usize,
    /// Total bytes the frame occupies (4 + body_len + 4).
    pub total_len: usize,
}

/// Outcome of scanning a byte stream into frames.
///
/// The scan is the torn-write detector: it never returns frames it
/// cannot fully justify, and it says exactly where and why it stopped.
#[derive(Debug, Clone, PartialEq)]
pub enum ScanOutcome {
    /// Every byte belongs to a complete, CRC-valid frame.
    Clean,
    /// The bytes are not a quilt journal at all.
    NotAJournal(String),
    /// The file ends inside the 16-byte header. A power-loss outcome
    /// for a journal that died at birth.
    TornHeader {
        /// How many header bytes are present (all matching the
        /// magic prefix — truncation preserves prefixes).
        available: usize,
    },
    /// The file ends inside a frame: a torn write at EOF. Everything
    /// before it is complete and CRC-valid.
    TornFrame {
        /// Index the partial frame would have had (0-based).
        index: usize,
        /// Byte offset where the partial frame starts.
        offset: usize,
        /// Trailing bytes that belong to the torn frame.
        torn_bytes: usize,
    },
    /// A complete-looking frame failed its CRC or carried a bogus
    /// structure. Not a normal power-loss outcome — bit rot or a
    /// foreign file. Reported, never truncated away.
    CorruptFrame {
        /// Index of the corrupt frame (0-based).
        index: usize,
        /// Byte offset where it starts.
        offset: usize,
        /// What failed.
        reason: String,
    },
}

/// Header bytes for a fresh journal.
pub fn header_bytes() -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0..8].copy_from_slice(&MAGIC);
    h[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    // flags (10..12) reserved, zero.
    let crc = crc32(&h[0..12]);
    h[12..16].copy_from_slice(&crc.to_le_bytes());
    h
}

/// Is `bytes` a prefix of `full`? (Truncation preserves prefixes.)
fn is_prefix(bytes: &[u8], full: &[u8]) -> bool {
    bytes.len() <= full.len() && &full[..bytes.len()] == bytes
}

/// Split a byte stream into raw frames, stopping at the first tear
/// or corruption and reporting it.
///
/// This layer checks: header integrity, frame completeness (length
/// prefix vs bytes available), frame CRC (over `body_len || body`),
/// and body structure. It does **not** check chain linkage or
/// payload semantics — that is [`journal_verify`] and
/// [`journal_replay`].
pub fn scan_frames(bytes: &[u8]) -> (Vec<RawFrame>, ScanOutcome) {
    if bytes.len() < HEADER_LEN {
        // Every truncation of a valid header preserves its prefix:
        // fewer than 8 bytes must match the magic prefix; 8+ must
        // match the magic exactly. Anything else is a foreign file.
        let head = &bytes[..bytes.len().min(MAGIC.len())];
        let magic_ok = is_prefix(head, &MAGIC);
        return if magic_ok {
            (
                Vec::new(),
                ScanOutcome::TornHeader {
                    available: bytes.len(),
                },
            )
        } else {
            (Vec::new(), ScanOutcome::NotAJournal("bad magic".into()))
        };
    }
    if bytes[..8] != MAGIC {
        return (Vec::new(), ScanOutcome::NotAJournal("bad magic".into()));
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version > FORMAT_VERSION {
        return (
            Vec::new(),
            ScanOutcome::NotAJournal(format!(
                "format version {version} > supported {FORMAT_VERSION}"
            )),
        );
    }
    let stored_crc = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    if stored_crc != crc32(&bytes[0..12]) {
        return (
            Vec::new(),
            ScanOutcome::NotAJournal("header CRC mismatch".into()),
        );
    }

    let mut frames = Vec::new();
    let mut pos = HEADER_LEN;
    while pos < bytes.len() {
        let index = frames.len();
        let remaining = bytes.len() - pos;
        if remaining < 4 {
            return (
                frames,
                ScanOutcome::TornFrame {
                    index,
                    offset: pos,
                    torn_bytes: remaining,
                },
            );
        }
        let body_len =
            u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                as usize;
        if !(FRAME_BODY_FIXED..=MAX_BODY).contains(&body_len) {
            return (
                frames,
                ScanOutcome::CorruptFrame {
                    index,
                    offset: pos,
                    reason: format!(
                        "frame body length {body_len} outside [{FRAME_BODY_FIXED}, {MAX_BODY}]"
                    ),
                },
            );
        }
        let total = 4 + body_len + 4;
        if remaining < total {
            return (
                frames,
                ScanOutcome::TornFrame {
                    index,
                    offset: pos,
                    torn_bytes: remaining,
                },
            );
        }
        let body = &bytes[pos + 4..pos + 4 + body_len];
        let crc_start = pos + 4 + body_len;
        let stored_crc = u32::from_le_bytes([
            bytes[crc_start],
            bytes[crc_start + 1],
            bytes[crc_start + 2],
            bytes[crc_start + 3],
        ]);
        let mut crc_input = Vec::with_capacity(4 + body_len);
        crc_input.extend_from_slice(&(body_len as u32).to_le_bytes());
        crc_input.extend_from_slice(body);
        if stored_crc != crc32(&crc_input) {
            return (
                frames,
                ScanOutcome::CorruptFrame {
                    index,
                    offset: pos,
                    reason: "frame CRC mismatch".into(),
                },
            );
        }
        let frame_version = body[0];
        if frame_version != 1 {
            return (
                frames,
                ScanOutcome::CorruptFrame {
                    index,
                    offset: pos,
                    reason: format!("unknown frame body version {frame_version}"),
                },
            );
        }
        let entry_type = body[1];
        let seq = u64::from_le_bytes([
            body[2], body[3], body[4], body[5], body[6], body[7], body[8], body[9],
        ]);
        let mut head = [0u8; 32];
        head.copy_from_slice(&body[10..42]);
        let payload = body[42..].to_vec();
        frames.push(RawFrame {
            seq,
            entry_type,
            head,
            payload,
            offset: pos,
            total_len: total,
        });
        pos += total;
    }
    (frames, ScanOutcome::Clean)
}

// ---------------------------------------------------------------------------
// Verify — chain linkage, sequencing, framing rules
// ---------------------------------------------------------------------------

/// Why a structurally intact journal still fails to verify.
#[derive(Debug, Clone, PartialEq)]
pub enum DivergenceKind {
    /// Frame 1 is not sheet metadata.
    MissingSheetMeta,
    /// Sheet metadata present but not at frame 1 (reserved: current
    /// framing rules make this unreachable, the check stays for
    /// forward formats).
    MisplacedSheetMeta,
    /// A second sheet-metadata frame appeared.
    DuplicateSheetMeta,
    /// Frame sequence numbers are not contiguous from 1.
    BadSeq {
        /// The sequence number expected at this position.
        expected: u64,
        /// The sequence number found.
        found: u64,
    },
    /// The stored chain head does not recompute from the previous
    /// head and this frame's chained bytes — reorder, splice, or
    /// edit.
    FrameChainMismatch,
    /// An entry type this reader does not know.
    UnknownEntryType(u8),
    /// A payload failed to parse as its claimed type.
    BadPayload(String),
    /// The ledger refused to restore an entry — one of its four
    /// gates (sequence, linkage, seal, continuity) failed inside the
    /// cell's own chain.
    LedgerReject {
        /// The cell whose ledger rejected the entry.
        cell: CellId,
    },
}

/// One divergence, fully located.
#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
    /// 0-based index of the offending frame.
    pub frame_index: usize,
    /// The frame's sequence number.
    pub seq: u64,
    /// What kind of divergence.
    pub kind: DivergenceKind,
    /// Human-readable detail. Printed, never swallowed.
    pub message: String,
}

/// Outcome of [`journal_verify`].
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyOutcome {
    /// The whole file verified: header, every CRC, every frame link,
    /// sequencing, and framing rules.
    Clean {
        /// Number of complete verified frames.
        frames: usize,
    },
    /// Not a quilt journal (bad magic / version / header CRC).
    NotAJournal(String),
    /// Truncated inside the header.
    TornHeader {
        /// Header bytes present.
        available: usize,
    },
    /// Truncated inside the last frame. The prefix (all complete
    /// frames) verified; the torn frame's write never happened.
    TornTail {
        /// Frames that verified before the tear.
        good_frames: usize,
        /// Trailing bytes belonging to the torn frame.
        torn_bytes: usize,
        /// Byte offset where the torn frame starts.
        torn_offset: usize,
    },
    /// A complete frame is internally corrupt (CRC, length sanity,
    /// body version). The prefix before it is good; the corruption
    /// is a hard stop.
    Corrupt {
        /// Index of the corrupt frame.
        index: usize,
        /// Verified frames before it.
        good_frames: usize,
        /// Why.
        reason: String,
    },
    /// The structure is intact but the content diverges (chain,
    /// sequence, framing rules). Reported honestly; replay stops at
    /// the divergence.
    Divergence {
        /// The divergence detail.
        divergence: Box<Divergence>,
        /// Verified frames before it.
        good_frames: usize,
    },
}

/// The result of [`journal_verify`] — structural verification of a
/// journal byte stream, without rebuilding ledgers.
#[derive(Debug, Clone)]
pub struct VerifyReport {
    /// The journal format version.
    pub format_version: u16,
    /// Every frame that verified, in order, with locations. Always
    /// the *verified prefix* — exactly the frames the outcome
    /// vouches for, no more.
    pub frames: Vec<RawFrame>,
    /// The outcome.
    pub outcome: VerifyOutcome,
}

/// Verify a journal byte stream: header, frame CRCs, chain linkage
/// (`head_k = sha256(head_{k-1} || chained_bytes_k)`), sequence
/// contiguity from 1, and the framing rules (sheet meta first and
/// exactly once; known entry types).
///
/// Payload *semantics* (ledger seals, continuity) are
/// [`journal_replay`]'s job. A torn tail does not mask a divergence
/// underneath it: the semantic pass runs on the good prefix too.
pub fn journal_verify(bytes: &[u8]) -> VerifyReport {
    let (frames, scan) = scan_frames(bytes);
    let format_version = if bytes.len() >= 10 {
        u16::from_le_bytes([bytes[8], bytes[9]])
    } else {
        FORMAT_VERSION
    };

    let finish = |frames: Vec<RawFrame>, outcome| VerifyReport {
        format_version,
        frames,
        outcome,
    };

    match scan {
        ScanOutcome::NotAJournal(why) => finish(Vec::new(), VerifyOutcome::NotAJournal(why)),
        ScanOutcome::TornHeader { available } => {
            finish(Vec::new(), VerifyOutcome::TornHeader { available })
        }
        ScanOutcome::CorruptFrame {
            index,
            offset: _,
            reason,
        } => finish(
            frames,
            VerifyOutcome::Corrupt {
                index,
                good_frames: index,
                reason,
            },
        ),
        ScanOutcome::TornFrame {
            index,
            offset,
            torn_bytes,
        } => {
            if let Some(outcome) = semantic_outcome(&frames) {
                return finish(frames, outcome);
            }
            finish(
                frames,
                VerifyOutcome::TornTail {
                    good_frames: index,
                    torn_bytes,
                    torn_offset: offset,
                },
            )
        }
        ScanOutcome::Clean => {
            if let Some(outcome) = semantic_outcome(&frames) {
                return finish(frames, outcome);
            }
            let n = frames.len();
            finish(frames, VerifyOutcome::Clean { frames: n })
        }
    }
}

/// The journal-level semantic rules over scanned frames. Returns the
/// divergent outcome on the first violation, else `None`.
fn semantic_outcome(frames: &[RawFrame]) -> Option<VerifyOutcome> {
    let mut expected_head = frame_genesis();
    for (index, frame) in frames.iter().enumerate() {
        let divergence = |kind, message| {
            Some(VerifyOutcome::Divergence {
                divergence: Box::new(Divergence {
                    frame_index: index,
                    seq: frame.seq,
                    kind,
                    message,
                }),
                good_frames: index,
            })
        };
        if frame.seq != (index as u64) + 1 {
            return divergence(
                DivergenceKind::BadSeq {
                    expected: index as u64 + 1,
                    found: frame.seq,
                },
                format!("frame {} carries seq {}", index + 1, frame.seq),
            );
        }
        let chained = chained_bytes(frame.entry_type, frame.seq, &frame.payload);
        if next_head(&expected_head, &chained) != frame.head {
            return divergence(
                DivergenceKind::FrameChainMismatch,
                format!(
                    "frame {} chain head does not recompute from its predecessor",
                    frame.seq
                ),
            );
        }
        expected_head = frame.head;
        let meta_rule_violated = match frame.entry_type {
            ENTRY_SHEET_META => index != 0,
            ENTRY_LEDGER_ENTRY | ENTRY_CHECKPOINT => index == 0,
            other => {
                return divergence(
                    DivergenceKind::UnknownEntryType(other),
                    format!("frame {} has unknown entry type {}", frame.seq, other),
                );
            }
        };
        if meta_rule_violated {
            let kind = if frame.entry_type == ENTRY_SHEET_META {
                if index == 0 {
                    DivergenceKind::MisplacedSheetMeta
                } else {
                    DivergenceKind::DuplicateSheetMeta
                }
            } else {
                DivergenceKind::MissingSheetMeta
            };
            let message = match kind {
                DivergenceKind::DuplicateSheetMeta => {
                    format!("duplicate sheet metadata at frame {}", frame.seq)
                }
                _ => "frame 1 must be sheet metadata".to_string(),
            };
            return divergence(kind, message);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Replay — rebuild the ledgers
// ---------------------------------------------------------------------------

/// The result of [`journal_replay`] — verification plus the rebuilt
/// per-cell ledgers and every divergence found.
///
/// The ledgers are always the *honest prefix*: everything that
/// verified and restored cleanly. `divergences` is empty iff the
/// whole journal replayed; non-empty means replay stopped at the
/// first divergence and the report says exactly where and why.
#[derive(Debug, Clone)]
pub struct ReplayReport {
    /// The structural verification report.
    pub verify: VerifyReport,
    /// Sheet metadata (when frame 1 parsed).
    pub sheet: Option<SheetMetaFrame>,
    /// Checkpoints encountered, in order (frame seq, checkpoint).
    pub checkpoints: Vec<(u64, CheckpointFrame)>,
    /// Rebuilt ledgers, keyed by cell id.
    pub ledgers: BTreeMap<CellId, CellLedger>,
    /// Payload-level divergences (ledger rejects, bad payloads).
    /// Empty iff fully clean.
    pub divergences: Vec<Divergence>,
    /// How many ledger entries were replayed (excluding metadata and
    /// checkpoints).
    pub replayed_entries: usize,
}

/// Replay a journal byte stream: verify structure and chain, then
/// restore every ledger entry into fresh per-cell ledgers.
///
/// Each `LedgerEntry` payload must pass [`CellLedger::restore_entry`]
/// — sequence, linkage, seal recomputation, and state continuity —
/// before it is accepted. The first failure stops the replay and is
/// reported in `divergences`; the rebuilt prefix is returned so the
/// caller can see exactly how much survived.
///
/// A torn tail is a *normal* outcome here (power loss): the prefix
/// replays and the tear sits in `verify.outcome`. It is not a
/// divergence.
pub fn journal_replay(bytes: &[u8]) -> ReplayReport {
    let verify = journal_verify(bytes);
    let mut sheet = None;
    let mut checkpoints = Vec::new();
    let mut ledgers: BTreeMap<CellId, CellLedger> = BTreeMap::new();
    let mut divergences = Vec::new();
    let mut replayed_entries = 0usize;

    // A structural divergence already found: nothing to replay.
    if let VerifyOutcome::Divergence { .. } = verify.outcome {
        return ReplayReport {
            verify,
            sheet,
            checkpoints,
            ledgers,
            divergences,
            replayed_entries,
        };
    }

    for (index, frame) in verify.frames.iter().enumerate() {
        let mut reject = |kind, message| {
            divergences.push(Divergence {
                frame_index: index,
                seq: frame.seq,
                kind,
                message,
            });
        };
        match frame.entry_type {
            ENTRY_SHEET_META => match serde_json::from_slice::<SheetMetaFrame>(&frame.payload) {
                Ok(meta) => sheet = Some(meta),
                Err(e) => {
                    reject(
                        DivergenceKind::BadPayload(format!("sheet metadata: {e}")),
                        format!("frame {} payload failed to parse: {e}", frame.seq),
                    );
                    break;
                }
            },
            ENTRY_CHECKPOINT => match serde_json::from_slice::<CheckpointFrame>(&frame.payload) {
                Ok(cp) => checkpoints.push((frame.seq, cp)),
                Err(e) => {
                    reject(
                        DivergenceKind::BadPayload(format!("checkpoint: {e}")),
                        format!("frame {} payload failed to parse: {e}", frame.seq),
                    );
                    break;
                }
            },
            ENTRY_LEDGER_ENTRY => {
                let entry_frame: LedgerEntryFrame = match serde_json::from_slice(&frame.payload) {
                    Ok(f) => f,
                    Err(e) => {
                        reject(
                            DivergenceKind::BadPayload(format!("ledger entry: {e}")),
                            format!("frame {} payload failed to parse: {e}", frame.seq),
                        );
                        break;
                    }
                };
                let cell = entry_frame.cell_id.clone();
                let ledger = ledgers
                    .entry(cell.clone())
                    .or_insert_with(|| CellLedger::new(cell.clone()));
                if let Err(e) = ledger.restore_entry(entry_frame.entry) {
                    reject(
                        DivergenceKind::LedgerReject { cell },
                        format!("frame {} rejected by cell ledger: {e}", frame.seq),
                    );
                    break;
                }
                replayed_entries += 1;
            }
            other => {
                // scan-level rules already reject unknown types; this
                // arm is defense in depth and stays honest.
                reject(
                    DivergenceKind::UnknownEntryType(other),
                    format!("frame {} has unknown entry type {other}", frame.seq),
                );
                break;
            }
        }
    }

    ReplayReport {
        verify,
        sheet,
        checkpoints,
        ledgers,
        divergences,
        replayed_entries,
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// When the writer fsyncs: before every ack (default), or never
/// (tests / scratch journals — same bytes, weaker durability).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPolicy {
    /// `sync_all()` before the ack is returned. The ack means "on
    /// the platter".
    EveryFrame,
    /// No fsync. Fast; the OS page cache decides durability. For
    /// tests and throwaway journals only.
    Off,
}

/// Acknowledgement for one appended frame.
#[derive(Debug, Clone)]
pub struct FrameAck {
    /// The frame's sequence number.
    pub seq: u64,
    /// Bytes appended (the whole frame).
    pub bytes: usize,
    /// The chain head after this frame (32 raw bytes).
    pub head: [u8; 32],
}

/// Cumulative writer statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriterStats {
    /// Frames appended this session.
    pub frames: u64,
    /// Bytes appended this session (frames only, not the header).
    pub bytes: u64,
    /// fsyncs actually performed this session.
    pub syncs: u64,
}

/// Errors from the journal writer and recovery paths.
#[derive(Debug)]
pub enum JournalError {
    /// Underlying I/O failure.
    Io(io::Error),
    /// The existing file is not a journal (bad magic / version /
    /// header CRC).
    NotAJournal(String),
    /// The existing file is corrupt or divergent; refusing to append
    /// to it.
    Unresumable {
        /// The outcome that blocked the resume.
        outcome: VerifyOutcome,
    },
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JournalError::Io(e) => write!(f, "journal i/o error: {e}"),
            JournalError::NotAJournal(why) => write!(f, "not a quilt journal: {why}"),
            JournalError::Unresumable { outcome } => {
                write!(f, "journal not safe to append to: {outcome:?}")
            }
        }
    }
}

impl std::error::Error for JournalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            JournalError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for JournalError {
    fn from(e: io::Error) -> Self {
        JournalError::Io(e)
    }
}

/// The append-only journal writer.
///
/// One instance owns one file. Frames are appended with a single
/// `write_all`, fsynced per the sync policy, and chained: each
/// frame's head commits to its predecessor's head over its own
/// chained bytes.
///
/// Resume semantics: [`JournalWriter::open_or_create`] verifies the
/// existing content first. A torn tail is recovered (truncated to
/// the last complete frame — the torn write never happened);
/// corruption or divergence is a hard refusal.
pub struct JournalWriter {
    file: File,
    path: PathBuf,
    next_seq: u64,
    head: [u8; 32],
    sync: SyncPolicy,
    stats: WriterStats,
}

impl JournalWriter {
    /// Create a fresh journal at `path`. Fails if the file already
    /// exists — journals are never silently overwritten.
    pub fn create(path: impl AsRef<Path>, sync: SyncPolicy) -> Result<Self, JournalError> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .read(true)
            .open(path.as_ref())?;
        file.write_all(&header_bytes())?;
        Ok(Self {
            file,
            path: path.as_ref().to_path_buf(),
            next_seq: 1,
            head: frame_genesis(),
            sync,
            stats: WriterStats::default(),
        })
    }

    /// Open an existing journal for appending, recovering a torn
    /// tail (truncate back to the last complete frame) and resuming
    /// the chain from the last verified head. Creates a fresh file
    /// (header included) if none exists.
    ///
    /// Refuses to open corrupt or divergent journals: appending to
    /// dishonest bytes would launder them, and the journal does not
    /// launder.
    pub fn open_or_create(path: impl AsRef<Path>, sync: SyncPolicy) -> Result<Self, JournalError> {
        let path = path.as_ref();
        if !path.exists() {
            return Self::create(path, sync);
        }
        let mut bytes = Vec::new();
        let mut file = OpenOptions::new().read(true).append(true).open(path)?;
        file.read_to_end(&mut bytes)?;
        // An empty file is treated as fresh: some filesystems
        // allocate the inode before the header write lands.
        if bytes.is_empty() {
            file.write_all(&header_bytes())?;
            return Ok(Self {
                file,
                path: path.to_path_buf(),
                next_seq: 1,
                head: frame_genesis(),
                sync,
                stats: WriterStats::default(),
            });
        }
        let report = journal_verify(&bytes);
        let next_seq = match &report.outcome {
            VerifyOutcome::Clean { .. } | VerifyOutcome::TornTail { .. } => {
                if let VerifyOutcome::TornTail {
                    good_frames,
                    torn_offset,
                    ..
                } = &report.outcome
                {
                    let kept = if *good_frames == 0 {
                        HEADER_LEN
                    } else {
                        *torn_offset
                    } as u64;
                    file.set_len(kept)?;
                }
                match report.frames.last() {
                    Some(last) => last.seq + 1,
                    None => 1,
                }
            }
            VerifyOutcome::NotAJournal(why) => {
                return Err(JournalError::NotAJournal(why.clone()));
            }
            other => {
                return Err(JournalError::Unresumable {
                    outcome: other.clone(),
                });
            }
        };
        let head = match report.frames.last() {
            Some(last) => last.head,
            None => frame_genesis(),
        };
        Ok(Self {
            file,
            path: path.to_path_buf(),
            next_seq,
            head,
            sync,
            stats: WriterStats::default(),
        })
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The current chain head (after the last appended frame).
    pub fn head(&self) -> [u8; 32] {
        self.head
    }

    /// The next sequence number to be assigned.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Statistics for this session.
    pub fn stats(&self) -> WriterStats {
        self.stats
    }

    /// Append one frame of the given entry type with `payload`
    /// pinned as-is. Returns the ack (seq, bytes, chain head) only
    /// after the sync policy is satisfied.
    pub fn append_frame(
        &mut self,
        entry_type: u8,
        payload: &[u8],
    ) -> Result<FrameAck, JournalError> {
        let seq = self.next_seq;
        let chained = chained_bytes(entry_type, seq, payload);
        let head = next_head(&self.head, &chained);

        let body_len = FRAME_BODY_FIXED + payload.len();
        let mut frame = Vec::with_capacity(4 + body_len + 4);
        frame.extend_from_slice(&(body_len as u32).to_le_bytes());
        frame.extend_from_slice(&chained[..10]); // version | type | seq
        frame.extend_from_slice(&head);
        frame.extend_from_slice(payload);
        let crc = crc32(&frame);
        frame.extend_from_slice(&crc.to_le_bytes());

        self.file.write_all(&frame)?;
        if self.sync == SyncPolicy::EveryFrame {
            self.file.sync_all()?;
            self.stats.syncs += 1;
        }
        self.next_seq += 1;
        self.head = head;
        self.stats.frames += 1;
        self.stats.bytes += frame.len() as u64;
        Ok(FrameAck {
            seq,
            bytes: frame.len(),
            head,
        })
    }

    /// Append the sheet-metadata frame (must be the first frame).
    pub fn append_sheet_meta(
        &mut self,
        id: &str,
        version: &str,
        source: &str,
    ) -> Result<FrameAck, JournalError> {
        let meta = SheetMetaFrame {
            id: id.to_string(),
            version: version.to_string(),
            source: source.to_string(),
        };
        let value = serde_json::to_value(&meta).expect("plain data serializes");
        let payload = canonical_json(&value).into_bytes();
        self.append_frame(ENTRY_SHEET_META, &payload)
    }

    /// Append one sealed ledger entry for `cell_id`.
    pub fn append_ledger_entry(
        &mut self,
        cell_id: &str,
        entry: &LedgerEntry,
    ) -> Result<FrameAck, JournalError> {
        let frame = LedgerEntryFrame {
            cell_id: cell_id.to_string(),
            entry: entry.clone(),
        };
        let value = serde_json::to_value(&frame).expect("plain data serializes");
        let payload = canonical_json(&value).into_bytes();
        self.append_frame(ENTRY_LEDGER_ENTRY, &payload)
    }

    /// Append a checkpoint marker (skipped by replay, covered by the
    /// chain — heartbeats and boot notes).
    pub fn append_checkpoint(&mut self, note: &str) -> Result<FrameAck, JournalError> {
        let cp = CheckpointFrame {
            note: note.to_string(),
        };
        let value = serde_json::to_value(&cp).expect("plain data serializes");
        let payload = canonical_json(&value).into_bytes();
        self.append_frame(ENTRY_CHECKPOINT, &payload)
    }
}

// ---------------------------------------------------------------------------
// Recovery — truncate a torn tail in place
// ---------------------------------------------------------------------------

/// The result of [`recover_file`].
#[derive(Debug, Clone)]
pub struct RecoveryReport {
    /// The verification that decided the recovery.
    pub verify: VerifyReport,
    /// Bytes kept (the verified prefix, header included).
    pub kept_bytes: u64,
    /// Trailing bytes truncated away (0 when the file was already
    /// clean).
    pub dropped_bytes: u64,
}

/// Recover a journal file in place: verify it, and if the tail is a
/// torn frame, truncate the file back to the last complete frame
/// boundary. That frame's write never happened — the honest rollback
/// power loss demands.
///
/// Clean files are left untouched. Corruption and divergence are
/// **not** recovered (nothing honest can be truncated away); the
/// outcome is returned for the caller to report.
pub fn recover_file(path: impl AsRef<Path>) -> Result<RecoveryReport, JournalError> {
    let path = path.as_ref();
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    let verify = journal_verify(&bytes);

    let (kept_bytes, dropped_bytes) = match &verify.outcome {
        VerifyOutcome::TornTail {
            good_frames,
            torn_offset,
            ..
        } => {
            let kept = if *good_frames == 0 {
                HEADER_LEN
            } else {
                *torn_offset
            } as u64;
            let dropped = bytes.len() as u64 - kept;
            let file = OpenOptions::new().write(true).open(path)?;
            file.set_len(kept)?;
            (kept, dropped)
        }
        _ => (bytes.len() as u64, 0),
    };

    Ok(RecoveryReport {
        verify,
        kept_bytes,
        dropped_bytes,
    })
}

// ---------------------------------------------------------------------------
// Recorder — the live black box over engine events
// ---------------------------------------------------------------------------

/// The live recorder: sheet metadata plus every engine event, sealed
/// into per-cell ledgers, framed to disk.
///
/// Built for `engine.subscribe_all()`: each subscription event
/// becomes a `LedgerEntry` in that cell's ledger (input = the value
/// the world delivered, output = the resulting cell state, delta =
/// the cell's edge, surprise = persistence-prior surprise) and one
/// journal frame. Timestamps are passed in by the caller — the
/// ledger's discipline, kept here.
///
/// Only *external* mutations (set/push) flow through subscriptions,
/// so the journal records exactly the world-facing inputs; derived
/// cells recompute deterministically from the sheet plus those
/// inputs. That is the black-box contract.
pub struct JournalRecorder {
    writer: JournalWriter,
    ledgers: BTreeMap<CellId, CellLedger>,
}

impl JournalRecorder {
    /// Start a recorder: writes the sheet-metadata frame first.
    pub fn start(
        writer: JournalWriter,
        sheet_id: &str,
        sheet_version: &str,
        sheet_source: &str,
    ) -> Result<Self, JournalError> {
        let mut writer = writer;
        writer.append_sheet_meta(sheet_id, sheet_version, sheet_source)?;
        Ok(Self {
            writer,
            ledgers: BTreeMap::new(),
        })
    }

    /// Record one engine event: `cell_id` changed to `value` at
    /// `ts` (millis since epoch). Seals the entry into the cell's
    /// ledger and appends the frame. Returns the frame ack.
    pub fn record_event(
        &mut self,
        cell_id: &str,
        value: &Value,
        ts: u64,
    ) -> Result<FrameAck, JournalError> {
        let ledger = self
            .ledgers
            .entry(cell_id.to_string())
            .or_insert_with(|| CellLedger::new(cell_id));
        let entry = ledger.record(value.clone(), value.clone(), ts);
        self.writer.append_ledger_entry(cell_id, &entry)
    }

    /// All ledgers built so far (for summaries and final heads).
    pub fn ledgers(&self) -> &BTreeMap<CellId, CellLedger> {
        &self.ledgers
    }

    /// The underlying writer (for checkpoints and stats).
    pub fn writer_mut(&mut self) -> &mut JournalWriter {
        &mut self.writer
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A temp file that cleans up after itself.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(tag: &str) -> Self {
            static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Self(std::env::temp_dir().join(format!(
                "quilt-journal-{}-{}-{}.bin",
                tag,
                std::process::id(),
                n
            )))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    // -- CRC-32 correctness -------------------------------------------------

    #[test]
    fn crc32_matches_known_vectors() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b"abc"), 0x3524_41C2);
    }

    #[test]
    fn frame_genesis_is_pinned() {
        // Pinned in docs/JOURNAL.md §2. If this ever changes, every
        // journal ever written changes — it must not.
        assert_eq!(
            crate::ledger::sha256::hex(FRAME_GENESIS_MESSAGE),
            "4a642d0792703e5f7a352897eada858756652c6aac74078eeca62cd56a7f6d4e"
        );
    }

    // -- Header --------------------------------------------------------------

    #[test]
    fn header_is_magic_version_flags_and_crc() {
        let h = header_bytes();
        assert_eq!(&h[0..8], b"QUILTJNL");
        assert_eq!(u16::from_le_bytes([h[8], h[9]]), FORMAT_VERSION);
        assert_eq!(u16::from_le_bytes([h[10], h[11]]), 0); // flags reserved
        assert_eq!(
            u32::from_le_bytes([h[12], h[13], h[14], h[15]]),
            crc32(&h[0..12])
        );
    }

    #[test]
    fn zero_partial_and_foreign_headers() {
        assert_eq!(
            journal_verify(b"").outcome,
            VerifyOutcome::TornHeader { available: 0 }
        );
        assert_eq!(
            journal_verify(b"QUILT").outcome,
            VerifyOutcome::TornHeader { available: 5 }
        );
        assert_eq!(
            journal_verify(b"QUILTJNL\x01\x00\x00\x00\x99\x99\x99\x99X").outcome,
            VerifyOutcome::NotAJournal("header CRC mismatch".into())
        );
        assert!(matches!(
            journal_verify(b"NOTAJRNL and some more").outcome,
            VerifyOutcome::NotAJournal(_)
        ));
        // Header only: a valid empty journal.
        assert_eq!(
            journal_verify(&header_bytes()).outcome,
            VerifyOutcome::Clean { frames: 0 }
        );
    }

    // -- Round trip ----------------------------------------------------------

    #[test]
    fn round_trips_meta_entries_and_checkpoints() {
        let tmp = TempFile::new("roundtrip");
        let mut writer = JournalWriter::create(tmp.path(), SyncPolicy::Off).unwrap();
        writer
            .append_sheet_meta("boat", "1", "id: boat\nversion: \"1\"\n")
            .unwrap();

        let mut bilge = CellLedger::new("bilge.level");
        let e1 = bilge.record(json!(40.0), json!(40.0), 1_000);
        writer.append_checkpoint("boot").unwrap();
        let mut pump = CellLedger::new("pump.relay");
        let e2 = pump.record(json!(true), json!(true), 1_500);
        writer.append_ledger_entry("bilge.level", &e1).unwrap();
        writer.append_ledger_entry("pump.relay", &e2).unwrap();
        drop(writer);

        let bytes = std::fs::read(tmp.path()).unwrap();
        let report = journal_replay(&bytes);
        assert!(report.divergences.is_empty(), "{:?}", report.divergences);
        assert_eq!(report.verify.outcome, VerifyOutcome::Clean { frames: 4 });
        assert_eq!(report.sheet.as_ref().unwrap().id, "boat");
        assert_eq!(report.checkpoints.len(), 1);
        assert_eq!(report.checkpoints[0].1.note, "boot");
        assert_eq!(report.replayed_entries, 2);

        let rebuilt = report.ledgers.get("bilge.level").unwrap();
        assert_eq!(rebuilt.chain_hash(), bilge.chain_hash());
        assert_eq!(rebuilt.state(), &json!(40.0));
        assert!(rebuilt.reconcile().balanced);
        let rebuilt2 = report.ledgers.get("pump.relay").unwrap();
        assert_eq!(rebuilt2.chain_hash(), pump.chain_hash());
    }

    // -- Torn writes -----------------------------------------------------------

    #[test]
    fn torn_tail_is_detected_and_prefix_replays() {
        let tmp = TempFile::new("torn");
        let mut writer = JournalWriter::create(tmp.path(), SyncPolicy::Off).unwrap();
        writer.append_sheet_meta("s", "1", "src").unwrap();
        let mut ledger = CellLedger::new("c");
        let e = ledger.record(json!(1), json!(1), 10);
        writer.append_ledger_entry("c", &e).unwrap();
        drop(writer);

        let full = std::fs::read(tmp.path()).unwrap();
        assert_eq!(
            journal_verify(&full).outcome,
            VerifyOutcome::Clean { frames: 2 }
        );

        // Cut one byte inside the last frame.
        let cut = &full[..full.len() - 1];
        match journal_verify(cut).outcome {
            VerifyOutcome::TornTail {
                good_frames,
                torn_bytes,
                ..
            } => {
                assert_eq!(good_frames, 1); // meta only
                assert!(torn_bytes > 0);
            }
            other => panic!("expected torn tail, got {other:?}"),
        }
        let replay = journal_replay(cut);
        assert!(replay.divergences.is_empty());
        assert_eq!(replay.replayed_entries, 0);
    }

    // -- Corruption --------------------------------------------------------------

    #[test]
    fn bit_flip_inside_a_frame_is_corruption_not_tear() {
        let tmp = TempFile::new("corrupt");
        let mut writer = JournalWriter::create(tmp.path(), SyncPolicy::Off).unwrap();
        writer.append_sheet_meta("s", "1", "src").unwrap();
        let mut ledger = CellLedger::new("c");
        let e = ledger.record(json!(5), json!(5), 10);
        writer.append_ledger_entry("c", &e).unwrap();
        drop(writer);
        let mut bytes = std::fs::read(tmp.path()).unwrap();

        // Flip a payload byte in the second frame (well inside it).
        let (frames, _) = scan_frames(&bytes);
        let frame2_start = frames[1].offset;
        bytes[frame2_start + 4 + 60] ^= 0xFF;

        let report = journal_verify(&bytes);
        match report.outcome {
            VerifyOutcome::Corrupt {
                index, good_frames, ..
            } => {
                assert_eq!(index, 1);
                assert_eq!(good_frames, 1);
            }
            other => panic!("expected corrupt, got {other:?}"),
        }
        // Replay keeps the honest prefix; nothing silent.
        let replay = journal_replay(&bytes);
        assert!(replay.divergences.is_empty());
        assert_eq!(replay.replayed_entries, 0);
    }

    #[test]
    fn reordered_frames_break_the_chain() {
        let tmp = TempFile::new("reorder");
        let mut writer = JournalWriter::create(tmp.path(), SyncPolicy::Off).unwrap();
        writer.append_sheet_meta("s", "1", "src").unwrap();
        let mut ledger = CellLedger::with_genesis("c", json!(0), 0);
        let e1 = ledger.record(json!(1), json!(1), 10);
        let e2 = ledger.record(json!(2), json!(2), 20);
        writer.append_ledger_entry("bilge.level", &e1).unwrap();
        writer.append_ledger_entry("pump.relay", &e2).unwrap();
        drop(writer);
        let bytes = std::fs::read(tmp.path()).unwrap();

        // Rebuild the file with the two entry frames swapped.
        let (frames, _) = scan_frames(&bytes);
        assert_eq!(frames.len(), 3);
        let f1 = bytes[frames[1].offset..frames[1].offset + frames[1].total_len].to_vec();
        let f2 = bytes[frames[2].offset..frames[2].offset + frames[2].total_len].to_vec();
        let mut swapped = bytes[..frames[1].offset].to_vec();
        swapped.extend_from_slice(&f2);
        swapped.extend_from_slice(&f1);

        let report = journal_verify(&swapped);
        match report.outcome {
            VerifyOutcome::Divergence {
                divergence,
                good_frames,
            } => {
                assert_eq!(
                    divergence.kind,
                    DivergenceKind::BadSeq {
                        expected: 2,
                        found: 3
                    }
                );
                assert_eq!(good_frames, 1);
            }
            other => panic!("expected divergence, got {other:?}"),
        }
    }

    #[test]
    fn spliced_frames_fail_the_chain_linkage() {
        // Two journals of identical shape but different sheet meta:
        // an entry frame from B grafted onto A carries a valid CRC
        // and the right seq, but its head cannot recompute over A's
        // chain — the frame chain does its job.
        let tmp_a = TempFile::new("splice-a");
        let tmp_b = TempFile::new("splice-b");
        let build = |tmp: &TempFile, source: &str| {
            let mut w = JournalWriter::create(tmp.path(), SyncPolicy::Off).unwrap();
            w.append_sheet_meta("s", "1", source).unwrap();
            let mut ledger = CellLedger::new("c");
            let e = ledger.record(json!(7), json!(7), 10);
            w.append_ledger_entry("c", &e).unwrap();
            drop(w);
            std::fs::read(tmp.path()).unwrap()
        };
        let a = build(&tmp_a, "sheet-a-source");
        let b = build(&tmp_b, "sheet-b-source");

        let (frames_a, _) = scan_frames(&a);
        let (frames_b, _) = scan_frames(&b);
        let mut spliced = a[..frames_a[1].offset].to_vec();
        let b_entry = &b[frames_b[1].offset..frames_b[1].offset + frames_b[1].total_len];
        spliced.extend_from_slice(b_entry);

        let report = journal_verify(&spliced);
        match report.outcome {
            VerifyOutcome::Divergence {
                divergence,
                good_frames,
            } => {
                assert_eq!(divergence.kind, DivergenceKind::FrameChainMismatch);
                assert_eq!(divergence.frame_index, 1);
                assert_eq!(good_frames, 1);
            }
            other => panic!("expected chain mismatch, got {other:?}"),
        }
    }

    #[test]
    fn missing_sheet_meta_is_a_divergence() {
        let tmp = TempFile::new("nometa");
        let mut writer = JournalWriter::create(tmp.path(), SyncPolicy::Off).unwrap();
        // Frame 1 is an entry, not meta. (Empty payload: the scan
        // accepts it structurally; the semantic pass must reject it.)
        writer.append_frame(ENTRY_LEDGER_ENTRY, &[]).unwrap();
        drop(writer);
        let bytes = std::fs::read(tmp.path()).unwrap();
        let report = journal_verify(&bytes);
        match report.outcome {
            VerifyOutcome::Divergence { divergence, .. } => {
                assert_eq!(divergence.kind, DivergenceKind::MissingSheetMeta);
            }
            other => panic!("expected divergence, got {other:?}"),
        }
    }

    // -- Recovery & resume -----------------------------------------------------

    #[test]
    fn recover_truncates_a_torn_tail_in_place() {
        let tmp = TempFile::new("recover");
        let mut writer = JournalWriter::create(tmp.path(), SyncPolicy::Off).unwrap();
        writer.append_sheet_meta("s", "1", "src").unwrap();
        let mut ledger = CellLedger::new("c");
        let e = ledger.record(json!(1), json!(1), 10);
        writer.append_ledger_entry("c", &e).unwrap();
        drop(writer);
        let full = std::fs::read(tmp.path()).unwrap();

        // Tear: cut inside the last frame.
        let tear_at = full.len() - 10;
        std::fs::write(tmp.path(), &full[..tear_at]).unwrap();

        let report = recover_file(tmp.path()).unwrap();
        assert_eq!(report.dropped_bytes, (tear_at as u64 - report.kept_bytes));
        // The recovered file is exactly the good prefix and replays clean.
        let recovered = std::fs::read(tmp.path()).unwrap();
        assert_eq!(recovered.len(), report.kept_bytes as usize);
        assert_eq!(
            journal_verify(&recovered).outcome,
            VerifyOutcome::Clean { frames: 1 }
        );

        // Recovering a clean file is a no-op.
        let report2 = recover_file(tmp.path()).unwrap();
        assert_eq!(report2.dropped_bytes, 0);
        assert_eq!(report2.kept_bytes, recovered.len() as u64);
    }

    #[test]
    fn writer_resumes_a_recovered_journal_and_chain_stays_contiguous() {
        // Restart semantics: a process that comes back after power
        // loss resumes the WRITER (torn frame dropped, seq continues)
        // and rebuilds its LEDGERS from the recovered journal — the
        // journal is the truth, the old in-memory ledger is not.
        let tmp = TempFile::new("resume");
        let mut writer = JournalWriter::create(tmp.path(), SyncPolicy::Off).unwrap();
        writer.append_sheet_meta("s", "1", "src").unwrap();
        let mut live = CellLedger::new("c");
        let e1 = live.record(json!(1), json!(1), 10);
        writer.append_ledger_entry("c", &e1).unwrap();
        drop(writer);
        let full = std::fs::read(tmp.path()).unwrap();
        std::fs::write(tmp.path(), &full[..full.len() - 5]).unwrap(); // tear e1 away

        // The restarted process: recover + resume the writer.
        let mut w2 = JournalWriter::open_or_create(tmp.path(), SyncPolicy::Off).unwrap();
        assert_eq!(w2.next_seq(), 2); // the torn frame 2 (e1) never happened
                                      // ...and rebuild the ledgers from the journal, not from memory.
        let report = journal_replay(&std::fs::read(tmp.path()).unwrap());
        assert!(report.divergences.is_empty());
        let mut live = report
            .ledgers
            .get("c")
            .cloned()
            .unwrap_or_else(|| CellLedger::new("c")); // e1 is gone: fresh chain
        let e2 = live.record(json!(2), json!(2), 20);
        let ack = w2.append_ledger_entry("c", &e2).unwrap();
        assert_eq!(ack.seq, 2);
        drop(w2);

        let bytes = std::fs::read(tmp.path()).unwrap();
        let replay = journal_replay(&bytes);
        assert!(replay.divergences.is_empty(), "{:?}", replay.divergences);
        assert_eq!(replay.verify.outcome, VerifyOutcome::Clean { frames: 2 });
        let rebuilt = replay.ledgers.get("c").unwrap();
        assert_eq!(rebuilt.chain_hash(), live.chain_hash());
        assert_eq!(rebuilt.len(), 1);
        assert_eq!(rebuilt.state(), &json!(2));
    }

    #[test]
    fn writer_refuses_to_resume_foreign_or_corrupt_files() {
        let tmp = TempFile::new("refuse");
        std::fs::write(tmp.path(), b"not-a-journal-at-all-just-bytes").unwrap();
        assert!(matches!(
            JournalWriter::open_or_create(tmp.path(), SyncPolicy::Off),
            Err(JournalError::NotAJournal(_))
        ));

        // A corrupt (bit-flipped) journal is equally unresumable.
        let tmp2 = TempFile::new("refuse2");
        let mut w = JournalWriter::create(tmp2.path(), SyncPolicy::Off).unwrap();
        w.append_sheet_meta("s", "1", "src").unwrap();
        drop(w);
        let mut bytes = std::fs::read(tmp2.path()).unwrap();
        bytes[20] ^= 0xFF; // inside the meta frame
        std::fs::write(tmp2.path(), &bytes).unwrap();
        assert!(matches!(
            JournalWriter::open_or_create(tmp2.path(), SyncPolicy::Off),
            Err(JournalError::Unresumable { .. })
        ));
    }

    // -- Sync policy -------------------------------------------------------------

    #[test]
    fn every_frame_policy_fsyncs_before_ack_and_off_does_not() {
        let on = TempFile::new("sync-on");
        let mut w = JournalWriter::create(on.path(), SyncPolicy::EveryFrame).unwrap();
        w.append_sheet_meta("s", "1", "src").unwrap();
        let mut ledger = CellLedger::new("c");
        let e = ledger.record(json!(1), json!(1), 10);
        w.append_ledger_entry("c", &e).unwrap();
        assert_eq!(w.stats().syncs, 2); // one per frame, before each ack
        drop(w);

        let off = TempFile::new("sync-off");
        let mut w2 = JournalWriter::create(off.path(), SyncPolicy::Off).unwrap();
        w2.append_sheet_meta("s", "1", "src").unwrap();
        let mut ledger2 = CellLedger::new("c");
        let e2 = ledger2.record(json!(1), json!(1), 10);
        w2.append_ledger_entry("c", &e2).unwrap();
        assert_eq!(w2.stats().syncs, 0);
        drop(w2);

        // The policy changes durability, not content: identical bytes.
        assert_eq!(
            std::fs::read(on.path()).unwrap(),
            std::fs::read(off.path()).unwrap()
        );
    }

    // -- Recorder ------------------------------------------------------------------

    #[test]
    fn recorder_seals_events_into_per_cell_chains() {
        let tmp = TempFile::new("recorder");
        let w = JournalWriter::create(tmp.path(), SyncPolicy::Off).unwrap();
        let mut rec = JournalRecorder::start(w, "sheet", "1", "id: sheet").unwrap();
        let ack1 = rec
            .record_event("bilge.level", &json!(41.0), 1_000)
            .unwrap();
        let ack2 = rec
            .record_event("bilge.level", &json!(42.0), 2_000)
            .unwrap();
        let ack3 = rec.record_event("pump.relay", &json!(true), 2_100).unwrap();
        assert_eq!(ack1.seq, 2); // meta is seq 1
        assert_eq!(ack2.seq, 3);
        assert_eq!(ack3.seq, 4);
        rec.writer_mut().append_checkpoint("done").unwrap();
        let live_heads: Vec<_> = rec
            .ledgers()
            .iter()
            .map(|(id, l)| (id.clone(), l.chain_hash()))
            .collect();
        drop(rec);

        let bytes = std::fs::read(tmp.path()).unwrap();
        let report = journal_replay(&bytes);
        assert!(report.divergences.is_empty(), "{:?}", report.divergences);
        assert_eq!(report.replayed_entries, 3);
        assert_eq!(report.ledgers.len(), 2);

        // Rebuilt chains match the live recorder's chains exactly.
        for (id, live_head) in live_heads {
            assert_eq!(report.ledgers[&id].chain_hash(), live_head);
        }
        let bilge = report.ledgers.get("bilge.level").unwrap();
        assert_eq!(bilge.len(), 2);
        assert_eq!(bilge.state(), &json!(42.0));
        assert!(bilge.reconcile().balanced);
    }

    #[test]
    fn replay_is_deterministic() {
        let tmp = TempFile::new("determinism");
        let w = JournalWriter::create(tmp.path(), SyncPolicy::Off).unwrap();
        let mut rec = JournalRecorder::start(w, "s", "1", "src").unwrap();
        for i in 0..5u64 {
            rec.record_event("cell.a", &json!(i as f64), 1_000 + i)
                .unwrap();
        }
        drop(rec);

        let bytes = std::fs::read(tmp.path()).unwrap();
        let r1 = journal_replay(&bytes);
        let r2 = journal_replay(&bytes);
        // Same rebuilt chains, twice.
        for (id, l1) in &r1.ledgers {
            assert_eq!(l1.chain_hash(), r2.ledgers[id].chain_hash());
        }
        assert_eq!(r1.replayed_entries, r2.replayed_entries);
    }
}
