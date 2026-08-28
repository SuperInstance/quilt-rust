//! `walks/2` walk-step construction and verification (std only).
//!
//! Byte-compatible with the dissertation's walks-bridge exporter
//! (`research/walks-bridge/exporter.py`, EXPORTER.md §3+§7): canonical JSON
//! is compact + code-point-sorted keys + raw UTF-8, exactly
//! `json.dumps(obj, sort_keys=True, separators=(",",":"), ensure_ascii=False)`.
//! serde_json's default `Map` is a `BTreeMap` (sorted) and its string
//! escaping matches Python's for the ASCII enum/ident strings used here; no
//! floats ever enter a digested field, so the two implementations produce
//! identical digest bytes.
//!
//! Six-field core (digest covers these; `prev_digest` inside the hashed core
//! *is* the chain link):
//!
//! ```text
//! core = {walk_id, ts, cell_id, opcode, payload_digest, prev_digest}
//! payload_digest = sha256(canonical(payload))
//! digest         = sha256(canonical(core))
//! ```
//!
//! walks/2 arrival-path fields (`road`, `link_quality`, `arrival_meta`) ride
//! outside the core, same tier as `meta` — annotations about ingress, not
//! walk identity. walks/1 chain logic therefore verifies walks/2 lines.

use serde_json::{json, Map, Value};

/// The road enum — closed, per EXPORTER.md §7.
pub const ROADS: [&str; 7] = ["local", "esp-now", "ble", "wifi", "tcp", "human", "unknown"];
/// Walk opcodes, per EXPORTER.md §3.
pub const OPCODES: [&str; 5] = ["qm_bind", "link", "effect", "view", "tick"];

pub const GENESIS: &str = "GENESIS";

fn canonical(value: &Value) -> String {
    // BTreeMap-backed Map => sorted keys; to_string => compact separators;
    // no ASCII escaping by default. Matches exporter.py `canonical()`.
    serde_json::to_string(value).expect("canonical JSON is infallible for our shapes")
}

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let out = h.finalize();
    let mut hex = String::with_capacity(64);
    for b in out {
        hex.push_str(&format!("{:02x}", b));
    }
    hex
}

/// Inputs to one walk-step, already stamped at arrival.
pub struct Arrival<'a> {
    /// e.g. `"cell-7"` — the sending cell's fabric-facing name.
    pub cell_id: &'a str,
    /// `cell_id` plus `#life` when the walk tore and a new one opened.
    pub walk_id: &'a str,
    /// Arrival stamp (epoch ms) — becomes the step `ts`.
    pub ts: u64,
    /// walks/2 opcode for this step.
    pub opcode: &'a str,
    /// Sender's frame fields, digested as the payload.
    pub payload: Value,
    /// Undigested extras: seq, kind, value rendering, gap notes, life.
    pub meta: Value,
    /// Transport that carried the arrival (closed enum).
    pub road: &'a str,
    /// EWMA quality: radio RSSI (dBm, negative) or delivery ratio (0..1)
    /// for wired links; `None` only before any observation.
    pub link_quality: Option<f64>,
    /// Free-form arrival annotations (medium, epoch, rssi-if-present).
    pub arrival_meta: Value,
}

/// Build one `walks/2` JSONL line (no trailing newline).
pub fn step_line(prev_digest: &str, a: &Arrival<'_>) -> (String, String) {
    let payload_digest = sha256_hex(&canonical(&a.payload));
    let core = json!({
        "walk_id": a.walk_id,
        "ts": a.ts,
        "cell_id": a.cell_id,
        "opcode": a.opcode,
        "payload_digest": payload_digest,
        "prev_digest": prev_digest,
    });
    let digest = sha256_hex(&canonical(&core));

    let mut line = Map::new();
    line.insert("walk_id".into(), Value::String(a.walk_id.into()));
    line.insert("ts".into(), json!(a.ts));
    line.insert("cell_id".into(), Value::String(a.cell_id.into()));
    line.insert("opcode".into(), Value::String(a.opcode.into()));
    line.insert("payload_digest".into(), Value::String(payload_digest));
    line.insert("prev_digest".into(), Value::String(prev_digest.into()));
    line.insert("digest".into(), Value::String(digest.clone()));
    line.insert("meta".into(), a.meta.clone());
    line.insert("road".into(), Value::String(a.road.into()));
    line.insert(
        "link_quality".into(),
        match a.link_quality {
            Some(q) => serde_json::Number::from_f64(q)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            None => Value::Null,
        },
    );
    line.insert("arrival_meta".into(), a.arrival_meta.clone());
    (serde_json::to_string(&Value::Object(line)).unwrap(), digest)
}

/// Verifier report, mirroring exporter.py `verify()` counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifyReport {
    pub steps: usize,
    pub walks: usize,
    pub roads_unknown: usize,
}

