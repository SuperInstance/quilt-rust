"""ledger — canonical JSON, the edge metric, and the hash-chained cell ledger.

Python port of `packages/core/src/ledger.rs`, plus the wire-edge
functions of the quilt-compat contract (docs/quilt-compat-contract.md,
compat/golden.json). Everything here is deterministic and pinned so the
same entries produce the same SHA-256 chain hashes in Rust, TypeScript,
and Python — the polyformal property, at cell grain.

Canonical JSON (the hash preimage form), pinned by cell-ledger.md §4
and quilt-compat-contract.md §2:

* compact — no whitespace;
* object keys sorted by UTF-8 byte order;
* integers rendered as integers (`85`), floats as shortest-round-trip
  decimals with the float marker preserved (`85.0`, `2.5`) — serde_json
  / ryū semantics. The float/int distinction is part of the preimage;
* strings via standard JSON escaping;
* SHA-256 (FIPS 180-4), lowercase hex, everywhere a hash appears.

Two layers, per the contract:

1. **The sealed unit** — `CellLedger` / `LedgerEntry`: the internal,
   append-only, hash-chained double entry (input posting + output
   posting + state edge + scored prediction). `hash = sha256(canonical
   (entry minus hash))`, `prev_hash` links to the prior seal (or the
   genesis root for the first entry).
2. **The wire edge** — the projection of a sealed entry into the
   language-neutral record every tier reads and writes::

       {"v": 1, "cell", "ts" (float millis), "before", "after",
        "delta" (§1.1: numeric diff / element-wise vector / null),
        "imbalance" (§1.2: |after − predict(before)|; L2 norm for
                     vectors; null when no prior — never fake a number),
        "provenance" (§1.3: sha256_hex(canonical_json(inputs array in
                      dependency-address order))),
        "chain" (§1.4: the prior entry's seal / the genesis root),
        "seq" (optional, contiguous from 1)}
"""

from __future__ import annotations

import hashlib
import json
import math

GENESIS_KIND = "quilt-cell-ledger/1"

#: Sentinel distinguishing "no prediction supplied" (persistence prior)
#: from an explicit forecast value.
_UNSET = object()


# ---------------------------------------------------------------------------
# Hashing — stdlib SHA-256, same bytes as the Rust inline implementation
# ---------------------------------------------------------------------------


def sha256_hex(data: bytes) -> str:
    """SHA-256 as 64 lowercase hex characters (matches ledger.rs)."""
    return hashlib.sha256(data).hexdigest()


# ---------------------------------------------------------------------------
# Canonical JSON — the hash preimage form
# ---------------------------------------------------------------------------


def fmt_float(x: float) -> str:
    """Shortest-round-trip float rendering, serde_json/ryū style.

    Python `repr` already yields the shortest round-trip decimal; we
    normalize the exponent form to ryū's (`1e-05` → `1e-5`,
    `1e+16` → `1e16`) so the canonical bytes match Rust.
    """
    if not math.isfinite(x):
        # serde_json cannot place non-finite numbers in a JSON Number;
        # they degrade to null rather than corrupt the chain.
        return "null"
    s = repr(float(x))
    if "e" in s or "E" in s:
        mant, _e, exp = s.lower().partition("e")
        sign = ""
        if exp.startswith("-"):
            sign, exp = "-", exp[1:]
        elif exp.startswith("+"):
            exp = exp[1:]
        exp = exp.lstrip("0") or "0"
        s = f"{mant}e{sign}{exp}"
    return s


def canonical_json(v) -> str:
    """Canonical JSON: compact, keys sorted, int/float distinguished."""
    out = []
    _write(v, out)
    return "".join(out)


