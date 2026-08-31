//! Tests for quilt-polyformalism — the 9-opcode Rust port.

use quilt_polyformalism::*;

#[test]
fn nine_opcodes_present() {
    assert_eq!(Op::COUNT, 9);
    assert_eq!(Op::Bind.name(), "BIND");
    assert_eq!(Op::Link.name(), "LINK");
    assert_eq!(Op::Effect.name(), "EFFECT");
    assert_eq!(Op::View.name(), "VIEW");
    assert_eq!(Op::Tick.name(), "TICK");
    assert_eq!(Op::Forget.name(), "FORGET");
    assert_eq!(Op::Proof.name(), "PROOF");
    assert_eq!(Op::Route.name(), "ROUTE");
    assert_eq!(Op::Crdt.name(), "CRDT");
}

#[test]
fn bind_basic() {
    let mut e = Engine::new();
    assert!(e.bind("x", Value::Int(7)));
    assert_eq!(e.view("x"), Some(&Value::Int(7)));
}

#[test]
fn bind_idempotent_law() {
    let mut e = Engine::new();
    assert!(law_bind_idempotent(&mut e, "x", Value::Int(7)));
    assert_eq!(e.view("x"), Some(&Value::Int(7)));
}

#[test]
fn view_purity_law() {
    let mut e = Engine::new();
    e.bind("k", Value::Int(99));
    assert!(law_view_purity(&e, "k"));
}

#[test]
fn tick_monotonic_law() {
    let mut e = Engine::new();
    assert_eq!(e.tick, 0);
    assert!(law_tick_monotonic(&mut e));
    assert_eq!(e.tick, 1);
    e.tick();
    e.tick();
    assert_eq!(e.tick, 3);
}

#[test]
fn forget_complete_law() {
    let mut e = Engine::new();
    e.bind("temp", Value::Int(123));
    assert!(law_forget_complete(&mut e, "temp"));
    assert!(e.view("temp").is_none());
}

#[test]
fn proof_ring_append_and_verify() {
    let mut ring = ProofRing::new(8);
    let v1 = Value::Int(1);
    let v2 = Value::Int(2);
    ring.append(&v1, 1, 1);
    ring.append(&v2, 2, 2);
    assert_eq!(ring.count, 2);
    assert!(ring.verify());
}

#[test]
fn proof_ring_distinct_hashes() {
    let mut ring = ProofRing::new(8);
    let v1 = Value::Int(1);
    let v2 = Value::Int(2);
    let v3 = Value::Bool(false);
    ring.append(&v1, 1, 1);
    ring.append(&v2, 2, 2);
    ring.append(&v3, 3, 3);
    // All three should have distinct state_hashes.
    assert_ne!(ring.ring[0].state_hash, ring.ring[1].state_hash);
    assert_ne!(ring.ring[0].state_hash, ring.ring[2].state_hash);
}

#[test]
fn route_policy_null() {
    assert_eq!(route_policy(&Value::Null), RouteKind::TextLog);
}

#[test]
fn route_policy_bool() {
    assert_eq!(route_policy(&Value::Bool(true)), RouteKind::ParamUpdate);
}

#[test]
fn route_policy_int() {
    assert_eq!(route_policy(&Value::Int(42)), RouteKind::SparseIdx);
}

#[test]
fn route_policy_float() {
    assert_eq!(route_policy(&Value::Float(3.14)), RouteKind::DenseVec);
}

#[test]
fn route_policy_short_str() {
    assert_eq!(route_policy(&Value::Str("hi")), RouteKind::HierStore);
}

#[test]
fn route_policy_long_str() {
    let s = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.";
    assert_eq!(route_policy(&Value::Str(s)), RouteKind::DenseVec);
}

#[test]
fn pn_counter_basic() {
    let mut c = PnCounter::new();
    assert_eq!(c.value(), 0);
    c.inc(0); c.inc(0); c.inc(1);
    assert_eq!(c.value(), 3);
    c.dec(0);
    assert_eq!(c.value(), 2);
}

#[test]
fn pn_counter_convergence() {
    // Two replicas. Same ops in different order. Same value.
    let mut a = PnCounter::new();
    let mut b = PnCounter::new();
    // A: inc(0), inc(0), inc(1)
    a.inc(0); a.inc(0); a.inc(1);
    // B: inc(1), inc(0), inc(0)
    b.inc(1); b.inc(0); b.inc(0);
    assert_eq!(a.value(), b.value());
    a.merge(&b);
    assert_eq!(a.value(), 3);
}

#[test]
fn fnv1a64_distinct_for_distinct_values() {
    let h1 = fnv1a64(&Value::Int(1));
    let h2 = fnv1a64(&Value::Int(2));
    let h3 = fnv1a64(&Value::Bool(false));
    assert_ne!(h1, h2);
    assert_ne!(h1, h3);
}
