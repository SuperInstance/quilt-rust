"""engine — the reactive cell engine (Python binding of QuiltEngine).

Same semantics as `packages/core/src/engine.rs`:

* **value** cells hold static data; **formula** cells are pure reactive
  expressions evaluated lazily; **sensor** cells are push-only streams
  (they read their `default` until an adapter pushes);
* `set`/`push` write a cell and mark every transitive dependent formula
  *stale* (nothing recomputes yet — Excel discipline);
* the next `get` of a stale formula recomputes it from a snapshot of
  its dependencies;
* dependencies are auto-detected by scanning the expression for known
  cell ids as whole tokens (port of `formula.rs::rewrite_known_ids`),
  longest-first so `compass.heading` wins over `compass`.

On top of the Rust engine, this binding wires in the cell ledger
(docs/cell-ledger.md §9's integration path): every successful state
transition — a `set`, a `push`, or a formula recompute — appends a
sealed double entry to that cell's `CellLedger`, projectable onto the
quilt-compat wire edge (§1.5):

* set/push: input posting = output posting = the written value;
* formula recompute: input posting = the dependency snapshot (the dep
  values in dependency-address order), output posting = the result.

`propagation_order(root)` exposes the deterministic topological order
the contract pins for op (c): Kahn's algorithm over the affected
closure, ties broken by lexicographic (UTF-8 byte) address order.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field

from . import formula as _f
from .ledger import CellLedger, Provenance
from .miniyaml import parse_yaml


def now_millis() -> float:
    return time.time() * 1000.0


VALUE = "value"
FORMULA = "formula"
SENSOR = "sensor"

_KNOWN_KINDS = {VALUE, FORMULA, "api", "program", SENSOR, "io", "listener", "router"}


class CellKind:
    VALUE = VALUE
    FORMULA = FORMULA
    SENSOR = SENSOR


@dataclass
class CellDef:
    id: str
    kind: str
    extra: dict = field(default_factory=dict)

    @property
    def value(self):
        return self.extra.get("value")

    @property
    def expr(self) -> str | None:
        return self.extra.get("expr")

    @property
    def default(self):
        return self.extra.get("default")


@dataclass
class SheetDef:
    id: str
    cells: list[CellDef]
    extra: dict = field(default_factory=dict)


@dataclass
class CellValue:
    """What a cell holds: the value plus its own freshness."""

    data: object = None
    status: str = "idle"  # idle | ready | error
    error: str | None = None

    @classmethod
    def ok(cls, data):
        return cls(data=data, status="ready")

    @classmethod
    def err(cls, message):
        return cls(data=None, status="error", error=message)


class Cell:
    __slots__ = ("cdef", "dependencies", "dependents", "value", "stale", "evaluator")

    def __init__(self, cdef: CellDef):
        self.cdef = cdef
        self.dependencies: set[str] = set()
        self.dependents: set[str] = set()
        self.value = None
        self.stale = True
        self.evaluator = None


def _cells_from_doc(raw_cells) -> list[CellDef]:
    if not isinstance(raw_cells, list):
        raise ValueError("`cells` must be a list")
    cells: list[CellDef] = []
    seen: set[str] = set()
    for i, entry in enumerate(raw_cells):
        if not isinstance(entry, dict):
            raise ValueError(f"cell #{i} must be a mapping")
        cid = entry.get("id")
        kind = entry.get("kind")
        if not isinstance(cid, str) or not cid.strip():
            raise ValueError(f"cell #{i} requires a non-empty `id`")
        if cid in seen:
            raise ValueError(f"duplicate cell id: {cid}")
        if kind not in _KNOWN_KINDS:
            raise ValueError(f"cell {cid!r}: unknown kind {kind!r}")
        seen.add(cid)
        if kind == VALUE and "value" not in entry:
            raise ValueError(f"value cell {cid!r} requires `value`")
        if kind == FORMULA and not isinstance(entry.get("expr"), str):
            raise ValueError(f"formula cell {cid!r} requires `expr`")
        cells.append(CellDef(id=cid, kind=kind, extra=entry))
    return cells


def _sheet_from_doc(doc) -> SheetDef:
    if not isinstance(doc, dict):
        raise ValueError("sheet must be a mapping")
    sheet_id = doc.get("id")
    if not isinstance(sheet_id, str) or not sheet_id:
        raise ValueError("sheet requires a top-level `id`")
    cells = _cells_from_doc(doc.get("cells") or [])
    extra = {k: v for k, v in doc.items() if k not in ("id", "cells")}
    return SheetDef(id=sheet_id, cells=cells, extra=extra)


def parse_sheet(source: str) -> SheetDef:
    """Parse quilt-sheet YAML into a SheetDef (validates the core rules)."""
    return _sheet_from_doc(parse_yaml(source))


def sheet_from_dict(doc: dict) -> SheetDef:
    """Build a SheetDef from an already-parsed document (e.g. the golden
    JSON `sheet` section) with the same validation as `parse_sheet`."""
    return _sheet_from_doc(doc)


# ---------------------------------------------------------------------------
# Dependency detection — port of formula.rs::rewrite_known_ids scan
# ---------------------------------------------------------------------------

_BOUNDARY = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_.")


def detect_dependencies(expr: str, known_ids) -> list[str]:
    """Cell ids referenced by `expr`, in first-appearance order.

    Whole-token matching (the char on each side must not be
    alphanumeric/underscore/dot), longest-first per position so
    `compass.heading` matches before `compass`. String literals are
    skipped.
    """
    known = sorted((k for k in known_ids if k), key=len, reverse=True)
    deps: list[str] = []
    seen: set[str] = set()
    i, n = 0, len(expr)
    while i < n:
        ch = expr[i]
        if ch in ("'", '"'):
            i += 1
            while i < n and expr[i] != ch:
                i += 1
            i += 1
            continue
        for kid in known:
            if expr.startswith(kid, i):
                left_ok = i == 0 or expr[i - 1] not in _BOUNDARY
                j = i + len(kid)
                right_ok = j >= n or expr[j] not in _BOUNDARY
                if left_ok and right_ok:
                    if kid not in seen:
                        seen.add(kid)
                        deps.append(kid)
                    i = j
                    break
        else:
            i += 1
    return deps


# ---------------------------------------------------------------------------
# The engine
# ---------------------------------------------------------------------------


class QuiltEngine:
    """A reactive grid of cells with per-cell sealed edge ledgers."""

    def __init__(self, sheet: SheetDef, *, record_edges: bool = True, sheet_id: str | None = None):
        self.sheet = sheet
        self.record_edges = record_edges
        self.cells: dict[str, Cell] = {}
        self.ledgers: dict[str, CellLedger] = {}

        for cdef in sheet.cells:
            cell = Cell(cdef)
            if cdef.kind == FORMULA:
                try:
                    cell.evaluator = _f.compile_expr(cdef.expr or "")
                    deps = detect_dependencies(
                        cdef.expr or "", (c.id for c in sheet.cells)
                    )
                except _f.FormulaError:
                    cell.evaluator = None
                    deps = []
                cell.dependencies = set(deps)
            self.cells[cdef.id] = cell

        # Wire the reverse index (dependents).
        for cell in self.cells.values():
            for dep in cell.dependencies:
                if dep in self.cells:
                    self.cells[dep].dependents.add(cell.cdef.id)

        # Seed initial state + genesis ledgers (ts=0, the sheet's birth).
        for cell in self.cells.values():
            cdef = cell.cdef
            if cdef.kind == VALUE:
                cell.value = CellValue.ok(cdef.value)
                ledger = CellLedger.with_genesis(cdef.id, cdef.value, 0)
            elif cdef.kind == SENSOR:
                cell.value = CellValue.ok(cdef.default)
                if "default" in (cdef.extra or {}):
                    ledger = CellLedger.with_genesis(cdef.id, cdef.default, 0)
                else:
                    ledger = CellLedger(cdef.id)
            elif cdef.kind == FORMULA:
                ledger = CellLedger(cdef.id)  # no genesis: computed later
            else:
                ledger = CellLedger(cdef.id)
            self.ledgers[cdef.id] = ledger

    # -- construction --------------------------------------------------------

    @classmethod
    def from_yaml(cls, source: str, **kwargs) -> "QuiltEngine":
        return cls(parse_sheet(source), **kwargs)

    # -- the universal verbs ---------------------------------------------------

    def get(self, cell_id: str, ts: float | None = None) -> CellValue:
        """Read a cell. A stale formula recomputes here (lazy, like Excel)."""
        cell = self._cell(cell_id)
        if cell.cdef.kind == FORMULA:
            if cell.stale or not isinstance(cell.value, CellValue) or cell.value.status != "ready":
                return self._recompute(cell, ts if ts is not None else now_millis())
            return cell.value
        return cell.value if isinstance(cell.value, CellValue) else CellValue.ok(cell.value)

    def set(self, cell_id: str, value, ts: float | None = None) -> None:
        """Write a cell, mark transitive dependents stale, record the edge."""
        cell = self._cell(cell_id)
        cell.value = CellValue.ok(value)
        cell.stale = False
        if self.record_edges:
            self.ledgers[cell_id].record(
                value, value, ts if ts is not None else now_millis(),
                Provenance("set"),
            )
        self._mark_stale(cell_id)

    def push(self, cell_id: str, value, ts: float | None = None) -> None:
        """Feed a sensor/io cell from an adapter (records a push edge)."""
        cell = self._cell(cell_id)
        if cell.cdef.kind not in (SENSOR, "io"):
            raise ValueError(f"push() is for sensor/io cells, not {cell.cdef.kind}")
        cell.value = CellValue.ok(value)
        cell.stale = False
        if self.record_edges:
            self.ledgers[cell_id].record(
                value, value, ts if ts is not None else now_millis(),
                Provenance("push"),
            )
        self._mark_stale(cell_id)

    # -- graph ------------------------------------------------------------------

    def get_cell(self, cell_id: str) -> Cell | None:
        return self.cells.get(cell_id)

    def dependencies(self, cell_id: str) -> list[str]:
        """Dependency set, sorted (dependency-address order)."""
        return sorted(self._cell(cell_id).dependencies)

    def dependents(self, cell_id: str) -> list[str]:
        return sorted(self._cell(cell_id).dependents)

    def chain_hash(self, cell_id: str) -> str:
        return self.ledgers[cell_id].chain_hash()

    def wire_edges(self, cell_id: str) -> list[dict]:
        """A cell's history projected onto the quilt-compat wire schema."""
        return self.ledgers[cell_id].wire_edges()

    def propagation_order(self, root: str) -> list[str]:
        """The deterministic propagation order for a mutation of `root`
        (quilt-compat op c): Kahn's algorithm over the affected closure,
        ties broken by lexicographic (UTF-8 byte) address order."""
        closure = {root}
        queue = [root]
        while queue:
            cur = queue.pop()
            for dep in sorted(self.cells[cur].dependents):
                if dep not in closure:
                    closure.add(dep)
                    queue.append(dep)

        graph = {cid: sorted(c.dependencies) for cid, c in self.cells.items()}
        indegree = {cid: 0 for cid in closure}
        dependents: dict[str, list[str]] = {cid: [] for cid in closure}
        for cid in closure:
            for dep in graph[cid]:
                if dep in closure:
                    indegree[cid] += 1
                    dependents[dep].append(cid)

        ready = sorted(cid for cid, d in indegree.items() if d == 0)
        order: list[str] = []
        while ready:
            cid = ready.pop(0)
            order.append(cid)
            for dep_id in sorted(dependents[cid]):
                indegree[dep_id] -= 1
                if indegree[dep_id] == 0:
                    ready.append(dep_id)
            ready.sort()
        if len(order) != len(closure):
            raise ValueError("dependency graph has a cycle")
        return order

    # -- internals -----------------------------------------------------------------

    def _cell(self, cell_id: str) -> Cell:
        cell = self.cells.get(cell_id)
        if cell is None:
            raise KeyError(f"cell not found: {cell_id}")
        return cell

    def _mark_stale(self, cell_id: str, seen: set | None = None) -> None:
        """Propagate staleness to transitive dependents (no recompute)."""
        seen = seen if seen is not None else set()
        for dep_id in sorted(self.cells[cell_id].dependents):
            if dep_id in seen:
                continue
            seen.add(dep_id)
            dep = self.cells[dep_id]
            if dep.cdef.kind == FORMULA:
                dep.stale = True
            self._mark_stale(dep_id, seen)

    def _recompute(self, cell: Cell, ts: float) -> CellValue:
        """Evaluate a formula against a snapshot of its dependencies."""
        if cell.evaluator is None:
            err = CellValue.err(f"formula does not compile: {cell.cdef.expr!r}")
            cell.value = err
            cell.stale = False
            return err

        snapshot: dict[str, object] = {}

        def resolve(name: str):
            if name not in snapshot:
                if name not in self.cells:
                    raise _f.FormulaError(f"unknown cell: {name}")
                snapshot[name] = self.get(name, ts).data
            return snapshot[name]

        try:
            data = cell.evaluator(resolve)
            result = CellValue.ok(data)
        except (_f.FormulaError, ZeroDivisionError, KeyError) as exc:
            result = CellValue.err(str(exc))

        cell.value = result
        cell.stale = False

        if self.record_edges and result.status == "ready":
            # Input posting: the dependency snapshot in
            # dependency-address order (sorted ids) — §1.3/§1.5.
            inputs = [snapshot[d] for d in sorted(cell.dependencies) if d in snapshot]
            self.ledgers[cell.cdef.id].record(
                inputs, result.data, ts, Provenance("get"),
            )
        return result
