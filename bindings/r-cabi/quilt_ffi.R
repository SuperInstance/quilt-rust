# quilt_ffi.R — thin base-R FFI over the quilt C ABI (libquilt_cabi.so).
#
# The C tier of docs/quilt-compat-contract.md §5, R flavor (docs/c-abi.md
# "binding recipes"): dyn.load the reference engine + ledger and call the
# ABI instead of reimplementing it. No packages, base R only.
#
# WHY THERE IS A COMPILED ADAPTER
# -------------------------------
# Base R has exactly two C calling conventions:
#   .C       passes *addresses of R data* and DISCARDS the C return value,
#            so raw `char *` returns and handle pointers can never come back.
#   .Call    requires SEXP-typed entry points; calling the raw quilt_* symbols
#            would reinterpret a char*/QuiltEngine* as a SEXP (undefined
#            behavior).
# So this file writes a ~90-line SEXP adapter to a temp dir at load time and
# compiles it with base R's own `R CMD SHLIB` (no packages), linking against
# libquilt_cabi.so. The adapter keeps the ABI 1:1: engine handles and
# library-owned strings cross into R as externalptrs; decoding is explicit
# (quilt_read_string) and every decoded pointer is then released with
# quilt_string_free — the header's memory contract, unchanged.
#
# MEMORY CONTRACT (crates/quilt-cabi/quilt_cabi.h)
#   - const char* arguments are borrowed by the library for the call only
#     (R strings are fine).
#   - returned library strings are caller-freed via quilt_string_free, never
#     left dangling: quilt_take() = decode + free in one step.
#   - engine handles are caller-owned (engine_new / engine_free).
#   - errors never unwind: int 0/-1, string NULL; detail in quilt_last_error(),
#     valid until the next quilt call on this thread.

.quilt <- new.env(parent = emptyenv())
.quilt$loaded <- FALSE
.quilt$lib <- NULL
.quilt$dll <- NULL
.quilt$shim <- NULL
.quilt$syms <- new.env(parent = emptyenv())

# ---------------------------------------------------------------------------
# The SEXP adapter, generated to tempdir() and compiled at load time.
# No single quotes appear below (each line is a single-quoted R string).
# ---------------------------------------------------------------------------

