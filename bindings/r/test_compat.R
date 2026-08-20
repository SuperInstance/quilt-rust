#!/usr/bin/env Rscript
# test_compat.R — quilt-r conformance harness (the R tier's proof).
#
# Mirrors compat/conformance_test.rs against the normative compat/golden.json,
# at the R tier's declared conformance class (quilt-compat-contract.md section 4):
#
#     (a) value read      exact
#     (b) formula eval    1e-12
#     (c) propagation     exact (ordered list)
#     (d) edge            1e-9   (dyadic golden vectors hold exactly)
#     (e) chain hashes    bit-for-bit
#     (e') reconcile      1e-6   (holds exactly here)
#
# Run:  Rscript bindings/r/test_compat.R     (from the repo root)
# Exit:  0 = PASS, 1 = FAIL. Prints PASS/FAIL per op + the golden numbers.
#
# The golden file is parsed with a minimal base-R JSON parser (jsonlite is
# used if installed and compatible, but the built-in parser is preferred:
# the int/float distinction is part of the hash preimage, and the parser
# tags it exactly — 2000 vs 2000.0 serialize differently on-chain).

# ---------------------------------------------------------------------------
# Minimal JSON parser — tags ints (attr "jint") vs floats, preserves order
# ---------------------------------------------------------------------------

parse_json <- function(text) {
  st <- new.env(parent = emptyenv())
  st$es <- strsplit(text, "", fixed = TRUE)[[1]]
  st$i <- 1L
  st$n <- length(st$es)
  .j_ws <- function() {
    while (st$i <= st$n && st$es[st$i] %in% c(" ", "\t", "\n", "\r")) st$i <- st$i + 1L
  }
  .lit <- function(word) {
    if (st$i + nchar(word) - 1L <= st$n &&
        paste0(st$es[st$i:(st$i + nchar(word) - 1L)], collapse = "") == word) {
      st$i <- st$i + nchar(word)
      TRUE
    } else FALSE
  }
  .string <- function() {
    st$i <- st$i + 1L
    out <- character(0)
    while (st$i <= st$n) {
      ch <- st$es[st$i]
      if (ch == '"') { st$i <- st$i + 1L; return(paste0(out, collapse = "")) }
      if (ch == "\\") {
        esc <- st$es[st$i + 1L]
        st$i <- st$i + 2L
        val <- switch(esc,
                      '"' = '"', "\\" = "\\", "/" = "/",
                      "b" = "\b", "f" = "\f", "n" = "\n", "r" = "\r", "t" = "\t",
                      "u" = {
                        hex <- paste0(st$es[st$i:(st$i + 3L)], collapse = "")
                        st$i <- st$i + 4L
                        intToUtf8(strtoi(hex, 16L))
                      },
                      stop("bad JSON escape: ", esc, call. = FALSE))
        out <- c(out, val)
      } else {
        out <- c(out, ch)
        st$i <- st$i + 1L
      }
    }
    stop("unterminated JSON string", call. = FALSE)
  }
  .number <- function() {
    start <- st$i
    numchars <- c("-", "+", ".", "e", "E", strsplit("0123456789", "", fixed = TRUE)[[1]])
    while (st$i <= st$n && st$es[st$i] %in% numchars) st$i <- st$i + 1L
    s <- paste0(st$es[start:(st$i - 1L)], collapse = "")
    if (!nzchar(s)) stop("bad JSON number at ", start, call. = FALSE)
    v <- as.numeric(s)
    if (is.na(v)) stop("bad JSON number: ", s, call. = FALSE)
    if (grepl("[.eE]", s)) v else jint(v)
  }
  .value <- function() {
    ch <- st$es[st$i]
    if (ch == "{") return(.object())
    if (ch == "[") return(.array())
    if (ch == '"') return(.string())
    if (ch == "t") { if (.lit("true")) return(TRUE); stop("bad JSON literal", call. = FALSE) }
    if (ch == "f") { if (.lit("false")) return(FALSE); stop("bad JSON literal", call. = FALSE) }
    if (ch == "n") { if (.lit("null")) return(NA); stop("bad JSON literal", call. = FALSE) }
    .number()
  }
  .object <- function() {
    st$i <- st$i + 1L
    .j_ws()
    obj <- list()
    nms <- character(0)
    if (st$i <= st$n && st$es[st$i] == "}") { st$i <- st$i + 1L; return(obj) }
    repeat {
      .j_ws()
      key <- .string()
      .j_ws()
      if (st$i > st$n || st$es[st$i] != ":") stop("expected ':' in JSON object", call. = FALSE)
      st$i <- st$i + 1L
      .j_ws()
      if (key %in% nms) obj[[key]] <- .value()
      else { obj <- c(obj, list(.value())); nms <- c(nms, key) }
      .j_ws()
      ch <- st$es[st$i]
      if (ch == ",") { st$i <- st$i + 1L; next }
      if (ch == "}") { st$i <- st$i + 1L; break }
      stop("bad JSON object", call. = FALSE)
    }
    names(obj) <- nms
    obj
  }
  .array <- function() {
    st$i <- st$i + 1L
    .j_ws()
    arr <- list()
    if (st$i <= st$n && st$es[st$i] == "]") { st$i <- st$i + 1L; return(arr) }
    repeat {
      .j_ws()
      arr[[length(arr) + 1L]] <- .value()
      .j_ws()
      ch <- st$es[st$i]
      if (ch == ",") { st$i <- st$i + 1L; next }
      if (ch == "]") { st$i <- st$i + 1L; break }
      stop("bad JSON array", call. = FALSE)
    }
    arr
  }
  .j_ws()
  v <- .value()
  .j_ws()
  if (st$i <= st$n) stop("trailing JSON content at ", st$i, call. = FALSE)
  v
}

