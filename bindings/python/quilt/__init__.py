"""quilt-py — the Python binding of the quilt cellular runtime (gym tier).

stdlib only. Parses quilt sheets (YAML), evaluates value/formula cells
with the same lazy-reactive semantics as the Rust/TS engines, and
records every cell state change in the quilt-compat contract's edge
schema (docs/quilt-compat-contract.md, compat/golden.json) — sealed
hash-chained double entries projectable onto the wire edge::

    {"v": 1, "cell", "ts", "before", "after", "delta",
     "imbalance", "provenance", "chain", "seq"}
"""

from .miniyaml import parse_yaml, ParseError
from .formula import FormulaError, compile_expr
from .ledger import (
    canonical_json,
    fmt_float,
    sha256_hex,
    value_distance,
    wire_delta,
    wire_imbalance,
    wire_provenance,
    wire_edge,
    Provenance,
    LedgerEntry,
    CellLedger,
)
from .engine import (
    CellKind,
    CellDef,
    SheetDef,
    CellValue,
    QuiltEngine,
    parse_sheet,
    sheet_from_dict,
    detect_dependencies,
)

__version__ = "0.1.0"

__all__ = [
    "parse_yaml",
    "ParseError",
    "FormulaError",
    "compile_expr",
    "canonical_json",
    "fmt_float",
    "sha256_hex",
    "value_distance",
    "wire_delta",
    "wire_imbalance",
    "wire_provenance",
    "wire_edge",
    "Provenance",
    "LedgerEntry",
    "CellLedger",
    "CellKind",
    "CellDef",
    "SheetDef",
    "CellValue",
    "QuiltEngine",
    "parse_sheet",
    "sheet_from_dict",
    "detect_dependencies",
]

