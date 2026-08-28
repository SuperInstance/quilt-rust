//! Desktop arrival peer (std): reads a serial-style byte stream, decodes
//! QuiltWire frames, stamps each arrival (road, rssi-if-present, epoch_ms),
//! and emits one `walks/2`-compatible JSONL line per frame.
//!
//! Subtext rule honored: the road and the arrival stamp are written by the
//! *receiver*, never self-declared by the sender (LINK-LAYER-FEASIBILITY
//! §2.1). RSSI is accepted per-chunk for radio drivers that observe it; the
//! pty/serial path passes `None` and the peer computes an app-layer quality
//! (delivery-ratio EWMA) instead — Rung 1's "app: latency+loss bucket".

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::frame::{Frame, FrameDecoder, Kind};
use crate::seq::{SeqTracker, SeqVerdict};
use crate::walks::{self, Arrival};

/// Peer configuration.
#[derive(Debug, Clone)]
pub struct PeerConfig {
    /// Road stamped on arrivals. Closed enum per EXPORTER.md §7:
    /// local | esp-now | ble | wifi | tcp | human | unknown.
    /// A wired USB-CDC link stamps `local` (wired, no radio, no broker);
    /// `serial` is a documented candidate for walks/3.
    pub road: String,
    /// Human name for the physical medium, recorded in `arrival_meta`.
    pub medium: String,
    /// Sending-cell name prefix (frame `cell_id` u8 → `"{prefix}-{id}"`).
    pub cell_prefix: String,
    /// EWMA alpha for link-quality estimates.
    pub alpha: f64,
}

impl Default for PeerConfig {
    fn default() -> Self {
        PeerConfig {
            road: "local".to_string(),
            medium: "usb-cdc".to_string(),
            cell_prefix: "cell".to_string(),
            alpha: 0.25,
        }
    }
}

/// Counters for honest reporting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PeerStats {
    pub frames: u64,
    pub lines: u64,
    pub gaps: u64,
    pub duplicates: u64,
    pub restarts: u64,
}

struct CellWalk {
    life: u32,
    seq: SeqTracker,
    prev_digest: String,
    /// Delivery-ratio EWMA (wired quality).
    quality: Option<f64>,
    /// RSSI EWMA in dBm (radio quality) — present only if a driver observed one.
    rssi_ewma: Option<f64>,
    gaps_total: u64,
}

impl CellWalk {
    fn new() -> Self {
        CellWalk {
            life: 1,
            seq: SeqTracker::new(),
            prev_digest: walks::GENESIS.to_string(),
            quality: None,
            rssi_ewma: None,
            gaps_total: 0,
        }
    }
}

/// The peer. Feed it bytes from any byte stream; JSONL lines come out.
pub struct ArrivalPeer {
    config: PeerConfig,
    decoder: FrameDecoder,
    cells: HashMap<u8, CellWalk>,
    stats: PeerStats,
}

impl ArrivalPeer {
    pub fn new(config: PeerConfig) -> Self {
        ArrivalPeer {
            config,
            decoder: FrameDecoder::new(),
            cells: HashMap::new(),
            stats: PeerStats::default(),
        }
    }

    /// Push a chunk of stream bytes. `epoch_ms` is the arrival wall-clock
    /// stamp for frames decoded from this chunk; `rssi` is the radio RSSI
    /// observed by the driver at this read, if any (radio roads only).
    /// Calls `emit` with one JSONL line (no trailing newline) per decoded
    /// frame, in arrival order. Returns the number of frames decoded.
    pub fn feed<F: FnMut(&str)>(
        &mut self,
        chunk: &[u8],
        epoch_ms: u64,
        rssi: Option<i16>,
        mut emit: F,
    ) -> usize {
        let mut n = 0;
        // Snapshot per-cell quality for arrival_meta before mutation.
        // Take the decoder out of `self` so the visitor closure can borrow
        // the rest of the peer mutably without fighting the borrow checker.
        let mut decoder = std::mem::take(&mut self.decoder);
        decoder.push(chunk, |frame| {
            n += 1;
            self.stats.frames += 1;
            let line = self.on_frame(frame, epoch_ms, rssi);
            self.stats.lines += 1;
            emit(&line);
        });
        self.decoder = decoder;
        n
    }

