//! Power-yank simulation for the quilt journal.
//!
//! The recovery contract (`docs/JOURNAL.md` §3): a journal torn at
//! *any* byte offset by power loss must either
//!
//! 1. replay clean to the last complete frame (the torn frame's
//!    write never happened), or
//! 2. report the tear (or a header tear) honestly —
//!
//! and **never** produce a silent wrong state. This test truncates a
//! journal at every possible byte offset and replays each prefix,
//! checking every outcome against ground truth computed from the
//! known frame boundaries, including the exact per-cell chain heads
//! after each prefix. Then it exercises on-disk recovery: truncating
//! at torn offsets and calling `recover_file` must yield exactly the
//! good prefix, which then replays 100% clean.
//!
//! A corruption matrix runs alongside: flipping any single byte in
//! any frame must be reported (corrupt or divergent), never silent.

use std::collections::BTreeMap;

use quilt_core::journal::{journal_replay, recover_file, SyncPolicy, VerifyOutcome};
use quilt_core::{CellLedger, JournalRecorder, JournalWriter};
use serde_json::json;

/// Build a journal with varied frame sizes: meta + checkpoint + 2
/// cells × 3 events each = 8 frames.
fn build_journal(tag: &str) -> (Vec<u8>, Vec<CellLedger>, Vec<usize>) {
    let mut ledgers: Vec<CellLedger> = Vec::new();
    let mut buffer: Vec<u8> = Vec::new();
    let mut boundaries: Vec<usize> = Vec::new();

    // Build through a real writer so the bytes are exactly what
    // production produces.
    let dir = std::env::temp_dir();
    let path = dir.join(format!("quilt-power-yank-{tag}-{}.bin", std::process::id()));
    let writer = JournalWriter::create(&path, SyncPolicy::Off).expect("create writer");
    let mut rec = JournalRecorder::start(writer, "power-yank", "1", "id: power-yank\n").unwrap();
    rec.writer_mut().append_checkpoint("boot").unwrap();

    let events: [(&str, f64, u64); 6] = [
        ("bilge.level", 41.0, 1_000),
        ("engine.rpm", 1800.0, 1_100),
        ("bilge.level", 42.5, 1_200),
        ("engine.rpm", 1950.0, 1_300),
        ("bilge.level", 43.0, 1_400),
        ("engine.rpm", 2100.0, 1_500),
    ];
    for (cell, value, ts) in events {
        rec.record_event(cell, &json!(value), ts).unwrap();
    }
    let live: BTreeMap<String, CellLedger> = rec
        .ledgers()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    drop(rec);

    let bytes = std::fs::read(&path).expect("read journal");
    let _ = std::fs::remove_file(&path);

    // Ground truth: frame boundaries from a scan of the intact file.
    let (frames, outcome) = quilt_core::journal::scan_frames(&bytes);
    assert_eq!(outcome, quilt_core::journal::ScanOutcome::Clean);
    assert_eq!(frames.len(), 8); // meta + checkpoint + 6 events
    let mut offset = quilt_core::journal::HEADER_LEN;
    for f in &frames {
        offset += f.total_len;
        boundaries.push(offset);
    }
    assert_eq!(offset, bytes.len());

    buffer.extend_from_slice(&bytes);
    ledgers.extend(live.values().cloned());
    (buffer, ledgers, boundaries)
}