# ---------------------------------------------------------------------------
# Locate golden.json, source the tier, load the contract
# ---------------------------------------------------------------------------

.args <- commandArgs(trailingOnly = FALSE)
.file_arg <- grep("^--file=", .args, value = TRUE)
here <- if (length(.file_arg)) dirname(normalizePath(sub("^--file=", "", .file_arg[1]))) else getwd()

source(file.path(here, "quilt.R"))

.candidates <- c(
  file.path(normalizePath(file.path(here, "..", "..")), "compat", "golden.json"),
  file.path(getwd(), "compat", "golden.json"),
  file.path(getwd(), "..", "..", "compat", "golden.json")
)
golden_path <- .candidates[file.exists(.candidates)][1]
if (is.na(golden_path)) stop("golden.json not found (looked in: ",
                             paste(.candidates, collapse = "; "), ")")
golden <- parse_json(paste0(readLines(golden_path, warn = FALSE), collapse = "\n"))

if (!identical(golden$contract, "quilt-compat/1"))
  stop("golden.json contract '", golden$contract, "' != 'quilt-compat/1' — ",
       "this tier implements quilt-compat/1; fail loudly, never guess (section 7).")
if (!(is_jnum(golden$spec$edge_schema_v) && as.numeric(golden$spec$edge_schema_v) == 1))
  stop("golden.json edge_schema_v != 1 — refusing to guess (section 7).")

# The R tier's declared conformance class (contract section 4).
TOL_FORMULA <- 1e-12
TOL_EDGE <- 1e-9
TOL_RECONCILE <- 1e-6

# ---------------------------------------------------------------------------
# Check framework
# ---------------------------------------------------------------------------

STATE <- new.env(parent = emptyenv())
STATE$failures <- character(0)

check <- function(label, cond) {
  if (!isTRUE(cond)) STATE$failures <- c(STATE$failures, label)
  invisible(isTRUE(cond))
}

# value printer: canonical JSON rendering (exact, deterministic)
pv <- function(v) canonical_json(v)

# Python-harness assert_close: numeric closeness with exact fallback.
.jc <- function(got, want, tol) {
  if (is_jnum(got) && is_jnum(want)) return(abs(got - want) <= tol)
  if (is.list(got) && is.list(want)) {
    if (is_jobj(got) && is_jobj(want)) {
      if (!setequal(names(got), names(want))) return(FALSE)
      for (k in names(want)) if (!.jc(got[[k]], want[[k]], tol)) return(FALSE)
      return(TRUE)
    }
    if (length(got) != length(want)) return(FALSE)
    if (length(want) == 0L) return(TRUE)
    for (i in seq_along(want)) if (!.jc(got[[i]], want[[i]], tol)) return(FALSE)
    return(TRUE)
  }
  if ((is.null(got) || is_json_null(got)) && (is.null(want) || is_json_null(want))) return(TRUE)
  identical(got, want)
}