.quilt_shim_src <- c(
'/* quilt_cabi_r shim — GENERATED at load time by quilt_ffi.R; do not edit.',
' * Minimal SEXP adapter over libquilt_cabi.so so base R can .Call the ABI:',
' * .C discards C return values and .Call needs SEXP entry points, so this',
' * shim returns engine handles and library strings as externalptrs and lets',
' * the R side decode + quilt_string_free them per the memory contract. */',
'#include <R.h>',
'#include <Rinternals.h>',
'#include <stdint.h>',
'#include "quilt_cabi.h"',
'',
'static QuiltEngine *eng(SEXP x) {',
'  return (QuiltEngine *)(x == R_NilValue ? NULL : R_ExternalPtrAddr(x));',
'}',
'',
'static SEXP wrap_str(char *s) {',
'  if (!s) return R_NilValue;',
'  return R_MakeExternalPtr((void *)s, Rf_install("quilt_string"), R_NilValue);',
'}',
'',
'static const char *cstr(SEXP s) {',
'  return CHAR(STRING_ELT(s, 0));',
'}',
'',
'static uint64_t as_u64(SEXP x) {',
'  if (TYPEOF(x) == INTSXP || TYPEOF(x) == LGLSXP) return (uint64_t)INTEGER(x)[0];',
'  return (uint64_t)REAL(x)[0];',
'}',
'',
'SEXP r_quilt_abi_version(void) {',
'  return Rf_ScalarInteger((int)quilt_abi_version());',
'}',
'',
'SEXP r_quilt_engine_new(void) {',
'  QuiltEngine *e = quilt_engine_new();',
'  if (!e) return R_NilValue;',
'  return R_MakeExternalPtr((void *)e, Rf_install("quilt_engine"), R_NilValue);',
'}',
'',
'SEXP r_quilt_engine_load_sheet(SEXP engine, SEXP yaml) {',
'  return Rf_ScalarInteger(quilt_engine_load_sheet(eng(engine), cstr(yaml)));',
'}',
'',
'SEXP r_quilt_engine_get(SEXP engine, SEXP cell) {',
'  return wrap_str(quilt_engine_get(eng(engine), cstr(cell)));',
'}',
'',
'SEXP r_quilt_engine_set(SEXP engine, SEXP cell, SEXP value) {',
'  return Rf_ScalarInteger(quilt_engine_set(eng(engine), cstr(cell), cstr(value)));',
'}',
'',
'SEXP r_quilt_engine_free(SEXP engine) {',
'  quilt_engine_free(eng(engine));',
'  return R_NilValue;',
'}',
'',
'SEXP r_quilt_ledger_init(SEXP cell, SEXP genesis, SEXP ts) {',
'  return Rf_ScalarInteger(quilt_ledger_init(cstr(cell), cstr(genesis), as_u64(ts)));',
'}',
'',
'SEXP r_quilt_ledger_record(SEXP cell, SEXP input, SEXP output, SEXP ts) {',
'  return wrap_str(quilt_ledger_record(cstr(cell), cstr(input), cstr(output), as_u64(ts)));',
'}',
'',
'SEXP r_quilt_ledger_verify(SEXP cell) {',
'  return Rf_ScalarInteger(quilt_ledger_verify(cstr(cell)));',
'}',
'',
'SEXP r_quilt_ledger_reconcile(SEXP cell) {',
'  return wrap_str(quilt_ledger_reconcile(cstr(cell)));',
'}',
'',
'SEXP r_quilt_ledger_chain_hash(SEXP cell) {',
'  return wrap_str(quilt_ledger_chain_hash(cstr(cell)));',
'}',
'',
'SEXP r_quilt_ledgers_reset(void) {',
'  return Rf_ScalarInteger(quilt_ledgers_reset());',
'}',
'',
'SEXP r_quilt_string_free(SEXP ptr) {',
'  if (ptr != R_NilValue)',
'    quilt_string_free((char *)R_ExternalPtrAddr(ptr));',
'  return R_NilValue;',
'}',
'',
'/* Copy a library-owned C string into an R string (the decode step); the',
' * caller then releases the pointer with r_quilt_string_free. */',
'SEXP r_quilt_read_string(SEXP ptr) {',
'  const char *s;',
'  if (ptr == R_NilValue) return R_NilValue;',
'  s = (const char *)R_ExternalPtrAddr(ptr);',
'  if (!s) return R_NilValue;',
'  return Rf_ScalarString(Rf_mkChar(s));',
'}',
'',
'SEXP r_quilt_last_error(void) {',
'  return Rf_ScalarString(Rf_mkChar(quilt_last_error()));',
'}'
)

# ---------------------------------------------------------------------------
# Loading: find the cdylib, dyn.load it, build + load the adapter.
# ---------------------------------------------------------------------------

# libquilt_cabi.so search order: explicit hint, $QUILT_CABI_LIB, then an
# upward walk from the working directory (target/release, then target/debug).
.quilt_find_lib <- function(hint = NULL) {
  cands <- character(0)
  if (is.character(hint) && length(hint) && nzchar(hint[1])) cands <- hint[1]
  env <- Sys.getenv("QUILT_CABI_LIB", "")
  if (nzchar(env)) cands <- c(cands, env)
  d <- normalizePath(getwd(), mustWork = FALSE)
  for (i in seq_len(40)) {
    cands <- c(cands, file.path(d, "target", "release", "libquilt_cabi.so"),
                     file.path(d, "target", "debug", "libquilt_cabi.so"))
    d2 <- dirname(d)
    if (identical(d2, d)) break
    d <- d2
  }
  hit <- cands[file.exists(cands)]
  if (!length(hit))
    stop("libquilt_cabi.so not found; pass lib= or set $QUILT_CABI_LIB")
  normalizePath(hit[1], mustWork = TRUE)
}

