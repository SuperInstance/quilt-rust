"""quilt_ffi.py — a thin stdlib-only (ctypes) Python binding over the
quilt C ABI (crates/quilt-cabi/quilt_cabi.h).

One native core, N bindings: this module is pure passthrough. It never
reimplements the engine or the ledger — every semantic (formula eval,
reactive propagation, seal computation, reconciliation) happens inside
libquilt_cabi.so and is only marshalled across the ABI.

Memory contract honored here:
  - borrowed `const char *` arguments: we encode to NUL-terminated UTF-8
    bytes for the duration of the call only;
  - returned `char *` strings: decoded to `str`, then released with
    quilt_string_free() inside the same call (never free());
  - engine handles are opaque and caller-owned (context manager).

Library resolution order:
  1. $QUILT_CABI_SO (explicit override);
  2. target/release/libquilt_cabi.so found by walking up from this file
     to the repository root.
"""

from __future__ import annotations

import ctypes
import os
from pathlib import Path
from typing import Optional

__all__ = [
    "ABI_VERSION",
    "REPO_ROOT",
    "LIBRARY_PATH",
    "QuiltError",
    "Engine",
    "abi_version",
    "ledger_init",
    "ledger_record",
    "ledger_verify",
    "ledger_reconcile",
    "ledger_chain_hash",
    "ledgers_reset",
    "string_free",
    "last_error",
]

# Must match QUILT_ABI_VERSION in quilt_cabi.h.
ABI_VERSION = 1


class QuiltError(RuntimeError):
    """A quilt ABI call failed; args[0] carries quilt_last_error()."""


def _find_repo_root_and_so() -> tuple[Optional[Path], Optional[Path]]:
    env = os.environ.get("QUILT_CABI_SO")
    if env:
        so = Path(env).expanduser().resolve()
        if not so.is_file():
            raise FileNotFoundError(f"QUILT_CABI_SO={env} does not exist")
        return so.parent.parent.parent, so
    here = Path(__file__).resolve()
    for d in (here, *here.parents):
        cand = d / "target" / "release" / "libquilt_cabi.so"
        if cand.is_file():
            return d, cand
    return None, None


REPO_ROOT, _so = _find_repo_root_and_so()
if _so is None:
    raise FileNotFoundError(
        "libquilt_cabi.so not found; build it (cargo build -p quilt-cabi "
        "--release) or point QUILT_CABI_SO at the cdylib"
    )
LIBRARY_PATH = _so

_lib = ctypes.CDLL(str(LIBRARY_PATH))

# ---- signatures (argtypes/restype pinned for every symbol) ----------------

_cchar = ctypes.c_char_p  # borrowed args only; never a restype we must free
_pv = ctypes.c_void_p  # returned library-owned strings / opaque handles

_lib.quilt_abi_version.restype = ctypes.c_uint32
_lib.quilt_abi_version.argtypes = []

_lib.quilt_engine_new.restype = _pv
_lib.quilt_engine_new.argtypes = []

_lib.quilt_engine_load_sheet.restype = ctypes.c_int
_lib.quilt_engine_load_sheet.argtypes = [_pv, _cchar]

_lib.quilt_engine_get.restype = _pv
_lib.quilt_engine_get.argtypes = [_pv, _cchar]

_lib.quilt_engine_set.restype = ctypes.c_int
_lib.quilt_engine_set.argtypes = [_pv, _cchar, _cchar]

_lib.quilt_engine_free.restype = None
_lib.quilt_engine_free.argtypes = [_pv]

_lib.quilt_ledger_init.restype = ctypes.c_int
_lib.quilt_ledger_init.argtypes = [_cchar, _cchar, ctypes.c_uint64]

_lib.quilt_ledger_record.restype = _pv
_lib.quilt_ledger_record.argtypes = [_cchar, _cchar, _cchar, ctypes.c_uint64]

_lib.quilt_ledger_verify.restype = ctypes.c_int
_lib.quilt_ledger_verify.argtypes = [_cchar]

_lib.quilt_ledger_reconcile.restype = _pv
_lib.quilt_ledger_reconcile.argtypes = [_cchar]

_lib.quilt_ledger_chain_hash.restype = _pv
_lib.quilt_ledger_chain_hash.argtypes = [_cchar]

_lib.quilt_ledgers_reset.restype = ctypes.c_int
_lib.quilt_ledgers_reset.argtypes = []

_lib.quilt_string_free.restype = None
_lib.quilt_string_free.argtypes = [_pv]

# Borrowed pointer (valid only until the next quilt call on this thread).
_lib.quilt_last_error.restype = _cchar
_lib.quilt_last_error.argtypes = []


# ---- helpers ---------------------------------------------------------------