expect_close <- function(what, got, want, tol) {
  ok <- tryCatch(.jc(got, want, tol), error = function(e) FALSE)
  if (!ok)
    STATE$failures <- c(STATE$failures,
                        sprintf("%s: got %s, want %s (tol %g)",
                                what, pv(if (is.null(got)) NA else got), pv(want), tol))
  invisible(ok)
}

expect_sha256_hex <- function(what, got, want) {
  ok <- is_jstr(got) && nchar(got) == 64L && grepl("^[0-9a-f]{64}$", got) && identical(got, want)
  if (!ok)
    STATE$failures <- c(STATE$failures, sprintf("%s: must be bit-for-bit; got %s, want %s",
                                                what, got, want))
  invisible(ok)
}

# JSON array of strings -> R character vector
vc <- function(l) if (is.null(l) || (is.list(l) && !length(l))) character(0) else as.character(unlist(l))

fresh_engine <- function() QuiltEngine(sheet_from_dict(golden$sheet))

run_section <- function(label, f) {
  before <- length(STATE$failures)
  errored <- FALSE
  tryCatch(f(), error = function(e) {
    errored <<- TRUE
    STATE$failures <- c(STATE$failures, sprintf("%s: ERROR: %s", label, conditionMessage(e)))
  })
  n_after <- length(STATE$failures)
  pass <- !errored && n_after == before
  cat(sprintf("  %-36s %s\n", label, if (pass) "PASS" else "FAIL"))
  if (n_after > before)
    for (m in STATE$failures[(before + 1L):n_after]) cat("      - ", m, "\n", sep = "")
  pass
}

# ---------------------------------------------------------------------------
# The five core ops
# ---------------------------------------------------------------------------

op_a <- function() {
  engine <- fresh_engine()
  for (v in golden$op_a_value_read) {
    got <- engine$get(v$cell)
    check(sprintf("(a) %s status ready", v$cell), identical(got$status, "ready"))
    expect_close(sprintf("(a) value read %s", v$cell), got$data, v$expect, 0)
  }
}

op_b <- function() {
  engine <- fresh_engine()
  sec <- golden$op_b_formula_eval
  for (v in sec$initial)
    expect_close(sprintf("(b) formula %s (initial)", v$cell),
                 engine$get(v$cell)$data, v$expect, TOL_FORMULA)
  push <- sec$after_push
  engine$push(push$cell, push$value)
  for (v in sec$post)
    expect_close(sprintf("(b) formula %s (post-push)", v$cell),
                 engine$get(v$cell)$data, v$expect, TOL_FORMULA)
}

op_c <- function() {
  g <- golden$graph
  sec <- golden$op_c_propagation
  root <- sec$mutate$cell
  engine <- fresh_engine()
  order <- engine$propagation_order(root)
  check("(c) propagation order is the deterministic topo order",
        identical(order, vc(sec$expected_order)))
  for (cid in names(g)) {
    check(sprintf("(c) engine deps of %s match golden graph", cid),
          identical(engine$dependencies(cid), vc(g[[cid]])))
  }
  for (cell in names(sec$engine_dependency_graph_must_match)) {
    check(sprintf("(c) must-match deps of %s", cell),
          identical(engine$dependencies(cell), vc(sec$engine_dependency_graph_must_match[[cell]])))
  }
  engine$push(root, sec$mutate$value)
  expect_close("(c) post-mutation read", engine$get("bilge.level")$data, 85.0, 0)
}