/// The expected state of the recorder's ledgers after the first `n`
/// frames (frame 0 = meta, frame 1 = checkpoint, frames 2..8 =
/// events in order).
fn expected_ledgers_after_frames(n: usize, live: &[CellLedger]) -> BTreeMap<String, CellLedger> {
    // Event i is frame i + 2; events interleave two cells.
    let events: [(&str, f64, u64); 6] = [
        ("bilge.level", 41.0, 1_000),
        ("engine.rpm", 1800.0, 1_100),
        ("bilge.level", 42.5, 1_200),
        ("engine.rpm", 1950.0, 1_300),
        ("bilge.level", 43.0, 1_400),
        ("engine.rpm", 2100.0, 1_500),
    ];
    let events_applied = n.saturating_sub(2).min(6);
    let mut expected: BTreeMap<String, CellLedger> = BTreeMap::new();
    for (cell, value, ts) in events.iter().take(events_applied) {
        let ledger = expected
            .entry((*cell).to_string())
            .or_insert_with(|| CellLedger::new(*cell));
        ledger.record(json!(value), json!(value), *ts);
    }
    // Sanity when everything applied: chains must match the live ones.
    if events_applied == 6 {
        for l in live {
            assert_eq!(
                expected.get(l.cell_id()).unwrap().chain_hash(),
                l.chain_hash(),
                "ground-truth rebuild diverged from live ledger"
            );
        }
    }
    expected
}

#[test]
fn power_yank_truncation_matrix() {
    let (bytes, live, boundaries) = build_journal("matrix");
    let header_len = quilt_core::journal::HEADER_LEN;

    let mut stats = MatrixStats::default();

    for offset in 0..=bytes.len() {
        let torn = &bytes[..offset];
        let report = journal_replay(torn);
        stats.offsets += 1;

        // Classify the expected outcome from ground truth.
        let good_frames = boundaries.iter().filter(|b| **b <= offset).count();
        // A byte offset that lands exactly at the end of the header
        // or at the end of any frame is a clean boundary.
        let at_boundary = boundaries.contains(&offset) || offset == header_len;

        if offset < header_len {
            stats.torn_header += 1;
            assert_eq!(
                report.verify.outcome,
                VerifyOutcome::TornHeader { available: offset },
                "offset {offset}: header tear must be reported exactly"
            );
            assert!(report.ledgers.is_empty());
            continue;
        }

        // The prefix must never diverge, whatever happened to the tail.
        assert!(
            report.divergences.is_empty(),
            "offset {offset}: prefix diverged silently: {:?}",
            report.divergences
        );

        if at_boundary {
            // Exact frame boundary (header-only counts too): clean
            // replay of exactly the contained frames.
            stats.clean += 1;
            assert_eq!(
                report.verify.outcome,
                VerifyOutcome::Clean {
                    frames: good_frames
                },
                "offset {offset} (a frame boundary): expected clean with {good_frames} frames, got {:?}",
                report.verify.outcome
            );
        } else {
            // Inside frame good_frames (0-based): torn tail, prefix intact.
            stats.torn += 1;
            match report.verify.outcome {
                VerifyOutcome::TornTail {
                    good_frames: g,
                    torn_bytes,
                    torn_offset,
                } => {
                    assert_eq!(g, good_frames, "offset {offset}: good frame count");
                    assert!(torn_bytes > 0);
                    assert_eq!(
                        torn_offset,
                        if good_frames == 0 {
                            header_len
                        } else {
                            boundaries[good_frames - 1]
                        },
                        "offset {offset}: tear offset must be the boundary after the last good frame"
                    );
                }
                ref other => panic!(
                    "offset {offset}: expected torn tail, got {other:?} (silent wrong state!)"
                ),
            }
        }

        // THE invariant: rebuilt ledgers equal the ground-truth
        // ledgers for this prefix — never a wrong state, silently
        // or otherwise.
        let expected = expected_ledgers_after_frames(good_frames, &live);
        assert_eq!(
            report.ledgers.len(),
            expected.len(),
            "offset {offset}: cell set mismatch"
        );
        for (id, exp) in &expected {
            let got = report
                .ledgers
                .get(id)
                .unwrap_or_else(|| panic!("offset {offset}: cell {id} missing"));
            assert_eq!(
                got.chain_hash(),
                exp.chain_hash(),
                "offset {offset}: cell {id} chain head mismatch — silent wrong state"
            );
            assert_eq!(got.state(), exp.state(), "offset {offset}: cell {id} state");
        }
    }

    println!("power-yank matrix: {stats}");
    // Sanity on coverage: every offset was classified, and there is
    // at least one of each class.
    assert_eq!(stats.offsets, bytes.len() + 1);
    assert!(stats.clean >= 9); // header-only + 8 frame boundaries
    assert!(stats.torn > 0);
    assert_eq!(stats.torn_header, header_len);
}