# Compile the adapter. R's Makeconf may pin a compiler name that does not
# exist on this PATH (conda R wants x86_64-conda-linux-gnu-cc, the system
# only ships cc), so bridge to a working compiler through a temp bin dir
# when needed. Everything is base R: R CMD SHLIB + system2.
.quilt_build_shim <- function(lib) {
  rbin <- file.path(R.home("bin"), "R")
  cc <- Sys.getenv("CC", "")
  if (!nzchar(cc)) {
    cc <- suppressWarnings(
      system2(rbin, c("CMD", "config", "CC"), stdout = TRUE, stderr = FALSE))
    cc <- if (length(cc)) trimws(cc[1]) else ""
  }
  if (!nzchar(cc)) cc <- "cc"
  cc_tokens <- strsplit(trimws(cc), "[[:space:]]+")[[1]]
  real <- if (nzchar(Sys.which(cc_tokens[1]))) cc_tokens[1] else {
    hit <- Sys.which(c("cc", "gcc", "clang"))
    hit <- hit[nzchar(hit)]
    if (!length(hit)) stop("no C compiler found (cc/gcc/clang) for R CMD SHLIB")
    hit[1]
  }
  tool_dir <- file.path(tempdir(), "quilt_cabi_r", "toolchain")
  dir.create(tool_dir, showWarnings = FALSE, recursive = TRUE)
  if (!identical(cc_tokens[1], real)) {
    wrap <- file.path(tool_dir, cc_tokens[1])
    if (!file.exists(wrap)) {
      writeLines(c("#!/bin/sh",
                   paste0(c("exec", real, cc_tokens[-1], '"$@"'), collapse = " ")),
                 wrap)
      Sys.chmod(wrap, "0755")
    }
    old_path <- Sys.getenv("PATH")
    Sys.setenv(PATH = paste(tool_dir, old_path, sep = .Platform$path.sep))
    on.exit(Sys.setenv(PATH = old_path), add = TRUE)
  }
  root <- dirname(dirname(dirname(lib)))          # <root>/target/release/x.so
  hdr_dir <- file.path(root, "crates", "quilt-cabi")
  if (!file.exists(file.path(hdr_dir, "quilt_cabi.h")))
    stop("quilt_cabi.h not found under ", hdr_dir)
  out_dir <- file.path(tempdir(), "quilt_cabi_r")
  dir.create(out_dir, showWarnings = FALSE, recursive = TRUE)
  src <- file.path(out_dir, "quilt_cabi_r.c")
  so <- file.path(out_dir, "quilt_cabi_r.so")
  writeLines(.quilt_shim_src, src)
  libdir <- dirname(lib)
  old_cflags <- Sys.getenv("PKG_CFLAGS", "")
  old_libs <- Sys.getenv("PKG_LIBS", "")
  Sys.setenv(PKG_CFLAGS = paste0("-I", hdr_dir),
             PKG_LIBS = sprintf("-L%s -lquilt_cabi -Wl,-rpath,%s", libdir, libdir))
  on.exit({
    if (nzchar(old_cflags)) Sys.setenv(PKG_CFLAGS = old_cflags) else Sys.unsetenv("PKG_CFLAGS")
    if (nzchar(old_libs)) Sys.setenv(PKG_LIBS = old_libs) else Sys.unsetenv("PKG_LIBS")
  }, add = TRUE)
  log <- suppressWarnings(system2(rbin,
                                  c("CMD", "SHLIB", "-o", shQuote(so), shQuote(src)),
                                  stdout = TRUE, stderr = TRUE))
  st <- attr(log, "status")
  if (!is.null(st) && !identical(as.integer(st), 0L))
    stop("R CMD SHLIB failed:\n", paste(log, collapse = "\n"))
  so
}