op_d <- function() {
  for (v in golden$op_d_edge) {
    name <- v$name
    expect_close(sprintf("(d) edge %s delta", name),
                 wire_delta(v$before, v$after), v$expect$delta, TOL_EDGE)
    expect_close(sprintf("(d) edge %s imbalance", name),
                 wire_imbalance(v$before, v$after), v$expect$imbalance, TOL_EDGE)
    expect_sha256_hex(sprintf("(d) edge %s provenance", name),
                      wire_provenance(v$inputs), v$expect$provenance)
  }
  # Full wire-edge record shape (mirror of the python tier's check).
  edge <- wire_edge("x", 1000.0, 40.0, 85.0, list(85.0), paste0("ab", 32L), seq = jint(1))
  expect_close("(d) wire_edge v", edge$v, jint(1), 0)
  expect_close("(d) wire_edge ts", edge$ts, 1000.0, 0)
  expect_close("(d) wire_edge delta", edge$delta, 45.0, 0)
  expect_close("(d) wire_edge imbalance", edge$imbalance, 45.0, 0)
  expect_sha256_hex("(d) wire_edge provenance", edge$provenance, sha256_hex("[85.0]"))
  # Non-numeric edges: recorded as having happened, not faked.
  expect_close("(d) string edge delta is null", wire_delta("idle", "running"), NA, 0)
  expect_close("(d) null-prior edge delta is null", wire_delta(NA, 7.0), NA, 0)
}

op_e <- function() {
  sec <- golden$op_e_chain
  t <- sec$transcript
  ledger <- CellLedger_with_genesis(t$cell, t$genesis, t$genesis_ts)
  for (rec in t$records) ledger$record(rec$input, rec$output, rec$ts)

  entries <- ledger$entries
  for (i in seq_along(sec$entries)) {
    e <- entries[[i]]
    w <- sec$entries[[i]]
    check(sprintf("(e) entry %d seq contiguous from 1", i), identical(as.numeric(e$seq), as.numeric(w$seq)))
    expect_sha256_hex(sprintf("(e) entry %d prev_hash", i), e$prev_hash, w$prev_hash)
    expect_sha256_hex(sprintf("(e) entry %d seal", i), e$hash, w$hash)
  }
  expect_sha256_hex("(e) chain_hash (head)", ledger$chain_hash(), sec$chain_hash)

  rep <- ledger$reconcile()
  w <- sec$reconcile
  check("(e) reconcile cell_id", identical(rep$cell_id, t$cell))
  check("(e) reconcile entries", identical(as.numeric(rep$entries), as.numeric(w$entries)))
  check("(e) reconcile open_inputs", identical(as.numeric(rep$open_inputs), as.numeric(w$open_inputs)))
  check("(e) reconcile matched_pairs", identical(as.numeric(rep$matched_pairs), as.numeric(w$matched_pairs)))
  check("(e) reconcile chain_intact", identical(rep$chain_intact, w$chain_intact))
  check("(e) reconcile continuity_intact", identical(rep$continuity_intact, w$continuity_intact))
  check("(e) reconcile balanced", identical(rep$balanced, w$balanced))
  expect_close("(e) total_surprise", rep$total_surprise, w$total_surprise, TOL_RECONCILE)
  expect_close("(e) mean_surprise", rep$mean_surprise, w$mean_surprise, TOL_RECONCILE)
}

# ---------------------------------------------------------------------------
# Supporting tiers (the section 2 pins every bit-for-bit claim stands on)
# ---------------------------------------------------------------------------

sup_canonical <- function() {
  check("canonical compact + sorted keys",
        identical(canonical_json(list(b = jint(1), a = list(2.5, TRUE, NA, "x"))),
                  '{"a":[2.5,true,null,"x"],"b":1}'))
  check("int/float distinction in the preimage", identical(canonical_json(jint(85)), "85") &&
        identical(canonical_json(85), "85.0") &&
        identical(canonical_json(2.5), "2.5") &&
        identical(canonical_json(45.0), "45.0") &&
        identical(canonical_json(0.1875), "0.1875"))
  check("float exponents normalized to ryu form",
        identical(canonical_json(1e-5), "1e-5") &&
        identical(canonical_json(1e16), "1e16"))
  check("insertion order irrelevant",
        identical(canonical_json(list(x = jint(1), y = jint(2))),
                  canonical_json(list(y = jint(2), x = jint(1)))))
}