/// Verify a `walks/2` (or `walks/1`) JSONL document exactly the way
/// `exporter.py --verify` does: recompute every digest, check chain linkage
/// and append-only order, validate road/link_quality/arrival_meta shapes.
/// walks/1 rows (no `road`) map to `unknown` and are not rewritten.
pub fn verify(content: &str) -> Result<VerifyReport, String> {
    let mut steps = 0usize;
    let mut roads_unknown = 0usize;
    let mut last_digest: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for (lineno, line) in content.lines().enumerate() {
        let lineno = lineno + 1;
        if line.trim().is_empty() {
            continue;
        }
        let s: Value =
            serde_json::from_str(line).map_err(|e| format!("line {lineno}: not JSON: {e}"))?;
        let obj = s
            .as_object()
            .ok_or_else(|| format!("line {lineno}: not an object"))?;

        let core_keys = [
            "walk_id",
            "ts",
            "cell_id",
            "opcode",
            "payload_digest",
            "prev_digest",
        ];
        let mut core = Map::new();
        for k in core_keys {
            core.insert(
                k.into(),
                obj.get(k)
                    .cloned()
                    .ok_or_else(|| format!("line {lineno}: missing core field {k}"))?,
            );
        }
        let digest = obj
            .get("digest")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("line {lineno}: missing digest"))?;
        if sha256_hex(&canonical(&Value::Object(core))) != digest {
            return Err(format!("line {lineno}: digest mismatch"));
        }
        let opcode = obj["opcode"].as_str().unwrap_or("");
        if !OPCODES.contains(&opcode) {
            return Err(format!("line {lineno}: bad opcode {opcode:?}"));
        }
        let walk_id = obj["walk_id"].as_str().unwrap_or("").to_string();
        let prev = obj["prev_digest"].as_str().unwrap_or("").to_string();
        match last_digest.get(&walk_id) {
            Some(last) if &prev != last => {
                return Err(format!("line {lineno}: chain break in walk {walk_id}"))
            }
            None if prev != GENESIS => {
                return Err(format!(
                    "line {lineno}: walk {walk_id} opened without GENESIS"
                ))
            }
            _ => {}
        }
        last_digest.insert(walk_id, digest.to_string());

        // walks/2 arrival-path fields (tolerant of walks/1 rows).
        let road = match obj.get("road") {
            Some(Value::String(r)) => {
                if !ROADS.contains(&r.as_str()) {
                    return Err(format!("line {lineno}: bad road {r:?} (not in ROADS)"));
                }
                r.clone()
            }
            Some(_) => return Err(format!("line {lineno}: road is not a string")),
            None => "unknown".to_string(), // walks/1 row — mapped, never rewritten
        };
        match obj.get("link_quality") {
            None | Some(Value::Null) => {}
            Some(Value::Number(n)) if n.as_f64().is_some() => {}
            Some(_) => {
                return Err(format!(
                    "line {lineno}: link_quality is not a number or null"
                ))
            }
        }
        match obj.get("arrival_meta") {
            None | Some(Value::Object(_)) => {}
            Some(_) => return Err(format!("line {lineno}: arrival_meta is not an object")),
        }
        if road == "unknown" {
            roads_unknown += 1;
        }
        steps += 1;
    }
    Ok(VerifyReport {
        steps,
        walks: last_digest.len(),
        roads_unknown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genesis_line_verifies() {
        let arrival = Arrival {
            cell_id: "cell-7",
            walk_id: "cell-7",
            ts: 1_700_000_000_123,
            opcode: "effect",
            payload: json!({"cell": 7, "kind": "delta", "seq": 1, "tick": 10, "value_bits": 1103101952u32}),
            meta: json!({"seq": 1, "kind": "delta"}),
            road: "local",
            link_quality: Some(1.0),
            arrival_meta: json!({"arrival_epoch_ms": 1_700_000_000_123u64, "medium": "usb-cdc"}),
        };
        let (line, digest) = step_line(GENESIS, &arrival);
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["digest"].as_str().unwrap(), digest);
        assert_eq!(v["road"].as_str().unwrap(), "local");
        let report = verify(&line).unwrap();
        assert_eq!(report.steps, 1);
        assert_eq!(report.walks, 1);
        assert_eq!(report.roads_unknown, 0);
        // A second step chaining on the first verifies too.
        let (line2, _) = step_line(&digest, &arrival);
        let doc = format!("{line}\n{line2}\n");
        let report = verify(&doc).unwrap();
        assert_eq!(report.steps, 2);
    }

    #[test]
    fn chain_break_detected() {
        let arrival = Arrival {
            cell_id: "c",
            walk_id: "c",
            ts: 5,
            opcode: "tick",
            payload: json!({"seq": 0}),
            meta: json!({"seq": 0}),
            road: "human",
            link_quality: None,
            arrival_meta: json!({}),
        };
        let (l1, d1) = step_line(GENESIS, &arrival);
        let (l2, _) = step_line(&d1, &arrival);
        // Corrupt the chain: step 2 claims wrong prev.
        let mut v: Value = serde_json::from_str(&l2).unwrap();
        v["prev_digest"] = Value::String(String::from(GENESIS));
        let bad = serde_json::to_string(&v).unwrap();
        let doc = format!("{l1}\n{bad}\n");
        assert!(verify(&doc).is_err());
    }

    #[test]
    fn bad_road_rejected() {
        let arrival = Arrival {
            cell_id: "c",
            walk_id: "c",
            ts: 5,
            opcode: "tick",
            payload: json!({"seq": 0}),
            meta: json!({"seq": 0}),
            road: "carrier-pigeon",
            link_quality: None,
            arrival_meta: json!({}),
        };
        let (line, _) = step_line(GENESIS, &arrival);
        assert!(verify(&line).is_err());
    }
}