def _write(v, out: list) -> None:
    if v is None:
        out.append("null")
    elif v is True:
        out.append("true")
    elif v is False:
        out.append("false")
    elif isinstance(v, str):
        out.append(json.dumps(v, ensure_ascii=False, separators=(",", ":")))
    elif isinstance(v, int):  # bool handled above
        out.append(str(v))
    elif isinstance(v, float):
        out.append(fmt_float(v))
    elif isinstance(v, (list, tuple)):
        out.append("[")
        for i, item in enumerate(v):
            if i:
                out.append(",")
            _write(item, out)
        out.append("]")
    elif isinstance(v, dict):
        out.append("{")
        # str keys; sorted() on str == UTF-8 byte order.
        for i, key in enumerate(sorted(v.keys())):
            if i:
                out.append(",")
            out.append(json.dumps(key, ensure_ascii=False, separators=(",", ":")))
            out.append(":")
            _write(v[key], out)
        out.append("}")
    else:
        raise TypeError(f"cannot canonicalize {type(v).__name__}: {v!r}")


# ---------------------------------------------------------------------------
# The generic distance metric (sealed entries' delta.magnitude)
# ---------------------------------------------------------------------------


def _is_json_num(v) -> bool:
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def _serde_eq(a, b) -> bool:
    """serde_json::Value equality: int/float are *different* numbers.

    json!(40) != json!(40.0) in Rust, so an edge 40 -> 40.0 is
    `changed: true` with `magnitude: 0.0` — the float-vs-int hazard
    cell-ledger.md warns about, preserved faithfully here.
    """
    if isinstance(a, bool) or isinstance(b, bool):
        return isinstance(a, bool) and isinstance(b, bool) and a == b
    if _is_json_num(a) and _is_json_num(b):
        return isinstance(a, int) == isinstance(b, int) and a == b
    if isinstance(a, str) and isinstance(b, str):
        return a == b
    if a is None and b is None:
        return True
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(_serde_eq(x, y) for x, y in zip(a, b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(_serde_eq(v, b[k]) for k, v in a.items())
    return False


def value_distance(a, b) -> float:
    """Total metric between two JSON values (port of ledger.rs).

    numbers: |a-b|; equal values: 0; arrays: mean of element-wise
    distances with missing elements costing 1.0; objects: mean over
    the key union with missing keys costing 1.0; any type shift: 1.0.
    """
    if _is_json_num(a) and _is_json_num(b):
        return abs(float(a) - float(b))
    if isinstance(a, list) and isinstance(b, list):
        n = max(len(a), len(b))
        if n == 0:
            return 0.0
        total = 0.0
        for i in range(n):
            if i < len(a) and i < len(b):
                total += value_distance(a[i], b[i])
            else:
                total += 1.0
        return total / n
    if isinstance(a, dict) and isinstance(b, dict):
        keys = set(a.keys()) | set(b.keys())
        if not keys:
            return 0.0
        total = 0.0
        for k in keys:
            if k in a and k in b:
                total += value_distance(a[k], b[k])
            else:
                total += 1.0
        return total / len(keys)
    if _serde_eq(a, b):
        return 0.0
    return 1.0


# ---------------------------------------------------------------------------
# The wire edge — quilt-compat-contract §1
# ---------------------------------------------------------------------------


def wire_delta(before, after):
    """§1.1 — `delta = after − before`, first-person.

    number → scalar difference; equal-length numeric vectors →
    element-wise difference; anything else (strings, booleans, objects,
    mixed, `before: null`) → None. Never fake a number.
    """
    if _is_json_num(before) and _is_json_num(after):
        return float(after) - float(before)
    if (
        isinstance(before, list)
        and isinstance(after, list)
        and len(before) == len(after)
    ):
        out = []
        for b, a in zip(before, after):
            if not (_is_json_num(b) and _is_json_num(a)):
                return None
            out.append(float(a) - float(b))
        return out
    return None


def wire_imbalance(before, after, predicted=_UNSET):
    """§1.2 — `|after − predict(before)|`, the JEPA loss at cell grain.

    Default predictor: the persistence prior (`predict(before) = before`)
    — scalar edges give `|after − before|`, equal-length numeric
    vectors give the L2 norm `‖after − before‖₂` (the kernel's `d_mu`
    shape — a norm, not a vector). No prior (`before: null` without an
    explicit forecast) → None. Never fake a number.
    """
    prior = before if predicted is _UNSET else predicted
    if _is_json_num(prior) and _is_json_num(after):
        return abs(float(after) - float(prior))
    if (
        isinstance(prior, list)
        and isinstance(after, list)
        and len(prior) == len(after)
    ):
        total = 0.0
        for b, a in zip(prior, after):
            if not (_is_json_num(b) and _is_json_num(a)):
                return None
            total += (float(a) - float(b)) ** 2
        return math.sqrt(total)
    return None


def wire_provenance(inputs) -> str:
    """§1.3 — `sha256_hex(canonical_json(inputs))`.

    `inputs` is the JSON array of input values in dependency-address
    order (dependencies sorted by UTF-8 byte order of their ids).
    Single inputs are still wrapped in the array.
    """
    return sha256_hex(canonical_json(list(inputs)).encode("utf-8"))


def wire_edge(cell: str, ts: float, before, after, inputs, chain: str, seq: int | None = None) -> dict:
    """Build the full wire edge record (quilt-compat/1 §1)."""
    edge = {
        "v": 1,
        "cell": cell,
        "ts": float(ts),
        "before": before,
        "after": after,
        "delta": wire_delta(before, after),
        "imbalance": wire_imbalance(before, after),
        "provenance": wire_provenance(inputs),
        "chain": chain,
    }
    if seq is not None:
        edge["seq"] = seq
    return edge


# ---------------------------------------------------------------------------
# The sealed unit — Rust CellLedger port (LedgerEntry shape)
# ---------------------------------------------------------------------------


class Provenance(dict):
    """First-person 'who touched me' (Rust `Provenance`).

    origin: get | set | push | system; caller: cell id or None;
    trace: ancestor chain, outermost first. Serializes with caller and
    trace omitted when unset, matching serde skip_serializing_if.
    """

    def __init__(self, origin: str = "system", caller: str | None = None, trace=()):
        if origin not in ("get", "set", "push", "system"):
            raise ValueError(f"bad provenance origin: {origin!r}")
        super().__init__(origin=origin)
        if caller is not None:
            self["caller"] = caller
        if trace:
            self["trace"] = list(trace)


def _posting(side: str, value, ts: int) -> dict:
    return {"side": side, "value": value, "ts": ts}


class LedgerEntry:
    """One sealed double entry — the internal unit of cell-ledger.md.

    Body (the hash preimage is this minus `hash`, canonically
    serialized): seq, ts, input posting, output posting, provenance,
    delta {before, after, changed, magnitude}, expected?, imbalance?,
    prev_hash, hash.
    """

    def __init__(self, body: dict):
        self.body = body

    # -- accessors ---------------------------------------------------------
    @property
    def seq(self) -> int:
        return self.body["seq"]

    @property
    def ts(self) -> int:
        return self.body["ts"]

    @property
    def before(self):
        return self.body["delta"]["before"]

    @property
    def after(self):
        return self.body["delta"]["after"]

    @property
    def changed(self) -> bool:
        return self.body["delta"]["changed"]

    @property
    def magnitude(self) -> float:
        return self.body["delta"]["magnitude"]

    @property
    def imbalance(self):
        return self.body.get("imbalance")

    @property
    def expected(self):
        return self.body.get("expected")

    @property
    def prev_hash(self) -> str:
        return self.body["prev_hash"]

    @property
    def hash(self) -> str:
        return self.body["hash"]

    @property
    def input_value(self):
        return self.body["input"]["value"]

    @property
    def output_value(self):
        return self.body["output"]["value"]

    # -- hashing / projection -----------------------------------------------

    def seal(self) -> str:
        """sha256 over canonical JSON of the entry minus its hash."""
        body = {k: v for k, v in self.body.items() if k != "hash"}
        return sha256_hex(canonical_json(body).encode("utf-8"))

    def to_wire(self, cell_id: str) -> dict:
        """Project onto the quilt-compat wire edge (§1.5)."""
        return wire_edge(
            cell=cell_id,
            ts=float(self.ts),
            before=self.before,
            after=self.after,
            inputs=[self.input_value],
            chain=self.prev_hash,
            seq=self.seq,
        )

    def to_json(self) -> dict:
        return json.loads(json.dumps(self.body))

    def __repr__(self) -> str:  # pragma: no cover
        return (
            f"LedgerEntry(seq={self.seq}, ts={self.ts}, "
            f"before={self.before!r}, after={self.after!r}, "
            f"magnitude={self.magnitude}, imbalance={self.imbalance}, "
            f"hash={self.hash[:12]}…)"
        )


class CellLedger:
    """A per-cell, append-only, hash-chained, double-entry ledger.

    Port of Rust `CellLedger` (ledger.rs) — the sealed side of the
    compat contract. Pure data: callers pass timestamps, no clocks, no
    I/O. `reconcile()` audits the books; `wire_edges()` projects onto
    the interchange schema.
    """

    def __init__(self, cell_id: str):
        """A fresh ledger: state null, no genesis (Rust `new`)."""
        self.cell_id = cell_id
        self.genesis = None
        self.genesis_ts = None
        self._has_genesis = False
        self.state = None
        self._entries: list[LedgerEntry] = []
        self.pending: list[dict] = []
        self._next_seq = 1
        self._next_ticket = 1

    @classmethod
    def with_genesis(cls, cell_id: str, genesis, genesis_ts: int) -> "CellLedger":
        """Seed a known initial state (Rust `with_genesis`) — committed
        by the chain root; scores the first transaction against the
        persistence prior."""
        led = cls(cell_id)
        led.genesis = genesis
        led.genesis_ts = genesis_ts
        led._has_genesis = True
        led.state = genesis
        return led

    # -- recording -----------------------------------------------------------

    def record(self, input_value, output_value, ts: int, provenance: Provenance | None = None,
               expected=_UNSET) -> LedgerEntry:
        """Record a complete double entry atomically (Rust `record[_with]`).

        Under the default persistence prior the prediction is the
        cell's `before` state and surprise == edge magnitude. An
        explicit `expected` is recorded — and hashed — either way.
        """
        ts = int(ts)
        return self._append(
            input_value, ts, output_value, ts,
            provenance or Provenance(), expected,
        )

    def open_input(self, input_value, ts: int, provenance: Provenance | None = None) -> int:
        """Post a debit without its credit (async cells). Returns the
        ticket for `settle_output`. Does not move state or the chain."""
        ticket = self._next_ticket
        self._next_ticket += 1
        self.pending.append(
            {
                "ticket": ticket,
                "ts": int(ts),
                "input": input_value,
                "provenance": provenance or Provenance(),
            }
        )
        return ticket

    def settle_output(self, ticket: int, output_value, ts: int, expected=_UNSET) -> LedgerEntry:
        """Close an open input with its credit, sealing the pair."""
        for i, p in enumerate(self.pending):
            if p["ticket"] == ticket:
                pending = self.pending.pop(i)
                return self._append(
                    pending["input"], pending["ts"], output_value, int(ts),
                    pending["provenance"], expected,
                )
        raise KeyError(
            f"ledger '{self.cell_id}': no open input with ticket {ticket}"
        )

    def _append(self, input_value, input_ts: int, output_value, output_ts: int,
                provenance: Provenance, expected) -> LedgerEntry:
        before = self.state
        after = output_value
        magnitude = value_distance(before, after)
        changed = not _serde_eq(before, after)

        # A prior exists iff genesis or a completed entry; without one
        # no surprise is claimed (never fake a number). `expected=None`
        # means "no forecast supplied" — the persistence prior applies.
        has_prior = self._has_genesis or bool(self._entries)
        if expected is not _UNSET and expected is not None:
            imbalance = value_distance(expected, after)
        elif has_prior:
            expected = before
            imbalance = magnitude
        else:
            expected, imbalance = None, None

        body = {
            "seq": self._next_seq,
            "ts": input_ts,
            "input": _posting("input", input_value, input_ts),
            "output": _posting("output", output_value, output_ts),
            "provenance": dict(provenance),
            "delta": {
                "before": before,
                "after": after,
                "changed": changed,
                "magnitude": magnitude,
            },
            "prev_hash": self.chain_hash(),
        }
        if expected is not None:
            body["expected"] = expected
        if imbalance is not None:
            body["imbalance"] = imbalance
        self._next_seq += 1
        entry = LedgerEntry(body)
        body["hash"] = entry.seal()

        self.state = after
        self._entries.append(entry)
        return entry

    # -- hashing / audit -------------------------------------------------------

    def genesis_commit(self) -> str:
        """Chain root for an empty ledger — commits cell identity +
        genesis state. Byte-identical to Rust `CellLedger`."""
        body = {
            "kind": GENESIS_KIND,
            "cell_id": self.cell_id,
            "genesis": self.genesis,        # null when genesis-less
            "genesis_ts": self.genesis_ts,  # null when genesis-less
        }
        return sha256_hex(canonical_json(body).encode("utf-8"))

    def chain_hash(self) -> str:
        """Head of the chain: last seal, or the genesis commit."""
        return self._entries[-1].hash if self._entries else self.genesis_commit()

    def verify_chain(self) -> dict:
        """Recompute every seal and prev-link (Rust `ChainAudit`)."""
        expected_prev = self.genesis_commit()
        for entry in self._entries:
            if entry.prev_hash != expected_prev or entry.hash != entry.seal():
                return {
                    "verified": entry.seq - 1,
                    "intact": False,
                    "first_break": entry.seq,
                }
            expected_prev = entry.hash
        return {"verified": len(self._entries), "intact": True, "first_break": None}

    def reconcile(self) -> dict:
        """The books: matched pairs, open inputs, chain, continuity,
        surprise totals (Rust `Reconciliation`)."""
        audit = self.verify_chain()
        continuity = True
        prior = self.genesis  # None stands in for Value::Null (Rust)
        for entry in self._entries:
            if not _serde_eq(entry.before, prior):
                continuity = False
                break
            prior = entry.after

        matched = sum(
            1
            for e in self._entries
            if e.body["input"]["side"] == "input"
            and e.body["output"]["side"] == "output"
        )
        scored = [e.imbalance for e in self._entries if e.imbalance is not None]
        total = sum(scored)
        return {
            "cell_id": self.cell_id,
            "entries": len(self._entries),
            "open_inputs": len(self.pending),
            "matched_pairs": matched,
            "chain_intact": audit["intact"],
            "first_break": audit["first_break"],
            "continuity_intact": continuity,
            "total_surprise": total,
            "mean_surprise": (total / len(scored)) if scored else None,
            "balanced": (
                not self.pending
                and matched == len(self._entries)
                and audit["intact"]
                and continuity
            ),
        }

    def replay(self, until_ts: int) -> dict:
        """Point-in-time view: entries at/before the cutoff, the state
        reconstructed from them, and cumulative surprise of the prefix."""
        entries = [e for e in self._entries if e.ts <= until_ts]
        state = entries[-1].after if entries else self.genesis
        return {
            "cell_id": self.cell_id,
            "until_ts": until_ts,
            "replayed": len(entries),
            "state": state,
            "surprise": sum(e.imbalance for e in entries if e.imbalance is not None),
        }

    # -- accessors ---------------------------------------------------------------

    @property
    def entries(self) -> list[LedgerEntry]:
        return list(self._entries)

    @property
    def head(self) -> LedgerEntry | None:
        return self._entries[-1] if self._entries else None

    def wire_edges(self) -> list[dict]:
        """The whole history projected onto the wire edge schema."""
        return [e.to_wire(self.cell_id) for e in self._entries]

    def __len__(self) -> int:
        return len(self._entries)