sup_sha256 <- function() {
  expect_sha256_hex("sha256 empty vector", sha256_hex(""),
                    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
  expect_sha256_hex("sha256 abc vector", sha256_hex("abc"),
                    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
}

sup_distance <- function() {
  cases <- list(
    list(3.0, 5.0, 2.0),
    list(TRUE, TRUE, 0.0),
    list(FALSE, jint(0), 1.0),                       # type shift: full surprise
    list(list(1.0, 2.0), list(3.0, 2.0), 1.0),       # mean of element-wise
    list(list(1.0), list(1.0, 5.0), 0.5),            # missing element costs 1.0
    list(list(a = jint(1)), list(a = jint(1), b = jint(2)), 0.5),  # missing key costs 1.0
    list(40, 40.0, 0.0)                              # int vs float: magnitude 0
  )
  for (i in seq_along(cases)) {
    a <- cases[[i]][[1]]; b <- cases[[i]][[2]]; want <- cases[[i]][[3]]
    expect_close(sprintf("value_distance case %d", i), value_distance(a, b), want, 1e-12)
  }
  check("int-vs-float edge is changed with magnitude 0 (serde eq is typed)",
        !serde_eq(jint(40), 40.0) && value_distance(jint(40), 40.0) == 0)
}

GOLDEN_YAML <- paste(
  "id: bilge-reflex",
  "description: The golden sheet (YAML round-trip of the contract sheet)",
  "cells:",
  "  - id: bilge.level",
  "    kind: sensor",
  "    source: simulated",
  "    default: 40.0",
  "  - id: bilge.threshold",
  "    kind: value",
  "    value: 80.0",
  "  - id: pump.should_run",
  "    kind: formula",
  '    expr: "=bilge.level >= bilge.threshold"',
  "  - id: pump.relay_cmd",
  "    kind: formula",
  '    expr: "=clamp((bilge.level - bilge.threshold) * 0.5, -30.0, 30.0)"',
  "  - id: status",
  "    kind: value",
  "    value: idle",
  sep = "\n")

sup_yaml <- function() {
  sheet <- parse_sheet(GOLDEN_YAML)
  ref <- sheet_from_dict(golden$sheet)
  check("yaml id equals golden sheet id", identical(sheet$id, ref$id))
  check("yaml cell ids equal golden", identical(vapply(sheet$cells, function(c) c$id, ""), vapply(ref$cells, function(c) c$id, "")))
  check("yaml cell kinds equal golden", identical(vapply(sheet$cells, function(c) c$kind, ""), vapply(ref$cells, function(c) c$kind, "")))
  for (i in seq_along(sheet$cells)) {
    a <- sheet$cells[[i]]$extra
    b <- ref$cells[[i]]$extra
    expect_close(sprintf("yaml value of %s", sheet$cells[[i]]$id), a$value, b$value, 0)
    expect_close(sprintf("yaml expr of %s", sheet$cells[[i]]$id), a$expr, b$expr, 0)
    expect_close(sprintf("yaml default of %s", sheet$cells[[i]]$id), a$default, b$default, 0)
  }
  e <- QuiltEngine(sheet)
  expect_close("(b spot) yaml-engine should_run initial", e$get("pump.should_run")$data, FALSE, 0)
  expect_close("(b spot) yaml-engine relay_cmd initial", e$get("pump.relay_cmd")$data, -20.0, 0)
  check("yaml-engine rejects duplicate ids",
        inherits(tryCatch(parse_sheet(paste0(
          "id: dup\ncells:\n  - id: a\n    kind: value\n    value: 1\n",
          "  - id: a\n    kind: value\n    value: 2\n")),
          error = function(e) e), "error"))
  check("yaml-engine rejects value cell without value",
        inherits(tryCatch(parse_sheet("id: bad\ncells:\n  - id: x\n    kind: value\n"),
                          error = function(e) e), "error"))
}

sup_deps <- function() {
  ids <- c("bilge.level", "bilge.threshold", "pump.relay_cmd")
  check("golden clamp expr deps",
        identical(detect_dependencies(
          "=clamp((bilge.level - bilge.threshold) * 0.5, -30.0, 30.0)", ids),
          c("bilge.level", "bilge.threshold")))
  check("longest id wins",
        identical(detect_dependencies("=compass.heading > 10 ? a : b",
                                      c("compass", "compass.heading", "a", "b")),
                  c("compass.heading", "a", "b")))
  check("string literals are not references",
        identical(detect_dependencies("='temp' + temp", "temp"), "temp"))
}

sup_integration <- function() {
  engine <- fresh_engine()
  engine$push("bilge.level", 85.0, ts = 2000.0)
  edges <- engine$wire_edges("bilge.level")
  check("push recorded exactly one edge", length(edges) == 1L)
  e <- edges[[1]]
  expect_close("edge before", e$before, 40.0, 0)
  expect_close("edge after", e$after, 85.0, 0)
  expect_close("edge delta", e$delta, 45.0, 0)
  expect_close("edge imbalance", e$imbalance, 45.0, 0)
  expect_sha256_hex("edge provenance", e$provenance, wire_provenance(list(85.0)))
  expect_sha256_hex("edge chain = genesis commit", e$chain,
                    engine$ledgers[["bilge.level"]]$genesis_commit())
  expect_close("edge ts", e$ts, 2000.0, 0)
  expect_close("edge v", e$v, jint(1), 0)
  expect_close("edge seq", e$seq, jint(1), 0)

  engine2 <- fresh_engine()
  engine2$push("bilge.level", 85.0, ts = 2000.0)
  engine2$get("pump.relay_cmd", ts = 2001.0)
  head_e <- engine2$ledgers[["pump.relay_cmd"]]$head()
  expect_close("formula recompute posts dep snapshot in dep-address order",
               head_e$input$value, list(85.0, 80.0), 0)
  expect_close("formula recompute output", head_e$output$value, 2.5, 0)
  check("formula recompute provenance origin is get",
        identical(head_e$provenance$origin, "get"))

  engine3 <- fresh_engine()
  engine3$push("bilge.level", 85.0, ts = 2000.0)
  for (cell in c("pump.should_run", "pump.relay_cmd")) engine3$get(cell, ts = 2001.0)
  for (cid in names(engine3$ledgers)) {
    check(sprintf("books balance for %s", cid),
          isTRUE(engine3$ledgers[[cid]]$reconcile()$balanced) &&
          isTRUE(engine3$ledgers[[cid]]$verify_chain()$intact))
  }
}

sup_tamper_settle <- function() {
  ledger <- CellLedger_with_genesis("sensor.a", 1.0, 0)
  ledger$record(2.0, 2.0, 1000)
  ledger$record(3.0, 3.0, 2000)
  check("clean chain intact", isTRUE(ledger$verify_chain()$intact))
  ledger$entries[[2]]$output$value <- 99.0   # rewrite history
  audit <- ledger$verify_chain()
  check("tampered chain broken at seq 2",
        !isTRUE(audit$intact) && identical(as.numeric(audit$first_break), 2))

  led2 <- CellLedger("slow.cell")
  ticket <- led2$open_input(list(request = jint(1)), 5000)
  check("open input does not balance",
        identical(as.numeric(led2$reconcile()$open_inputs), 1) && !isTRUE(led2$reconcile()$balanced))
  entry <- led2$settle_output(ticket, list(answer = jint(42)), 5050)
  expect_close("settled input value", entry$input$value, list(request = jint(1)), 0)
  expect_close("settled output value", entry$output$value, list(answer = jint(42)), 0)
  check("settled books balance",
        identical(as.numeric(led2$reconcile()$open_inputs), 0) && isTRUE(led2$reconcile()$balanced))
}

# ---------------------------------------------------------------------------
# Golden numbers report
# ---------------------------------------------------------------------------

report_numbers <- function(chain_hex) {
  g <- golden
  cat("\n")
  cat(strrep("-", 72), "\n", sep = "")
  cat("golden numbers (compat/golden.json)\n")
  cat(strrep("-", 72), "\n", sep = "")
  cat("(a) value reads:\n")
  for (v in g$op_a_value_read)
    cat(sprintf("    %-16s = %s\n", v$cell, pv(v$expect)))
  b <- g$op_b_formula_eval
  cat("(b) formula eval:\n")
  for (v in b$initial)
    cat(sprintf("    %-16s = %-6s (initial)\n", v$cell, pv(v$expect)))
  cat(sprintf("    push bilge.level -> %s\n", pv(b$after_push$value)))
  for (v in b$post)
    cat(sprintf("    %-16s = %-6s (post-push)\n", v$cell, pv(v$expect)))
  c <- g$op_c_propagation
  cat(sprintf("(c) propagation order after %s=%s:\n", c$mutate$cell, pv(c$mutate$value)))
  cat("    ", paste0("[", paste0(vc(c$expected_order), collapse = ", "), "]"), "\n", sep = "")
  cat("(d) wire edges:\n")
  for (v in g$op_d_edge)
    cat(sprintf("    %-20s delta=%-28s imb=%-8s prov=%s...\n",
                v$name, pv(v$expect$delta), pv(v$expect$imbalance),
                substr(v$expect$provenance, 1L, 16L)))
  e <- g$op_e_chain
  cat(sprintf("(e) chain (%s, genesis %s @ %s):\n",
              e$transcript$cell, pv(e$transcript$genesis), pv(e$transcript$genesis_ts)))
  for (entry in e$entries)
    cat(sprintf("    seq %d  prev %s...  seal %s...\n",
                as.numeric(entry$seq), substr(entry$prev_hash, 1L, 16L),
                substr(entry$hash, 1L, 16L)))
  cat("    chain_hash          = ", e$chain_hash, "\n", sep = "")
  cat("    chain_hash (R tier) = ", chain_hex, "\n", sep = "")
  rec <- e$reconcile
  cat(sprintf("    reconcile: entries=%d balanced=%s total_surprise=%s mean_surprise=%s\n",
              as.numeric(rec$entries), rec$balanced, pv(rec$total_surprise), pv(rec$mean_surprise)))
  cat(strrep("-", 72), "\n", sep = "")
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

main <- function() {
  cat("=== quilt-r conformance (tier: R) ===\n")
  cat("contract:", golden$contract, " golden:", golden_path, "\n")
  cat("R:", R.version.string, "\n")
  cat("sha256 backend:", sha256_backend(), "\n")
  cat("tolerances: (a) exact (b)", TOL_FORMULA, "(c) exact (d)", TOL_EDGE,
      "(e) bit-for-bit (e')", TOL_RECONCILE, "\n\n")

  results <- c(
    run_section("(a) value cell read", op_a),
    run_section("(b) formula cell eval", op_b),
    run_section("(c) propagation order", op_c),
    run_section("(d) edge record", op_d),
    run_section("(e) chain + reconcile", op_e),
    run_section("supporting: canonical JSON", sup_canonical),
    run_section("supporting: sha256 vectors", sup_sha256),
    run_section("supporting: value_distance", sup_distance),
    run_section("supporting: yaml sheet parsing", sup_yaml),
    run_section("supporting: dependency detection", sup_deps),
    run_section("supporting: engine/ledger integration", sup_integration),
    run_section("supporting: tamper + settle", sup_tamper_settle)
  )
  all_ok <- all(results)

  # Reproduce the op (e) chain independently for the final headline.
  chain_hex <- tryCatch({
    t <- golden$op_e_chain$transcript
    led <- CellLedger_with_genesis(t$cell, t$genesis, t$genesis_ts)
    for (rec in t$records) led$record(rec$input, rec$output, rec$ts)
    led$chain_hash()
  }, error = function(e) paste0("<error: ", conditionMessage(e), ">"))

  report_numbers(chain_hex)

  cat("\n")
  cat(strrep("=", 72), "\n", sep = "")
  if (all_ok) {
    cat("RESULT: PASS -- R tier conforms to quilt-compat/1\n")
  } else {
    cat("RESULT: FAIL -- ", length(STATE$failures), " failure(s), see above\n", sep = "")
  }
  cat("chain_hash: ", chain_hex, "\n", sep = "")
  cat(strrep("=", 72), "\n", sep = "")
  quit(status = if (all_ok) 0L else 1L)
}

main()