#[derive(Default)]
struct MatrixStats {
    offsets: usize,
    clean: usize,
    torn: usize,
    torn_header: usize,
}

impl std::fmt::Display for MatrixStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} offsets tested -> {} clean-boundary, {} torn-tail (reported), {} torn-header (reported)",
            self.offsets, self.clean, self.torn, self.torn_header
        )
    }
}

#[test]
fn power_yank_recovery_truncates_to_exactly_the_good_prefix() {
    let (bytes, _live, boundaries) = build_journal("recover");
    let header_len = quilt_core::journal::HEADER_LEN;
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "quilt-power-yank-rec-{}-{}.bin",
        std::process::id(),
        line!()
    ));

    // Every non-boundary offset beyond the header (a torn write), at
    // a stride that keeps CI fast: every 3rd byte.
    let mut recovered = 0usize;
    for offset in (header_len + 1..bytes.len()).step_by(3) {
        if boundaries.contains(&offset) {
            continue;
        }
        std::fs::write(&path, &bytes[..offset]).unwrap();
        let report = recover_file(&path).unwrap();
        let good_frames = boundaries.iter().filter(|b| **b <= offset).count();
        let expected_len = if good_frames == 0 {
            header_len
        } else {
            boundaries[good_frames - 1]
        } as u64;
        assert_eq!(report.kept_bytes, expected_len, "offset {offset}");
        assert!(report.dropped_bytes > 0);
        // The recovered file replays 100% clean with the full prefix.
        let recovered_bytes = std::fs::read(&path).unwrap();
        let replay = journal_replay(&recovered_bytes);
        assert!(replay.divergences.is_empty());
        assert_eq!(
            replay.verify.outcome,
            VerifyOutcome::Clean {
                frames: good_frames
            },
            "offset {offset}: recovered file must replay clean"
        );
        recovered += 1;
    }
    let _ = std::fs::remove_file(&path);
    println!("power-yank recovery: {recovered} torn offsets recovered, all replayed clean");
    assert!(recovered > 100);
}

#[test]
fn single_byte_corruption_is_always_reported_never_silent() {
    let (bytes, _live, boundaries) = build_journal("corrupt");
    let last_boundary = *boundaries.last().unwrap();

    let mut flipped = 0usize;
    let mut corrupt = 0usize;
    let mut divergent = 0usize;
    // Flip one byte at a stride inside the frame region (skip the
    // header — header flips are NotAJournal, covered in unit tests).
    for offset in (quilt_core::journal::HEADER_LEN..last_boundary).step_by(7) {
        let mut damaged = bytes.clone();
        damaged[offset] ^= 0x5A;
        let report = journal_replay(&damaged);
        match &report.verify.outcome {
            VerifyOutcome::Clean { .. } => {
                panic!("byte flip at {offset} verified clean — silent corruption!")
            }
            VerifyOutcome::Corrupt { .. } => corrupt += 1,
            VerifyOutcome::Divergence { .. } => divergent += 1,
            VerifyOutcome::TornTail { good_frames, .. } => {
                // A flip in a length prefix can look like a torn tail
                // (length says the frame needs more bytes than exist).
                // That is still an honest report of damaged goods:
                // the good-frame count must not exceed the frames
                // before the damaged one.
                let good = boundaries.iter().filter(|b| **b <= offset).count();
                assert!(
                    *good_frames <= good,
                    "flip at {offset} accepted a frame past the damage"
                );
            }
            other => panic!("byte flip at {offset}: unexpected outcome {other:?}"),
        }
        // The invariant everywhere above: corruption is reported,
        // never silently accepted as a wrong-but-verified state.
        flipped += 1;
    }
    println!(
        "corruption matrix: {flipped} flips -> {corrupt} corrupt, {divergent} divergent (all reported)"
    );
    assert!(flipped > 50);
    assert!(corrupt + divergent > 0);
}