# dyn.load libquilt_cabi.so, build the adapter, resolve every r_* routine,
# and check the ABI version against quilt_cabi.h's QUILT_ABI_VERSION (1).
quilt_cabi_load <- function(lib = NULL) {
  if (.quilt$loaded) return(invisible(TRUE))
  lib <- .quilt_find_lib(lib)
  .quilt$lib <- lib
  .quilt$dll <- dyn.load(lib, local = FALSE)  # global: shim links against it
  if (!inherits(getNativeSymbolInfo("quilt_abi_version", .quilt$dll),
                "NativeSymbolInfo"))
    stop("libquilt_cabi.so loaded but quilt_abi_version is missing")
  .quilt$shim_path <- .quilt_build_shim(lib)
  .quilt$shim <- dyn.load(.quilt$shim_path)
  nms <- c("r_quilt_abi_version", "r_quilt_engine_new",
           "r_quilt_engine_load_sheet", "r_quilt_engine_get",
           "r_quilt_engine_set", "r_quilt_engine_free",
           "r_quilt_ledger_init", "r_quilt_ledger_record",
           "r_quilt_ledger_verify", "r_quilt_ledger_reconcile",
           "r_quilt_ledger_chain_hash", "r_quilt_ledgers_reset",
           "r_quilt_string_free", "r_quilt_read_string", "r_quilt_last_error")
  for (n in nms) .quilt$syms[[n]] <- getNativeSymbolInfo(n, .quilt$shim)
  ver <- .Call(.quilt$syms[["r_quilt_abi_version"]])
  if (!identical(ver, 1L))
    stop("ABI version mismatch: library reports ", ver, ", quilt_cabi.h pins 1")
  .quilt$loaded <- TRUE
  invisible(TRUE)
}

quilt_cabi_is_loaded <- function() .quilt$loaded

.qc <- function(name, ...) {
  if (!.quilt$loaded) stop("quilt FFI not loaded; call quilt_cabi_load() first")
  .Call(.quilt$syms[[name]], ...)
}

.s1 <- function(x) { x <- as.character(x); if (length(x) < 1L || is.na(x[1])) "" else x[1] }

# ---------------------------------------------------------------------------
# The ABI, 1:1 (crates/quilt-cabi/quilt_cabi.h). String-returning functions
# hand back an externalptr (NULL on error, like the C NULL); decode with
# quilt_read_string + quilt_string_free, or quilt_take for both at once.
# ---------------------------------------------------------------------------

quilt_abi_version <- function() .qc("r_quilt_abi_version")

quilt_engine_new <- function() .qc("r_quilt_engine_new")

quilt_engine_load_sheet <- function(engine, yaml)
  .qc("r_quilt_engine_load_sheet", engine, .s1(yaml))

quilt_engine_get <- function(engine, cell)
  .qc("r_quilt_engine_get", engine, .s1(cell))

quilt_engine_set <- function(engine, cell, value_json)
  .qc("r_quilt_engine_set", engine, .s1(cell), .s1(value_json))

quilt_engine_free <- function(engine) invisible(.qc("r_quilt_engine_free", engine))

quilt_ledger_init <- function(cell, genesis_json, ts_millis)
  .qc("r_quilt_ledger_init", .s1(cell), .s1(genesis_json), as.numeric(ts_millis)[1])

quilt_ledger_record <- function(cell, input_json, output_json, ts_millis)
  .qc("r_quilt_ledger_record", .s1(cell), .s1(input_json), .s1(output_json),
      as.numeric(ts_millis)[1])

quilt_ledger_verify <- function(cell) .qc("r_quilt_ledger_verify", .s1(cell))

quilt_ledger_reconcile <- function(cell) .qc("r_quilt_ledger_reconcile", .s1(cell))

quilt_ledger_chain_hash <- function(cell) .qc("r_quilt_ledger_chain_hash", .s1(cell))

quilt_ledgers_reset <- function() .qc("r_quilt_ledgers_reset")

# Free a library-returned string pointer. NULL / NULL-pointer is a no-op,
# exactly like quilt_string_free(NULL).
quilt_string_free <- function(ptr) invisible(.qc("r_quilt_string_free", ptr))

# Decode: copy the C string into an R string. The pointer stays valid until
# quilt_string_free; read it before freeing.
quilt_read_string <- function(ptr) .qc("r_quilt_read_string", ptr)

# The idiomatic pair: decode then quilt_string_free.
quilt_take <- function(ptr) {
  s <- quilt_read_string(ptr)
  quilt_string_free(ptr)
  s
}

# The last error message from this thread's most recent quilt call ("" if it
# succeeded). Borrowed by the library — the shim copies it into R memory.
quilt_last_error <- function() .qc("r_quilt_last_error")
