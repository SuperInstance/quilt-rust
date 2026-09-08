# 🦀 quilt-rust

> **The Quilt cell-fabric runtime in Rust.** Part of the [polyformalism](https://github.com/SuperInstance/quilt-claude-charts/blob/main/QUILT_CHARTER.md) — the same cell model, expressed in 12+ languages, byte-exact compatible.

<p align="center">
  <img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="Apache-2.0">
  <img src="https://img.shields.io/badge/language-Rust-blue.svg" alt="Rust">
  <img src="https://img.shields.io/badge/hash-0xe435d91d6d92a1d8-brightgreen.svg" alt="byte-exact">
</p>

## ✦ Why this port exists

The Rust port of the Quilt. The Rust port is distinctive because of its position in the language hierarchy — `rustc-compileable, Rust-idiomatic, byte-exact with the rest of the polyformalism.

## ✦ The 5 opcodes

```
BIND(cell, dials)   # set dials, idempotent
LINK(c1, c2)        # add an undirected edge
EFFECT(cell)        # propagate dial[0] to neighbors
VIEW(cell)          # return dials
TICK(fabric)        # advance all dials by 1, alternating direction
```

The canonical serialization is `type(1) || id(8 LE) || dials(32 LE) || neighbors(8*N LE)`. The state hash is FNV-1a 64-bit. The test cell (id=1, dials=[1..16], neighbors=[2,3,4]) produces `0xe435d91d6d92a1d8` byte-exactly.

## ✦ The full polyformalism

| Lang | Hash | Tests |
|------|------|-------|
| Python 3 | ✓ | 7/7 |
| C99 (Rust) | ✓ | manual |
| Rust | ✓ | 6/6 |
| Go | ✓ | 7/7 |
| Zig | ✓ | 7/7 |
| Mojo | ✓ | ref |
| Verilog | ✓ | manual |
| VHDL | ✓ | manual |
| JavaScript | ✓ | live |
| TypeScript | ✓ | 5/5 |

## ✦ The educational root

Every port points back to the [Quilt Charter](https://github.com/SuperInstance/quilt-claude-charts/blob/main/QUILT_CHARTER.md) — the educational root document.

## ✦ See also

- [The Quilt Charter](https://github.com/SuperInstance/quilt-claude-charts/blob/main/QUILT_CHARTER.md)
- [quilt-claude-charts](https://github.com/SuperInstance/quilt-claude-charts) — protocol + 3 charts
- [AI-Writings](https://github.com/SuperInstance/AI-Writings) — the canon (230+ papers)
- [live-canon.superinstance.dev](https://live-canon.superinstance.dev) — the live worker
- [quilt-cowboy](https://github.com/SuperInstance/quilt-cowboy) — the writers' room

## ✦ License

Apache-2.0. Free as in freedom.
