//! # cells/sensor.rs
//!
//! Push-based input cells. The runtime stores values that come in
//! from outside the engine (MQTT, Modbus, GPIO, simulator, file tail,
//! web socket, etc.). The engine has no built-in adapters — adapters
//! live in the embedding application and call `engine.push(id, data)`
//! to inject new readings.
//!
//! ## Role in the system
//!
//! `sensor` is one of the eight cell kinds. In the type system it
//! sits next to `value` (static), `formula` (computed), and `api`
//! (queried). A `sensor` cell is **read from outside the engine**.
//! Its value is whatever the adapter last pushed; until something
//! is pushed, the cell holds `idle`.
//!
//! ## Depends on
//!
//! - `crate::types` — `CellValue`, `CellStatus`.
//!
//! ## Used by
//!
//! - The engine's `push` method, which calls `make_sensor_value` to
//!   build the `CellValue` to store.
//! - Adapters in the embedding application (out of scope for
//!   `quilt-core`).
//!
//! ## Key decision
//!
//! The "what data the sensor reports" is NOT decided by this cell.
//! It's decided by the adapter, which the user plugs in. The
//! `CellDef::source` field is an opaque string (e.g. `"mqtt://..."`,
//! `"simulated"`) that the adapter interprets.
//!
//! This means `quilt-core` itself has no I/O dependencies for
//! sensors — a deliberate design choice. The cost is that you need
//! to write a small adapter (typically 50-200 lines) to connect a
//! sensor to your engine. The benefit is that `quilt-core` stays
//! tiny and embeddable.

use crate::types::{now_millis, CellStatus, CellValue};

/// Build a `CellValue` for a sensor reading. Used by the engine's
/// `push` method when an adapter injects a new value.
///
/// The cell's `data` is the JSON-serializable reading. The status
/// is `Ready`. The `effects` list is empty (a sensor does not
/// produce effects; the *act of pushing* is the effect, and that's
/// owned by the adapter, not the cell).
pub fn make_sensor_value(data: serde_json::Value) -> CellValue {
    CellValue {
        data,
        status: CellStatus::Ready,
        computed_at: Some(now_millis()),
        error: None,
        effects: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn make_sensor_value_basic() {
        let v = make_sensor_value(json!({"temperature": 21.5, "unit": "celsius"}));
        assert_eq!(v.data["temperature"], 21.5);
        assert_eq!(v.data["unit"], "celsius");
        assert_eq!(v.status, CellStatus::Ready);
        assert!(v.computed_at.is_some());
        assert!(v.error.is_none());
        assert!(v.effects.is_empty());
    }
}