def last_error() -> str:
    """The last error message from this thread ("" if the call succeeded)."""
    raw = _lib.quilt_last_error()
    return raw.decode("utf-8") if raw is not None else ""


def _utf8(s: str) -> bytes:
    """Borrowed-argument encoding: NUL-terminated UTF-8."""
    return s.encode("utf-8") + b"\x00"


def _take(ptr) -> Optional[str]:
    """Take ownership of a library-returned char*: decode, then free it
    with quilt_string_free (never free()). None stays None."""
    if not ptr:
        return None
    try:
        return ctypes.cast(_pv(ptr), _cchar).value.decode("utf-8")
    finally:
        _lib.quilt_string_free(_pv(ptr))


def string_free(s) -> None:
    """Passthrough to quilt_string_free; tolerates NULL/None."""
    _lib.quilt_string_free(_pv(s) if s is not None else None)


def abi_version() -> int:
    return int(_lib.quilt_abi_version())


def _check_version() -> None:
    got = abi_version()
    if got != ABI_VERSION:
        raise QuiltError(
            f"ABI mismatch: library reports {got}, binding expects {ABI_VERSION}"
        )


_check_version()


# ---- engine ----------------------------------------------------------------

class Engine:
    """Opaque QuiltEngine handle. Use as a context manager for scoped life."""

    def __init__(self) -> None:
        self._handle = _lib.quilt_engine_new()
        if not self._handle:
            raise QuiltError(f"engine_new failed: {last_error()}")

    def load_sheet(self, yaml_text: str) -> None:
        rc = _lib.quilt_engine_load_sheet(self._handle, _utf8(yaml_text))
        if rc != 0:
            raise QuiltError(f"load_sheet failed: {last_error()}")

    def get(self, cell_id: str) -> str:
        """A cell's current value as JSON text (e.g. "80.0", "\"idle\"")."""
        got = _take(_lib.quilt_engine_get(self._handle, _utf8(cell_id)))
        if got is None:
            raise QuiltError(f"get({cell_id}) failed: {last_error()}")
        return got

    def set(self, cell_id: str, value_json: str) -> None:
        """Write a cell (any kind) and propagate downstream."""
        rc = _lib.quilt_engine_set(self._handle, _utf8(cell_id), _utf8(value_json))
        if rc != 0:
            raise QuiltError(f"set({cell_id}) failed: {last_error()}")

    def free(self) -> None:
        if getattr(self, "_handle", None):
            _lib.quilt_engine_free(self._handle)
            self._handle = None

    def __enter__(self) -> "Engine":
        return self

    def __exit__(self, *exc) -> None:
        self.free()

    def __del__(self) -> None:  # best-effort; prefer the context manager
        try:
            self.free()
        except Exception:
            pass


def get_null_engine(cell_id: str) -> Optional[str]:
    """quilt_engine_get(NULL, cell) — an error, not a crash. Test hook."""
    return _take(_lib.quilt_engine_get(None, _utf8(cell_id)))


# ---- ledger (process-global registry, keyed by cell id) --------------------


def ledger_init(cell_id: str, genesis_json: str, ts_millis: int) -> None:
    rc = _lib.quilt_ledger_init(_utf8(cell_id), _utf8(genesis_json), ts_millis)
    if rc != 0:
        raise QuiltError(f"ledger_init({cell_id}) failed: {last_error()}")


def ledger_record(cell_id: str, input_json: str, output_json: str,
                  ts_millis: int) -> str:
    """Record one double entry; returns the entry's seal (64 hex chars)."""
    seal = _take(_lib.quilt_ledger_record(
        _utf8(cell_id), _utf8(input_json), _utf8(output_json), ts_millis))
    if seal is None:
        raise QuiltError(f"ledger_record({cell_id}) failed: {last_error()}")
    return seal


def ledger_verify(cell_id: str) -> int:
    """1 intact, 0 broken, -1 no such ledger (no exception: tri-state ABI)."""
    return int(_lib.quilt_ledger_verify(_utf8(cell_id)))


def ledger_reconcile(cell_id: str) -> str:
    """The Reconciliation report as JSON text."""
    got = _take(_lib.quilt_ledger_reconcile(_utf8(cell_id)))
    if got is None:
        raise QuiltError(f"ledger_reconcile({cell_id}) failed: {last_error()}")
    return got


def ledger_chain_hash(cell_id: str) -> str:
    """The chain head: last entry's seal, or the genesis commit if empty."""
    got = _take(_lib.quilt_ledger_chain_hash(_utf8(cell_id)))
    if got is None:
        raise QuiltError(f"ledger_chain_hash({cell_id}) failed: {last_error()}")
    return got


def ledgers_reset() -> None:
    _lib.quilt_ledgers_reset()