    fn on_frame(&mut self, frame: Frame, epoch_ms: u64, rssi: Option<i16>) -> String {
        let verdict = {
            let cell = self
                .cells
                .entry(frame.cell_id)
                .or_insert_with(CellWalk::new);
            match cell.seq.observe(frame.seq) {
                SeqVerdict::Start => SeqVerdict::Start,
                SeqVerdict::Contiguous => SeqVerdict::Contiguous,
                SeqVerdict::Gap { missing } => {
                    cell.gaps_total += missing as u64;
                    self.stats.gaps += missing as u64;
                    SeqVerdict::Gap { missing }
                }
                SeqVerdict::Duplicate => {
                    self.stats.duplicates += 1;
                    SeqVerdict::Duplicate
                }
                SeqVerdict::Restart => {
                    // Torn walk: never splice across the tear. Quality EWMA
                    // survives (the *link* didn't restart), the chain does.
                    self.stats.restarts += 1;
                    cell.life += 1;
                    cell.prev_digest = walks::GENESIS.to_string();
                    SeqVerdict::Restart
                }
            }
        };

        let cell = self
            .cells
            .get_mut(&frame.cell_id)
            .expect("cell entry exists");

        // ---- link quality: observed, receiver-side ----
        // Radio: RSSI EWMA (dBm). Wired: delivery-ratio EWMA (0..1),
        // penalized by seq gaps — "app: latency+loss bucket".
        if let Some(r) = rssi {
            cell.rssi_ewma = Some(match cell.rssi_ewma {
                Some(q) => q + self.config.alpha * (r as f64 - q),
                None => r as f64,
            });
        }
        let penalty = match verdict {
            SeqVerdict::Gap { missing } => (missing as f64 * 0.25).min(1.0),
            _ => 0.0,
        };
        cell.quality = Some(match cell.quality {
            Some(q) => q + self.config.alpha * ((1.0 - penalty) - q),
            None => 1.0 - penalty,
        });
        let link_quality = cell.rssi_ewma.or(cell.quality);

        // ---- walks/2 step ----
        let base = format!("{}-{}", self.config.cell_prefix, frame.cell_id);
        let walk_id = if cell.life > 1 {
            format!("{base}#{}", cell.life)
        } else {
            base.clone()
        };

        // Payload is float-free (value rides as raw bits) so the digest is
        // byte-stable across languages; the human value renders in meta.
        let payload = json!({
            "cell": frame.cell_id,
            "kind": frame.kind.as_str(),
            "seq": frame.seq,
            "tick": frame.tick,
            "value_bits": frame.value_bits,
        });

        // Kind → opcode: heartbeats are `tick` steps; value-bearing arrivals
        // are `effect` steps (inbound arrival receipts).
        let opcode = match frame.kind {
            Kind::Tick => "tick",
            _ => "effect",
        };

        let mut meta = Map::new();
        meta.insert("seq".into(), json!(frame.seq));
        meta.insert("kind".into(), json!(frame.kind.as_str()));
        let vf = frame.value_f32();
        meta.insert(
            "value".into(),
            if vf.is_finite() {
                json!((vf as f64))
            } else {
                Value::Null
            },
        );
        if let SeqVerdict::Gap { missing } = verdict {
            meta.insert("gap".into(), json!(missing));
        }
        if verdict == SeqVerdict::Duplicate {
            meta.insert("duplicate".into(), json!(true));
        }
        if verdict == SeqVerdict::Restart {
            meta.insert("restarted".into(), json!(true));
        }
        if cell.life > 1 {
            meta.insert("life".into(), json!(cell.life));
        }

        let mut arrival_meta = Map::new();
        arrival_meta.insert("arrival_epoch_ms".into(), json!(epoch_ms));
        arrival_meta.insert("medium".into(), json!(self.config.medium));
        arrival_meta.insert("seq_gap_total".into(), json!(cell.gaps_total));
        if let Some(r) = rssi {
            arrival_meta.insert("rssi".into(), json!(r));
        }

        let arrival = Arrival {
            cell_id: &base,
            walk_id: &walk_id,
            ts: epoch_ms,
            opcode,
            payload,
            meta: Value::Object(meta),
            road: &self.config.road,
            link_quality,
            arrival_meta: Value::Object(arrival_meta),
        };
        let (line, digest) = walks::step_line(&cell.prev_digest, &arrival);
        cell.prev_digest = digest;
        line
    }

    /// Current receiver-side quality estimate for a cell (radio: RSSI EWMA
    /// in dBm; wired: delivery-ratio EWMA). `None` before any observation.
    pub fn link_quality(&self, cell_id: u8) -> Option<f64> {
        let c = self.cells.get(&cell_id)?;
        c.rssi_ewma.or(c.quality)
    }

    pub fn stats(&self) -> PeerStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(n: u16) -> Vec<Vec<u8>> {
        (0..n)
            .map(|i| {
                Frame::from_f32(Kind::Delta, 7, i, i as u32 + 1, 21.5)
                    .encode()
                    .to_vec()
            })
            .collect()
    }

    #[test]
    fn emits_walks2_lines_with_chain() {
        let mut peer = ArrivalPeer::new(PeerConfig::default());
        let mut lines = String::new();
        let n = peer.feed(&frames(3).concat(), 1_700_000_000_000, None, |l| {
            lines.push_str(l);
            lines.push('\n');
        });
        assert_eq!(n, 3);
        let report = walks::verify(&lines).unwrap();
        assert_eq!(report.steps, 3);
        assert_eq!(report.walks, 1);
        assert_eq!(report.roads_unknown, 0);
        assert!(peer.link_quality(7).unwrap() > 0.99);
    }

    #[test]
    fn gap_penalizes_quality_and_annotates() {
        let mut peer = ArrivalPeer::new(PeerConfig::default());
        let mut f = frames(1);
        f.push(
            Frame::from_f32(Kind::Delta, 7, 5, 6, 21.5)
                .encode()
                .to_vec(),
        ); // seq 1..4 lost: gap of 4
        let mut lines = String::new();
        peer.feed(&f.concat(), 42, None, |l| {
            lines.push_str(l);
            lines.push('\n');
        });
        assert!(lines.contains("\"gap\":4"));
        assert!(peer.link_quality(7).unwrap() < 1.0);
        assert_eq!(peer.stats().gaps, 4);
        walks::verify(&lines).unwrap();
    }

    #[test]
    fn restart_tears_walk() {
        let mut peer = ArrivalPeer::new(PeerConfig::default());
        let mut bytes = frames(3).concat();
        bytes.extend_from_slice(&Frame::from_f32(Kind::Delta, 7, 0, 100, 22.0).encode());
        let mut lines = String::new();
        peer.feed(&bytes, 42, None, |l| {
            lines.push_str(l);
            lines.push('\n');
        });
        let report = walks::verify(&lines).unwrap();
        assert_eq!(report.steps, 4);
        assert_eq!(report.walks, 2); // torn: cell-7 and cell-7#2
        assert!(lines.contains("\"restarted\":true"));
    }
}
