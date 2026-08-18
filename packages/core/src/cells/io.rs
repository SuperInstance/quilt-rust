//! # cells/io.rs
//!
//! Bidirectional I/O cells. The runtime's interface to physical or
//! virtual ports that can be read from and written to.
//!
//! ## Role in the system
//!
//! `io` is the cell kind for **bidirectional ports**: a GPIO pin, a
//! serial port, a CAN bus address, a virtual actuator, a memory-
//! mapped register, etc. Like `sensor`, the engine doesn't know
//! *how* to talk to the port — that's the adapter's job. But unlike
//! `sensor`, an `io` cell has a `direction` (in, out, or both) and
//! can be written to as well as read from.
//!
//! ## Depends on
//!
//! - `crate::types` — `CellValue`, `CellStatus`, `Direction`.
//!
//! ## Used by
//!
//! - The engine's `push` method (incoming side).
//! - User code that calls `engine.set(id, value)` to write to an
//!   out-direction I/O cell.
//! - Adapters in the embedding application that bridge physical
//!   ports to the engine.
//!
//! ## Key decision
//!
//! We treat I/O as a value type, not a capability. The fact that
//! writing to a port produces a side effect is captured by the
//! `Effects` field on the resulting `CellValue` and by the `Direction`
//! on the `CellDef`. This keeps the engine simple: a write to an
//! I/O cell is a `set` like any other. The adapter observes the
//! change via subscription and translates it to the wire protocol.

use crate::types::{now_millis, CellStatus, CellValue};

/// Build a `CellValue` for an I/O event. Used by the engine's
/// `push` method when an adapter receives new data on an inbound
/// I/O port.
pub fn make_io_value(data: serde_json::Value) -> CellValue {
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
    fn make_io_value_basic() {
        let v = make_io_value(json!({"pin": 17, "state": true}));
        assert_eq!(v.data["pin"], 17);
        assert_eq!(v.data["state"], true);
        assert_eq!(v.status, CellStatus::Ready);
    }
}
