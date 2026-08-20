# quilt.R — the R conformant tier (representation tier) for quilt-compat/1.
#
# Base-R-only implementation of the quilt engine and the sealed cell
# ledger, reproducing compat/golden.json bit-for-bit: same wire edges,
# same sha256 provenance, same chain seals. Mirrors the reference tier
# (packages/core/src/ledger.rs + engine.rs) and the sibling bindings
# (bindings/python, bindings/go).
#
# JSON value representation in R (the int/float distinction is part of
# the hash preimage, contract section 2):
#   JSON int    -> double with attr "jint"=TRUE   (canonical "85")
#   JSON float  -> plain double                    (canonical "85.0")
#   JSON string -> character (length 1)
#   JSON bool   -> logical  (length 1)
#   JSON null   -> NA (logical); R NULL means "absent", never a JSON value
#   JSON array  -> unnamed list; JSON object -> named list

# ---------------------------------------------------------------------------
# SHA-256 — prefer system sha256sum (base R, no packages)
# ---------------------------------------------------------------------------

..sha <- new.env(parent = emptyenv())
..sha$fn <- NULL
..sha$backend <- NULL

.init_sha256 <- function() {
  want <- "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
  fn <- NULL
  backend <- NULL
  if (nzchar(Sys.which("sha256sum"))) {
    fn <- function(s) {
      f <- tempfile(pattern = "quilt-")
      on.exit(unlink(f), add = TRUE)
      writeBin(charToRaw(s), f)
      out <- suppressWarnings(system2("sha256sum", shQuote(f), stdout = TRUE, stderr = FALSE))
      if (length(out) < 1L || !nzchar(out[1])) stop("sha256sum failed")
      sub("[^0-9a-f].*$", "", out[1])
    }
    backend <- sprintf("system sha256sum (%s)", Sys.which("sha256sum"))
  } else if (requireNamespace("digest", quietly = TRUE)) {
    fn <- function(s) digest::digest(s, algo = "sha256", serialize = FALSE)
    backend <- "digest package"
  } else {
    stop("no sha256 backend: need coreutils sha256sum on PATH (or the digest package)")
  }
  if (!identical(fn("abc"), want))
    stop("sha256 backend self-test FAILED: ", backend)
  ..sha$fn <- fn
  ..sha$backend <- backend
  invisible(NULL)
}

sha256_hex <- function(s) {
  if (is.null(..sha$fn)) .init_sha256()
  ..sha$fn(s)
}

sha256_backend <- function() {
  if (is.null(..sha$fn)) .init_sha256()
  ..sha$backend
}

# ---------------------------------------------------------------------------
# JSON value helpers
# ---------------------------------------------------------------------------

jint <- function(x) {
  v <- as.numeric(x)
  attributes(v) <- NULL
  attr(v, "jint") <- TRUE
  v
}

is_jint <- function(v) {
  is.numeric(v) && length(v) == 1L && !is.na(v) && !is.null(attr(v, "jint"))
}

is_float <- function(v) {
  is.numeric(v) && length(v) == 1L && !is.na(v) && is.null(attr(v, "jint"))
}

is_jnum <- function(v) is.numeric(v) && length(v) == 1L && !is.na(v)

is_json_null <- function(v) is.logical(v) && length(v) == 1L && is.na(v)

is_jstr <- function(v) is.character(v) && length(v) == 1L

is_jobj <- function(v) {
  is.list(v) && length(v) > 0L && !is.null(names(v)) && all(nzchar(names(v)))
}

is_jarr <- function(v) {
  is.list(v) && (length(v) == 0L || is.null(names(v)) || !all(nzchar(names(v))))
}

# C-locale radix sort == UTF-8 byte order for key canonicalization.
csort <- function(x) {
  if (is.null(x) || length(x) == 0L) return(character(0))
  unname(sort(x, method = "radix"))
}

# ---------------------------------------------------------------------------
# Canonical JSON — the hash preimage form (contract section 2)
# ---------------------------------------------------------------------------

# Shortest-round-trip float rendering, ryu / serde_json style (Python
# repr normalization: "1e-05" -> "1e-5", "1e+16" -> "1e16", 85.0 -> "85.0").
fmt_float <- function(x) {
  if (length(x) != 1L || is.na(x) || !is.finite(x))
    stop("non-finite number cannot be canonicalized")
  if (x == 0) return(if (1/x < 0) "-0.0" else "0.0")
  sgn <- if (x < 0) "-" else ""
  ax <- abs(x)
  s <- NULL
  for (p in 0:16) {
    cand <- sprintf(paste0("%.", p, "e"), ax)
    if (as.numeric(cand) == ax) { s <- cand; break }
  }
  if (is.null(s)) s <- sprintf("%.17e", ax)
  parts <- strsplit(s, "e", fixed = TRUE)[[1]]
  digits <- gsub(".", "", parts[1], fixed = TRUE)
  e <- as.integer(parts[2])
  n <- nchar(digits)
  if (e >= 16L || e < -4L) {
    mant <- if (n == 1L) digits else paste0(substr(digits, 1L, 1L), ".", substring(digits, 2L))
    return(paste0(sgn, mant, "e", e))
  }
  if (e >= n - 1L) return(paste0(sgn, digits, strrep("0", e - n + 1L), ".0"))
  if (e >= 0L) return(paste0(sgn, substr(digits, 1L, e + 1L), ".", substring(digits, e + 2L)))
  paste0(sgn, "0.", strrep("0", -e - 1L), digits)
}

fmt_int <- function(x) sprintf("%.0f", x)

json_escape <- function(s) {
  cps <- utf8ToInt(s)
  parts <- vapply(cps, function(cp) {
    if (cp == 0x22) return('\"')
    if (cp == 0x5C) return("\\\\")
    if (cp == 0x08) return("\\b")
    if (cp == 0x0C) return("\\f")
    if (cp == 0x0A) return("\\n")
    if (cp == 0x0D) return("\\r")
    if (cp == 0x09) return("\\t")
    if (cp < 0x20) return(sprintf("\\u%04x", cp))
    intToUtf8(cp)
  }, character(1), USE.NAMES = FALSE)
  paste0('"', paste0(parts, collapse = ""), '"')
}

canonical_json <- function(v) paste0(.cj(v), collapse = "")

.cj <- function(v) {
  if (is_json_null(v)) return("null")
  if (identical(v, TRUE)) return("true")
  if (identical(v, FALSE)) return("false")
  if (is_jint(v)) return(fmt_int(v))
  if (is_float(v)) return(fmt_float(v))
  if (is_jstr(v)) return(json_escape(v))
  if (is.list(v)) {
    if (is_jobj(v)) {
      keys <- csort(names(v))
      out <- "{"
      for (i in seq_along(keys)) {
        if (i > 1L) out <- c(out, ",")
        out <- c(out, json_escape(keys[i]), ":", .cj(v[[keys[i]]]))
      }
      return(c(out, "}"))
    }
    out <- "["
    for (i in seq_along(v)) {
      if (i > 1L) out <- c(out, ",")
      out <- c(out, .cj(v[[i]]))
    }
    return(c(out, "]"))
  }
  stop("cannot canonicalize value of class ", paste(class(v), collapse = "/"))
}

# ---------------------------------------------------------------------------
# serde equality + the generic distance metric (ledger.rs port)
# ---------------------------------------------------------------------------

# serde_json::Value equality: int and float are *different* numbers, so an
# edge 40 -> 40.0 is changed=TRUE with magnitude 0.0 (the cell-ledger.md
# float-vs-int hazard, preserved faithfully).
serde_eq <- function(a, b) {
  if (is.logical(a) || is.logical(b)) {
    return(is.logical(a) && is.logical(b) &&
             ((is.na(a) && is.na(b)) || (!is.na(a) && !is.na(b) && a == b)))
  }
  if (is_jnum(a) && is_jnum(b)) return(is_jint(a) == is_jint(b) && a == b)
  if (is_jstr(a) && is_jstr(b)) return(a == b)
  if (is.list(a) && is.list(b)) {
    a_obj <- is_jobj(a)
    b_obj <- is_jobj(b)
    if (a_obj != b_obj) return(FALSE)
    if (a_obj) {
      if (!setequal(names(a), names(b))) return(FALSE)
      for (k in names(a)) if (!serde_eq(a[[k]], b[[k]])) return(FALSE)
      return(TRUE)
    }
    if (length(a) != length(b)) return(FALSE)
    for (i in seq_along(a)) if (!serde_eq(a[[i]], b[[i]])) return(FALSE)
    return(TRUE)
  }
  FALSE
}

# Total metric between two JSON values (ledger.rs value_distance):
# numbers |a-b|; arrays mean of element-wise distances, missing cost 1.0;
# objects mean over key union, missing keys cost 1.0; equal values 0;
# any type shift 1.0.
value_distance <- function(a, b) {
  if (is_jnum(a) && is_jnum(b)) return(abs(a - b))
  if (is_jarr(a) && is_jarr(b)) {
    n <- max(length(a), length(b))
    if (n == 0L) return(0)
    total <- 0
    for (i in seq_len(n)) {
      if (i <= length(a) && i <= length(b)) total <- total + value_distance(a[[i]], b[[i]])
      else total <- total + 1
    }
    return(total / n)
  }
  if (is_jobj(a) && is_jobj(b)) {
    keys <- union(names(a), names(b))
    if (length(keys) == 0L) return(0)
    total <- 0
    for (k in keys) {
      if (k %in% names(a) && k %in% names(b)) total <- total + value_distance(a[[k]], b[[k]])
      else total <- total + 1
    }
    return(total / length(keys))
  }
  if (serde_eq(a, b)) return(0)
  1
}

# ---------------------------------------------------------------------------
# The wire edge — quilt-compat-contract section 1
# ---------------------------------------------------------------------------

# delta = after - before, first person: scalar diff / element-wise vector /
# null for anything non-numeric. Never fake a number.
wire_delta <- function(before, after) {
  if (is_jnum(before) && is_jnum(after)) return(after - before)
  if (is_jarr(before) && is_jarr(after) && length(before) == length(after)) {
    out <- list()
    for (i in seq_along(before)) {
      b <- before[[i]]
      a <- after[[i]]
      if (!is_jnum(b) || !is_jnum(a)) return(NA)
      out[[length(out) + 1L]] <- a - b
    }
    return(out)
  }
  NA
}

# |after - predict(before)| with the persistence prior: scalar abs diff,
# equal-length numeric vector -> L2 norm. No prior (before null) -> null.
wire_imbalance <- function(before, after) {
  if (is_jnum(before) && is_jnum(after)) return(abs(after - before))
  if (is_jarr(before) && is_jarr(after) && length(before) == length(after)) {
    total <- 0
    for (i in seq_along(before)) {
      b <- before[[i]]
      a <- after[[i]]
      if (!is_jnum(b) || !is_jnum(a)) return(NA)
      total <- total + (a - b)^2
    }
    return(sqrt(total))
  }
  NA
}

# sha256_hex(canonical_json(inputs)) — inputs in dependency-address order.
wire_provenance <- function(inputs) sha256_hex(canonical_json(inputs))

wire_edge <- function(cell, ts, before, after, inputs, chain, seq = NULL) {
  edge <- list(
    v = jint(1),
    cell = cell,
    ts = as.numeric(ts),
    before = before,
    after = after,
    delta = wire_delta(before, after),
    imbalance = wire_imbalance(before, after),
    provenance = wire_provenance(inputs),
    chain = chain
  )
  if (!is.null(seq)) edge$seq <- seq
  edge
}

# ---------------------------------------------------------------------------
# CellLedger — the sealed unit (port of packages/core/src/ledger.rs)
# ---------------------------------------------------------------------------

GENESIS_KIND <- "quilt-cell-ledger/1"

entry_seal <- function(body) {
  b <- body[names(body) != "hash"]
  sha256_hex(canonical_json(b))
}

CellLedger <- function(cell_id) {
  led <- new.env(parent = emptyenv())
  led$cell_id <- cell_id
  led$has_genesis <- FALSE
  led$genesis <- NA
  led$genesis_ts <- NA
  led$state <- NA
  led$next_seq <- 1
  led$next_ticket <- 1
  led$entries <- list()
  led$pending <- list()

  led$genesis_commit <- function() {
    body <- list(
      kind = GENESIS_KIND,
      cell_id = led$cell_id,
      genesis = if (led$has_genesis) led$genesis else NA,
      genesis_ts = if (led$has_genesis) led$genesis_ts else NA
    )
    sha256_hex(canonical_json(body))
  }

  led$chain_hash <- function() {
    n <- length(led$entries)
    if (n > 0L) led$entries[[n]]$hash else led$genesis_commit()
  }

  led$head <- function() {
    n <- length(led$entries)
    if (n > 0L) led$entries[[n]] else NULL
  }

  # The one place entries are born: edge computed, prediction scored,
  # seal hashed, state advanced.
  led$.append <- function(input, input_ts, output, output_ts, provenance, expected = NULL) {
    input_ts <- jint(input_ts)
    output_ts <- jint(output_ts)
    before <- led$state
    after <- output
    magnitude <- value_distance(before, after)
    changed <- !serde_eq(before, after)

    # A prior exists iff genesis or a completed entry; without one no
    # surprise is claimed (never fake a number).
    has_prior <- isTRUE(led$has_genesis) || length(led$entries) > 0L
    expected_val <- NULL
    imb_val <- NULL
    if (!is.null(expected)) {
      expected_val <- expected
      imb_val <- value_distance(expected, after)
    } else if (has_prior) {
      expected_val <- before  # persistence prior (Rust Some(Null) kept as null)
      imb_val <- magnitude
    }

    body <- list(
      seq = jint(led$next_seq),
      ts = input_ts,
      input = list(side = "input", value = input, ts = input_ts),
      output = list(side = "output", value = output, ts = output_ts),
      provenance = provenance,
      delta = list(before = before, after = after, changed = changed, magnitude = magnitude),
      prev_hash = led$chain_hash()
    )
    if (!is.null(expected_val)) body$expected <- expected_val
    if (!is.null(imb_val)) body$imbalance <- imb_val
    body$hash <- entry_seal(body)

    led$next_seq <- led$next_seq + 1
    led$state <- after
    led$entries[[length(led$entries) + 1L]] <- body
    body
  }

  led$record <- function(input, output, ts, provenance = list(origin = "system")) {
    led$.append(input, ts, output, ts, provenance)
  }

  led$open_input <- function(input, ts, provenance = list(origin = "system")) {
    ticket <- led$next_ticket
    led$next_ticket <- led$next_ticket + 1
    led$pending[[length(led$pending) + 1L]] <- list(
      ticket = ticket, ts = jint(ts), input = input, provenance = provenance
    )
    ticket
  }

  led$settle_output <- function(ticket, output, ts) {
    pos <- NULL
    for (i in seq_along(led$pending))
      if (identical(led$pending[[i]]$ticket, ticket)) { pos <- i; break }
    if (is.null(pos))
      stop("ledger '", led$cell_id, "': no open input with ticket ", ticket, call. = FALSE)
    pending <- led$pending[[pos]]
    led$pending <- led$pending[-pos]
    led$.append(pending$input, pending$ts, output, ts, pending$provenance)
  }

  led$verify_chain <- function() {
    expected_prev <- led$genesis_commit()
    for (e in led$entries) {
      if (!identical(e$prev_hash, expected_prev) || !identical(e$hash, entry_seal(e)))
        return(list(verified = as.numeric(e$seq - 1), intact = FALSE,
                    first_break = as.numeric(e$seq)))
      expected_prev <- e$hash
    }
    list(verified = length(led$entries), intact = TRUE, first_break = NULL)
  }

  led$reconcile <- function() {
    audit <- led$verify_chain()
    continuity <- TRUE
    prior <- if (led$has_genesis) led$genesis else NA
    for (e in led$entries) {
      if (!serde_eq(e$delta$before, prior)) { continuity <- FALSE; break }
      prior <- e$delta$after
    }
    matched <- 0
    scored <- numeric(0)
    for (e in led$entries) {
      if (identical(e$input$side, "input") && identical(e$output$side, "output"))
        matched <- matched + 1
      if (!is.null(e$imbalance)) scored <- c(scored, e$imbalance)
    }
    total <- if (length(scored)) sum(scored) else 0
    list(
      cell_id = led$cell_id,
      entries = length(led$entries),
      open_inputs = length(led$pending),
      matched_pairs = matched,
      chain_intact = audit$intact,
      first_break = audit$first_break,
      continuity_intact = continuity,
      total_surprise = total,
      mean_surprise = if (length(scored)) total / length(scored) else NA,
      balanced = length(led$pending) == 0L && matched == length(led$entries) &&
        isTRUE(audit$intact) && continuity
    )
  }

  led$replay <- function(until_ts) {
    picked <- list()
    surprise <- 0
    for (e in led$entries) {
      if (as.numeric(e$ts) <= as.numeric(until_ts)) {
        picked[[length(picked) + 1L]] <- e
        if (!is.null(e$imbalance)) surprise <- surprise + e$imbalance
      }
    }
    n <- length(picked)
    list(
      cell_id = led$cell_id,
      until_ts = until_ts,
      replayed = n,
      state = if (n > 0L) picked[[n]]$delta$after else if (led$has_genesis) led$genesis else NA,
      surprise = surprise
    )
  }

  led$to_wire <- function(e) {
    wire_edge(led$cell_id, as.numeric(e$ts), e$delta$before, e$delta$after,
              list(e$input$value), e$prev_hash, e$seq)
  }

  led$wire_edges <- function() lapply(led$entries, function(e) led$to_wire(e))

  led
}

CellLedger_with_genesis <- function(cell_id, genesis, genesis_ts) {
  led <- CellLedger(cell_id)
  led$has_genesis <- TRUE
  led$genesis <- genesis
  led$genesis_ts <- jint(genesis_ts)
  led$state <- genesis
  led
}

# ---------------------------------------------------------------------------
# The formula language — JS-flavored expression subset
# (port of bindings/python/quilt/formula.py; the Rust engine uses rhai)
# ---------------------------------------------------------------------------

formula_tokenize <- function(src) {
  es <- strsplit(src, "", fixed = TRUE)[[1]]
  n <- length(es)
  i <- 1L
  toks <- list()
  TWO <- c("<=", ">=", "==", "!=", "&&", "||")
  ONE <- strsplit("+-*/%()<>,!?:", "", fixed = TRUE)[[1]]
  dg <- strsplit("0123456789", "", fixed = TRUE)[[1]]
  add <- function(tok) toks[[length(toks) + 1L]] <<- tok
  while (i <= n) {
    ch <- es[i]
    if (ch %in% c(" ", "\t", "\r", "\n")) { i <- i + 1L; next }
    if (ch == "'" || ch == '"') {
      j <- i + 1L
      buf <- character(0)
      while (j <= n && es[j] != ch) {
        if (es[j] == "\\" && j + 1L <= n) {
          esc <- es[j + 1L]
          buf <- c(buf, switch(esc, "n" = "\n", "t" = "\t", "r" = "\r", esc))
          j <- j + 2L
        } else {
          buf <- c(buf, es[j])
          j <- j + 1L
        }
      }
      if (j > n) stop("unterminated string in formula: ", src, call. = FALSE)
      add(list("str", paste0(buf, collapse = "")))
      i <- j + 1L
      next
    }
    if (ch %in% dg || (ch == "." && i + 1L <= n && es[i + 1L] %in% dg)) {
      j <- i
      seen_dot <- FALSE
      seen_exp <- FALSE
      while (j <= n) {
        c1 <- es[j]
        if (c1 %in% dg) j <- j + 1L
        else if (c1 == "." && !seen_dot && !seen_exp) { seen_dot <- TRUE; j <- j + 1L }
        else if ((c1 == "e" || c1 == "E") && !seen_exp && j > i) {
          seen_exp <- TRUE
          j <- j + 1L
          if (j <= n && (es[j] == "+" || es[j] == "-")) j <- j + 1L
        } else break
      }
      text <- paste0(es[i:(j - 1L)], collapse = "")
      if (seen_dot || seen_exp) add(list("num", as.numeric(text)))
      else add(list("num", jint(as.numeric(text))))
      i <- j
      next
    }
    if (grepl("[A-Za-z_]", ch)) {
      j <- i
      while (j <= n && (grepl("[A-Za-z0-9]", es[j]) || es[j] == "_" || es[j] == ".")) j <- j + 1L
      add(list("id", paste0(es[i:(j - 1L)], collapse = "")))
      i <- j
      next
    }
    two <- if (i + 1L <= n) paste0(es[i], es[i + 1L]) else ""
    if (two %in% TWO) { add(list("op", two)); i <- i + 2L; next }
    if (ch %in% ONE) { add(list("op", ch)); i <- i + 1L; next }
    stop("unexpected character '", ch, "' in formula: ", src, call. = FALSE)
  }
  toks
}

formula_parse <- function(src) {
  body <- if (startsWith(src, "=")) substring(src, 2L) else src
  toks <- formula_tokenize(body)
  if (!length(toks)) stop("empty formula", call. = FALSE)
  p <- new.env(parent = emptyenv())
  p$toks <- toks
  p$i <- 1L

  peek <- function() if (p$i <= length(p$toks)) p$toks[[p$i]] else list(NULL, NULL)
  nxt <- function() { t <- peek(); p$i <- p$i + 1L; t }
  eat_op <- function(...) {
    t <- peek()
    if (identical(t[[1]], "op") && t[[2]] %in% c(...)) { p$i <- p$i + 1L; t[[2]] }
    else NULL
  }

  # ternary < or < and < equality < relational < additive < multiplicative
  #   < unary < primary
  ternary <- function() {
    cond <- or_()
    if (!is.null(eat_op("?"))) {
      a <- ternary()
      if (is.null(eat_op(":"))) stop("expected ':' in ternary", call. = FALSE)
      b <- ternary()
      return(list("ter", cond, a, b))
    }
    cond
  }
  or_ <- function() {
    left <- and_()
    while (!is.null(eat_op("||"))) left <- list("bin", "||", left, and_())
    left
  }
  and_ <- function() {
    left <- equality()
    while (!is.null(eat_op("&&"))) left <- list("bin", "&&", left, equality())
    left
  }
  equality <- function() {
    left <- relational()
    repeat {
      op <- eat_op("==", "!=")
      if (is.null(op)) return(left)
      left <- list("bin", op, left, relational())
    }
  }
  relational <- function() {
    left <- additive()
    repeat {
      op <- eat_op("<", ">", "<=", ">=")
      if (is.null(op)) return(left)
      left <- list("bin", op, left, additive())
    }
  }
  additive <- function() {
    left <- multiplicative()
    repeat {
      op <- eat_op("+", "-")
      if (is.null(op)) return(left)
      left <- list("bin", op, left, multiplicative())
    }
  }
  multiplicative <- function() {
    left <- unary()
    repeat {
      op <- eat_op("*", "/", "%")
      if (is.null(op)) return(left)
      left <- list("bin", op, left, unary())
    }
  }
  unary <- function() {
    if (!is.null(eat_op("!"))) return(list("un", "!", unary()))
    if (!is.null(eat_op("-"))) return(list("un", "-", unary()))
    if (!is.null(eat_op("+"))) return(unary())
    primary()
  }
  primary <- function() {
    t <- nxt()
    kind <- t[[1]]
    val <- t[[2]]
    if (identical(kind, "num")) return(list("num", val))
    if (identical(kind, "str")) return(list("str", val))
    if (identical(kind, "id")) {
      if (identical(val, "true")) return(list("num", TRUE))
      if (identical(val, "false")) return(list("num", FALSE))
      if (identical(val, "null")) return(list("num", NA))
      if (!is.null(eat_op("("))) {
        args <- list()
        if (is.null(eat_op(")"))) {
          repeat {
            args[[length(args) + 1L]] <- ternary()
            if (!is.null(eat_op(","))) next
            if (!is.null(eat_op(")"))) break
            stop("expected ',' or ')' in call", call. = FALSE)
          }
        }
        return(list("call", val, args))
      }
      return(list("ref", val))
    }
    if (identical(kind, "op") && identical(val, "(")) {
      inner <- ternary()
      if (is.null(eat_op(")"))) stop("expected ')'", call. = FALSE)
      return(inner)
    }
    stop("unexpected token in formula", call. = FALSE)
  }

  ast <- ternary()
  if (p$i <= length(p$toks)) stop("trailing tokens in formula", call. = FALSE)
  ast
}

# -- evaluation -------------------------------------------------------------

.f_truthy <- function(v) {
  if (is_json_null(v) || identical(v, FALSE)) return(FALSE)
  if (is_jnum(v)) return(v != 0)
  if (is_jstr(v)) return(v != "")
  TRUE
}

# Render a float the way JS String(x) would (for + concatenation).
fmt_float_js <- function(x) {
  if (x == trunc(x) && abs(x) < 1e21) sprintf("%.0f", x) else fmt_float(x)
}

.f_to_str <- function(v) {
  if (is_json_null(v)) return("null")
  if (identical(v, TRUE)) return("true")
  if (identical(v, FALSE)) return("false")
  if (is_jint(v)) return(fmt_int(v))
  if (is_jnum(v)) return(fmt_float_js(v))
  v
}

# IEEE division (always float); % is the JS truncated remainder (sign of
# the dividend); the int/float distinction is preserved for + - * % on
# two ints.
.f_numeric <- function(a, b, op) {
  if (!is_jnum(a) || !is_jnum(b))
    stop("'", op, "' on non-numbers", call. = FALSE)
  ai <- is_jint(a)
  bi <- is_jint(b)
  if (op == "+") return(if (ai && bi) jint(a + b) else a + b)
  if (op == "-") return(if (ai && bi) jint(a - b) else a - b)
  if (op == "*") return(if (ai && bi) jint(a * b) else a * b)
  if (op == "/") {
    if (b == 0) stop("division by zero", call. = FALSE)
    return(a / b)
  }
  if (op == "%") {
    if (b == 0) stop("modulo by zero", call. = FALSE)
    r <- a - trunc(a / b) * b  # fmod semantics
    return(if (ai && bi) jint(r) else r)
  }
  stop("unknown operator ", op, call. = FALSE)
}

.f_equals <- function(a, b) {
  if (is.logical(a) || is.logical(b))
    return(is.logical(a) && is.logical(b) &&
             ((is.na(a) && is.na(b)) || (!is.na(a) && !is.na(b) && a == b)))
  if (is_jnum(a) && is_jnum(b)) return(a == b)
  if (is_jstr(a) && is_jstr(b)) return(a == b)
  if (is.list(a) && is.list(b)) {
    if (length(a) != length(b) || !setequal(names(a), names(b))) return(FALSE)
    for (k in names(a)) if (!.f_equals(a[[k]], b[[k]])) return(FALSE)
    return(TRUE)
  }
  FALSE
}

.f_compare <- function(op, a, b) {
  if (!((is_jnum(a) && is_jnum(b)) || (is_jstr(a) && is_jstr(b))))
    stop("'", op, "' on incomparable values", call. = FALSE)
  switch(op, "<" = a < b, ">" = a > b, "<=" = a <= b, ">=" = a >= b)
}

.f_call <- function(name, args) {
  num <- function(v) {
    if (!is_jnum(v)) stop(name, "() expects numbers", call. = FALSE)
    v
  }
  if (identical(name, "abs")) {
    if (length(args) != 1L) stop("abs() takes one argument", call. = FALSE)
    v <- num(args[[1]])
    return(if (is_jint(v)) jint(abs(v)) else abs(v))
  }
  if (identical(name, "min") || identical(name, "max")) {
    if (!length(args)) stop(name, "() requires at least one argument", call. = FALSE)
    vals <- vapply(args, function(a) as.numeric(num(a)), numeric(1))
    pick <- if (identical(name, "min")) min(vals) else max(vals)
    return(if (all(vapply(args, is_jint, logical(1)))) jint(pick) else pick)
  }
  if (identical(name, "clamp")) {
    if (length(args) != 3L) stop("clamp(n, lo, hi) takes three arguments", call. = FALSE)
    nv <- num(args[[1]]); lo <- num(args[[2]]); hi <- num(args[[3]])
    if (nv < lo) return(lo)
    if (nv > hi) return(hi)
    return(nv)
  }
  stop("unknown function ", name, "()", call. = FALSE)
}

formula_eval <- function(node, resolve) {
  tag <- node[[1]]
  if (identical(tag, "num")) return(node[[2]])
  if (identical(tag, "str")) return(node[[2]])
  if (identical(tag, "ref")) return(resolve(node[[2]]))
  if (identical(tag, "un")) {
    v <- formula_eval(node[[3]], resolve)
    if (identical(node[[2]], "-")) {
      if (!is_jnum(v)) stop("unary '-' on non-number", call. = FALSE)
      return(if (is_jint(v)) jint(-v) else -v)
    }
    return(!.f_truthy(v))
  }
  if (identical(tag, "ter")) {
    cnd <- formula_eval(node[[2]], resolve)
    return(formula_eval(if (.f_truthy(cnd)) node[[3]] else node[[4]], resolve))
  }
  if (identical(tag, "call")) {
    args <- lapply(node[[3]], function(a) formula_eval(a, resolve))
    return(.f_call(node[[2]], args))
  }
  if (identical(tag, "bin")) {
    op <- node[[2]]
    if (identical(op, "&&")) {
      left <- formula_eval(node[[3]], resolve)
      if (!.f_truthy(left)) return(left)
      return(formula_eval(node[[4]], resolve))
    }
    if (identical(op, "||")) {
      left <- formula_eval(node[[3]], resolve)
      if (.f_truthy(left)) return(left)
      return(formula_eval(node[[4]], resolve))
    }
    a <- formula_eval(node[[3]], resolve)
    b <- formula_eval(node[[4]], resolve)
    if (identical(op, "+")) {
      if (is.character(a) || is.character(b)) return(paste0(.f_to_str(a), .f_to_str(b)))
      return(.f_numeric(a, b, "+"))
    }
    if (op %in% c("-", "*", "/", "%")) return(.f_numeric(a, b, op))
    if (identical(op, "==")) return(.f_equals(a, b))
    if (identical(op, "!=")) return(!.f_equals(a, b))
    if (op %in% c("<", ">", "<=", ">=")) return(.f_compare(op, a, b))
  }
  stop("bad AST node", call. = FALSE)
}

# ---------------------------------------------------------------------------
# Dependency detection — whole-token id scan, longest-first
# (port of formula.rs::rewrite_known_ids / bindings/python engine scan)
# ---------------------------------------------------------------------------

..BOUNDARY <- c(letters, LETTERS, strsplit("0123456789_.", "", fixed = TRUE)[[1]])

detect_dependencies <- function(expr, known_ids) {
  known <- known_ids[nzchar(known_ids)]
  known <- known[order(-nchar(known))]  # longest first, stable
  deps <- character(0)
  es <- strsplit(expr, "", fixed = TRUE)[[1]]
  n <- length(es)
  i <- 1L
  while (i <= n) {
    ch <- es[i]
    if (ch == "'" || ch == '"') {
      i <- i + 1L
      while (i <= n && es[i] != ch) i <- i + 1L
      i <- i + 1L
      next
    }
    matched <- FALSE
    for (kid in known) {
      lk <- nchar(kid)
      if (i + lk - 1L <= n && paste0(es[i:(i + lk - 1L)], collapse = "") == kid) {
        left_ok <- i == 1L || !(es[i - 1L] %in% ..BOUNDARY)
        j <- i + lk
        right_ok <- j > n || !(es[j] %in% ..BOUNDARY)
        if (left_ok && right_ok) {
          if (!(kid %in% deps)) deps <- c(deps, kid)
          i <- j
          matched <- TRUE
          break
        }
      }
    }
    if (!matched) i <- i + 1L
  }
  deps
}

# ---------------------------------------------------------------------------
# Mini-YAML — the quilt-sheet subset
# (port of bindings/python/quilt/miniyaml.py)
# ---------------------------------------------------------------------------

.y_strip_comment <- function(line) {
  es <- strsplit(line, "", fixed = TRUE)[[1]]
  quote <- NULL
  for (i in seq_along(es)) {
    ch <- es[i]
    if (!is.null(quote)) {
      if (identical(ch, quote)) quote <- NULL
    } else if (ch == "'" || ch == '"') {
      quote <- ch
    } else if (ch == "#" && (i == 1L || es[i - 1L] %in% c(" ", "\t"))) {
      return(paste0(es[seq_len(i - 1L)], collapse = ""))
    }
  }
  line
}

.y_scalar <- function(text) {
  s <- trimws(text)
  if (!nzchar(s)) return(NA)
  if (nchar(s) >= 2L && substr(s, 1L, 1L) == substr(s, nchar(s), nchar(s)) &&
      substr(s, 1L, 1L) %in% c("'", '"')) {
    inner <- substring(s, 2L, nchar(s) - 1L)
    if (substr(s, 1L, 1L) == '"') {
      inner <- gsub('\\"', '"', inner, fixed = TRUE)
      inner <- gsub("\\\\", "\\", inner, fixed = TRUE)
      inner <- gsub("\\n", "\n", inner, fixed = TRUE)
      inner <- gsub("\\t", "\t", inner, fixed = TRUE)
    } else {
      inner <- gsub("''", "'", inner, fixed = TRUE)
    }
    return(inner)
  }
  if (s %in% c("null", "~", "Null", "NULL")) return(NA)
  if (s %in% c("true", "True", "TRUE")) return(TRUE)
  if (s %in% c("false", "False", "FALSE")) return(FALSE)
  if (grepl("^[+-]?[0-9]+$", s)) return(jint(as.numeric(s)))
  if (grepl("^[+-]?([0-9]+\\.[0-9]*|\\.[0-9]+|[0-9]+)([eE][+-]?[0-9]+)?$", s) &&
      grepl("[.eE]", s))
    return(as.numeric(s))
  s
}

.y_split_flow <- function(text) {
  es <- strsplit(text, "", fixed = TRUE)[[1]]
  parts <- list()
  buf <- character(0)
  depth <- 0L
  quote <- NULL
  for (ch in es) {
    if (!is.null(quote)) {
      buf <- c(buf, ch)
      if (identical(ch, quote)) quote <- NULL
      next
    }
    if (ch == "'" || ch == '"') { quote <- ch; buf <- c(buf, ch); next }
    if (ch == "[" || ch == "{") { depth <- depth + 1L; buf <- c(buf, ch); next }
    if (ch == "]" || ch == "}") { depth <- depth - 1L; buf <- c(buf, ch); next }
    if (ch == "," && depth == 0L) {
      parts[[length(parts) + 1L]] <- paste0(buf, collapse = "")
      buf <- character(0)
      next
    }
    buf <- c(buf, ch)
  }
  if (nzchar(trimws(paste0(buf, collapse = ""))))
    parts[[length(parts) + 1L]] <- paste0(buf, collapse = "")
  parts
}

.y_parse_flow <- function(text) {
  s <- trimws(text)
  if (startsWith(s, "[") && endsWith(s, "]"))
    return(lapply(.y_split_flow(substring(s, 2L, nchar(s) - 1L)), .y_parse_flow))
  if (startsWith(s, "{") && endsWith(s, "}")) {
    out <- list()
    keys <- character(0)
    for (part in .y_split_flow(substring(s, 2L, nchar(s) - 1L))) {
      m <- regexpr(":", part, fixed = TRUE)
      if (m < 0) stop("bad flow mapping entry: ", part, call. = FALSE)
      keys <- c(keys, .y_scalar(trimws(substr(part, 1L, m - 1L))))
      out[[length(out) + 1L]] <- .y_parse_flow(trimws(substring(part, m + attr(m, "match.length"))))
    }
    names(out) <- keys
    return(out)
  }
  .y_scalar(s)
}

# Text up to and including the first top-level ':' (or all of it).
.y_key_span <- function(text) {
  es <- strsplit(text, "", fixed = TRUE)[[1]]
  quote <- NULL
  for (i in seq_along(es)) {
    ch <- es[i]
    if (!is.null(quote)) {
      if (identical(ch, quote)) quote <- NULL
    } else if (ch == "'" || ch == '"') {
      quote <- ch
    } else if (ch == ":") {
      return(paste0(es[seq_len(i)], collapse = ""))
    }
  }
  text
}

.y_split_kv <- function(text) {
  span <- .y_key_span(text)
  if (endsWith(span, ":")) {
    list(key = trimws(substr(span, 1L, nchar(span) - 1L)),
         val = trimws(substring(text, nchar(span) + 1L)))
  } else {
    list(key = text, val = NULL)
  }
}

yaml_parse <- function(source) {
  raw_lines <- strsplit(source, "\n", fixed = TRUE)[[1]]
  raw_lines <- sub("\r$", "", raw_lines)

  # -- lex: comments, indentation, block-scalar raw lines -------------------
  L <- list()
  i <- 1L
  while (i <= length(raw_lines)) {
    raw <- raw_lines[i]
    num <- i
    i <- i + 1L
    stripped <- sub("[ \t]+$", "", .y_strip_comment(raw))
    if (!nzchar(trimws(stripped))) next
    if (trimws(stripped) == "---") next
    indent <- nchar(stripped) - nchar(sub("^ +", "", stripped))
    if (grepl("\t", substr(stripped, 1L, indent + 1L), fixed = TRUE))
      stop("line ", num, ": tabs are not valid indentation", call. = FALSE)
    content <- sub("^ +", "", stripped)
    L[[length(L) + 1L]] <- list(indent = indent, content = content, num = num)
    # A block scalar header: keep the deeper lines RAW (# is legal in code).
    if (endsWith(content, "|") || endsWith(content, "|-") || endsWith(content, "|+")) {
      while (i <= length(raw_lines)) {
        nxt <- raw_lines[i]
        if (!nzchar(trimws(nxt))) {
          L[[length(L) + 1L]] <- list(indent = indent + 2L, content = "", num = i)
          i <- i + 1L
          next
        }
        n_indent <- nchar(nxt) - nchar(sub("^ +", "", nxt))
        if (n_indent <= indent) break
        L[[length(L) + 1L]] <- list(indent = n_indent, content = sub("^ +", "", nxt), num = i)
        i <- i + 1L
      }
      while (length(L) > 0L && !nzchar(L[[length(L)]]$content)) L[[length(L)]] <- NULL
    }
  }
  if (!length(L)) return(list())

  pos <- new.env(parent = emptyenv())
  pos$i <- 1L

  .peek_deeper <- function(indent) pos$i <= length(L) && L[[pos$i]]$indent > indent
  .is_item <- function(content) startsWith(content, "- ") || identical(content, "-")

  .parse_block <- function(indent) {
    if (pos$i > length(L)) return(NA)
    if (.is_item(L[[pos$i]]$content)) return(.parse_seq(indent))
    .parse_map(indent)
  }

  .parse_value <- function(val, indent) {
    if (identical(val, "|")) return(.block_scalar(indent))
    if (!nzchar(val)) {
      if (.peek_deeper(indent)) return(.parse_block(L[[pos$i]]$indent))
      return(NA)
    }
    .y_parse_flow(val)
  }

  .block_scalar <- function(indent) {
    body <- character(0)
    while (pos$i <= length(L) && L[[pos$i]]$indent > indent) {
      pad <- max(L[[pos$i]]$indent - (indent + 2L), 0)
      body <- c(body, paste0(strrep(" ", pad), L[[pos$i]]$content))
      pos$i <- pos$i + 1L
    }
    if (!length(body)) return("")
    paste0(paste0(body, collapse = "\n"), "\n")
  }

  .continue_map <- function(item, indent) {
    while (pos$i <= length(L)) {
      ln <- L[[pos$i]]
      if (ln$indent != indent || .is_item(ln$content)) break
      kv <- .y_split_kv(ln$content)
      if (is.null(kv$val) && !endsWith(ln$content, ":")) break
      pos$i <- pos$i + 1L
      val <- if (is.null(kv$val)) "" else kv$val
      if (!nzchar(val)) {
        if (.peek_deeper(indent)) item[[kv$key]] <- .parse_block(L[[pos$i]]$indent)
        else item[[kv$key]] <- NA
      } else {
        item[[kv$key]] <- .parse_value(val, indent)
      }
    }
    item
  }

  .parse_seq <- function(indent) {
    items <- list()
    while (pos$i <= length(L)) {
      ln <- L[[pos$i]]
      if (ln$indent != indent || !.is_item(ln$content)) break
      pos$i <- pos$i + 1L
      rest <- trimws(sub("^-", "", ln$content))
      if (!nzchar(rest)) {
        items[[length(items) + 1L]] <-
          if (.peek_deeper(indent)) .parse_block(L[[pos$i]]$indent) else NA
        next
      }
      if (endsWith(.y_key_span(rest), ":")) {
        # "- key: value" — inline first key of a mapping item; the item's
        # remaining keys sit two columns past the dash.
        kv <- .y_split_kv(rest)
        item_indent <- ln$indent + 2L
        item <- list()
        if (!nzchar(kv$val)) {
          if (.peek_deeper(item_indent)) item[[kv$key]] <- .parse_block(L[[pos$i]]$indent)
          else item[[kv$key]] <- NA
        } else {
          item[[kv$key]] <- .parse_value(kv$val, item_indent)
        }
        items[[length(items) + 1L]] <- .continue_map(item, item_indent)
      } else {
        items[[length(items) + 1L]] <- .y_parse_flow(rest)
      }
    }
    items
  }

  .parse_map <- function(indent) {
    item <- list()
    while (pos$i <= length(L)) {
      ln <- L[[pos$i]]
      if (ln$indent != indent) {
        if (ln$indent > indent)
          stop("line ", ln$num, ": unexpected indent ", ln$indent, call. = FALSE)
        break
      }
      if (.is_item(ln$content)) break
      kv <- .y_split_kv(ln$content)
      if (is.null(kv$val)) stop("line ", ln$num, ": expected 'key: value'", call. = FALSE)
      pos$i <- pos$i + 1L
      item[[kv$key]] <- .parse_value(kv$val, indent)
    }
    item
  }

  result <- .parse_block(L[[1L]]$indent)
  if (pos$i != length(L) + 1L)
    stop("line ", L[[pos$i]]$num, ": trailing content could not be parsed", call. = FALSE)
  result
}

# ---------------------------------------------------------------------------
# Sheet validation
# ---------------------------------------------------------------------------

KNOWN_KINDS <- c("value", "formula", "api", "program", "sensor", "io", "listener", "router")

# Build a SheetDef from an already-parsed document (e.g. the golden JSON
# `sheet` section) with the same validation as parse_sheet.
sheet_from_dict <- function(doc) {
  if (!is.list(doc) || is.null(names(doc))) stop("sheet must be a mapping")
  sheet_id <- doc$id
  if (!is_jstr(sheet_id) || !nzchar(sheet_id))
    stop("sheet requires a top-level `id`")
  raw <- doc$cells
  if (is.null(raw)) raw <- list()
  if (!is.list(raw)) stop("`cells` must be a list")
  cells <- list()
  seen <- character(0)
  for (i in seq_along(raw)) {
    entry <- raw[[i]]
    if (is.null(names(entry)) || !all(nzchar(names(entry))))
      stop("cell #", i - 1L, " must be a mapping")
    cid <- entry$id
    kind <- entry$kind
    if (!is_jstr(cid) || !nzchar(trimws(cid)))
      stop("cell #", i - 1L, " requires a non-empty `id`")
    if (cid %in% seen) stop("duplicate cell id: ", cid)
    if (!(is_jstr(kind) && kind %in% KNOWN_KINDS))
      stop("cell '", cid, "': unknown kind '", kind, "'")
    seen <- c(seen, cid)
    if (identical(kind, "value") && is.null(entry$value))
      stop("value cell '", cid, "' requires `value`")
    if (identical(kind, "formula") && !is_jstr(entry$expr))
      stop("formula cell '", cid, "' requires `expr`")
    cells[[length(cells) + 1L]] <- list(id = cid, kind = kind, extra = entry)
  }
  list(id = sheet_id, cells = cells)
}

# Parse quilt-sheet YAML into a SheetDef.
parse_sheet <- function(source) sheet_from_dict(yaml_parse(source))

# ---------------------------------------------------------------------------
# QuiltEngine — reactive cells + per-cell sealed ledgers
# (same semantics as packages/core/src/engine.rs and the python binding)
# ---------------------------------------------------------------------------

now_millis <- function() as.numeric(Sys.time()) * 1000

QuiltEngine <- function(sheet, record_edges = TRUE) {
  eng <- new.env(parent = emptyenv())
  eng$record_edges <- record_edges
  eng$cells <- list()
  eng$ledgers <- list()

  ids <- vapply(sheet$cells, function(x) x$id, character(1))

  # -- build cells, compile formulas, detect dependencies -------------------
  for (cdef in sheet$cells) {
    cell <- list(id = cdef$id, kind = cdef$kind, extra = cdef$extra,
                 deps = character(0), dependents = character(0),
                 value = NULL, has_value = FALSE, status = "idle", error = NULL,
                 stale = TRUE, ast = NULL)
    if (identical(cdef$kind, "formula")) {
      cell$ast <- tryCatch(formula_parse(cdef$extra$expr), error = function(e) NULL)
      cell$deps <- if (is.null(cell$ast)) character(0)
                   else detect_dependencies(cdef$extra$expr, ids)
    }
    eng$cells[[cdef$id]] <- cell
  }

  # -- wire the reverse index (dependents) ----------------------------------
  for (cid in ids) {
    for (d in eng$cells[[cid]]$deps) {
      if (!is.null(eng$cells[[d]]))
        eng$cells[[d]]$dependents <- csort(unique(c(eng$cells[[d]]$dependents, cid)))
    }
  }

  # -- seed initial state + genesis ledgers (ts=0, the sheet's birth) -------
  for (cdef in sheet$cells) {
    kind <- cdef$kind
    ex <- cdef$extra
    led <- NULL
    if (identical(kind, "value")) {
      eng$cells[[cdef$id]]$value <- ex$value
      eng$cells[[cdef$id]]$status <- "ready"
      eng$cells[[cdef$id]]$has_value <- TRUE
      led <- CellLedger_with_genesis(cdef$id, ex$value, 0)
    } else if (identical(kind, "sensor")) {
      eng$cells[[cdef$id]]$value <- if (is.null(ex$default)) NA else ex$default
      eng$cells[[cdef$id]]$status <- "ready"
      eng$cells[[cdef$id]]$has_value <- TRUE
      if ("default" %in% names(ex))
        led <- CellLedger_with_genesis(cdef$id, ex$default, 0)
      else
        led <- CellLedger(cdef$id)
    } else {
      led <- CellLedger(cdef$id)  # no genesis: computed later
    }
    eng$ledgers[[cdef$id]] <- led
  }

  eng$.cell <- function(cell_id) {
    cell <- eng$cells[[cell_id]]
    if (is.null(cell)) stop("cell not found: ", cell_id, call. = FALSE)
    cell
  }

  # -- the universal verbs ----------------------------------------------------

  # Read a cell. A stale formula recomputes here (lazy, like Excel).
  eng$get <- function(cell_id, ts = NULL) {
    cell <- eng$.cell(cell_id)
    if (identical(cell$kind, "formula")) {
      if (isTRUE(cell$stale) || !cell$has_value || !identical(cell$status, "ready"))
        return(eng$.recompute(cell_id, if (is.null(ts)) now_millis() else ts))
      return(list(data = cell$value, status = cell$status, error = cell$error))
    }
    list(data = cell$value, status = cell$status, error = cell$error)
  }

  # Write a cell, mark transitive dependents stale, record the edge.
  eng$set <- function(cell_id, value, ts = NULL) {
    cell <- eng$.cell(cell_id)
    cell$value <- value
    cell$has_value <- TRUE
    cell$status <- "ready"
    cell$error <- NULL
    cell$stale <- FALSE
    eng$cells[[cell_id]] <- cell
    if (eng$record_edges)
      eng$ledgers[[cell_id]]$record(value, value,
                                    if (is.null(ts)) now_millis() else ts,
                                    list(origin = "set"))
    eng$.mark_stale(cell_id, character(0))
    invisible(NULL)
  }

  # Feed a sensor/io cell from an adapter (records a push edge).
  eng$push <- function(cell_id, value, ts = NULL) {
    cell <- eng$.cell(cell_id)
    if (!(identical(cell$kind, "sensor") || identical(cell$kind, "io")))
      stop("push() is for sensor/io cells, not ", cell$kind, call. = FALSE)
    cell$value <- value
    cell$has_value <- TRUE
    cell$status <- "ready"
    cell$error <- NULL
    cell$stale <- FALSE
    eng$cells[[cell_id]] <- cell
    if (eng$record_edges)
      eng$ledgers[[cell_id]]$record(value, value,
                                    if (is.null(ts)) now_millis() else ts,
                                    list(origin = "push"))
    eng$.mark_stale(cell_id, character(0))
    invisible(NULL)
  }

  # -- graph --------------------------------------------------------------------

  eng$dependencies <- function(cell_id) csort(eng$.cell(cell_id)$deps)
  eng$dependents <- function(cell_id) csort(eng$.cell(cell_id)$dependents)
  eng$chain_hash <- function(cell_id) eng$ledgers[[cell_id]]$chain_hash()
  eng$wire_edges <- function(cell_id) eng$ledgers[[cell_id]]$wire_edges()

  # The deterministic propagation order for a mutation of `root`
  # (quilt-compat op c): Kahn's algorithm over the affected closure, ties
  # broken by lexicographic (UTF-8 byte) address order.
  eng$propagation_order <- function(root) {
    closure <- root
    queue <- root
    while (length(queue) > 0L) {
      cur <- queue[length(queue)]
      queue <- queue[-length(queue)]
      for (dep in csort(eng$.cell(cur)$dependents)) {
        if (!(dep %in% closure)) {
          closure <- c(closure, dep)
          queue <- c(queue, dep)
        }
      }
    }
    indegree <- as.list(setNames(rep(0, length(closure)), closure))
    dependents_of <- as.list(setNames(vector("list", length(closure)), closure))
    for (cid in closure) {
      for (d in csort(eng$.cell(cid)$deps)) {
        if (d %in% closure) {
          indegree[[cid]] <- indegree[[cid]] + 1
          dependents_of[[d]] <- c(dependents_of[[d]], cid)
        }
      }
    }
    ready <- csort(closure[vapply(closure, function(cid) indegree[[cid]] == 0, logical(1))])
    order <- character(0)
    while (length(ready) > 0L) {
      cid <- ready[1L]
      ready <- ready[-1L]
      order <- c(order, cid)
      for (dep_id in csort(dependents_of[[cid]])) {
        indegree[[dep_id]] <- indegree[[dep_id]] - 1
        if (indegree[[dep_id]] == 0) ready <- c(ready, dep_id)
      }
      ready <- csort(ready)
    }
    if (length(order) != length(closure))
      stop("dependency graph has a cycle", call. = FALSE)
    order
  }

  # -- internals ------------------------------------------------------------------

  # Propagate staleness to transitive dependents (no recompute).
  eng$.mark_stale <- function(cell_id, seen) {
    for (dep_id in eng$.cell(cell_id)$dependents) {
      if (dep_id %in% seen) next
      seen <- c(seen, dep_id)
      dc <- eng$.cell(dep_id)
      if (identical(dc$kind, "formula")) {
        dc$stale <- TRUE
        eng$cells[[dep_id]] <- dc
      }
      eng$.mark_stale(dep_id, seen)
    }
    invisible(NULL)
  }

  # Evaluate a formula against a snapshot of its dependencies.
  eng$.recompute <- function(cell_id, ts) {
    cell <- eng$.cell(cell_id)
    if (is.null(cell$ast)) {
      cell$value <- NULL
      cell$has_value <- TRUE
      cell$status <- "error"
      cell$error <- paste0("formula does not compile: '", cell$extra$expr, "'")
      cell$stale <- FALSE
      eng$cells[[cell_id]] <- cell
      return(list(data = NULL, status = "error", error = cell$error))
    }
    snap <- new.env(parent = emptyenv())
    resolve <- function(name) {
      if (!exists(name, envir = snap, inherits = FALSE)) {
        if (is.null(eng$cells[[name]])) stop("unknown cell: ", name, call. = FALSE)
        assign(name, eng$get(name, ts)$data, envir = snap)
      }
      get(name, envir = snap)
    }
    data <- tryCatch(formula_eval(cell$ast, resolve), error = function(e) e)
    if (inherits(data, "error")) {
      cell$value <- NULL
      cell$has_value <- TRUE
      cell$status <- "error"
      cell$error <- conditionMessage(data)
      cell$stale <- FALSE
      eng$cells[[cell_id]] <- cell
      return(list(data = NULL, status = "error", error = cell$error))
    }
    cell$value <- data
    cell$has_value <- TRUE
    cell$status <- "ready"
    cell$error <- NULL
    cell$stale <- FALSE
    eng$cells[[cell_id]] <- cell
    if (eng$record_edges) {
      # Input posting: the dependency snapshot in dependency-address order.
      inputs <- Filter(function(x) !is.null(x),
                       lapply(csort(cell$deps), function(d)
                         if (exists(d, envir = snap, inherits = FALSE))
                           get(d, envir = snap) else NULL))
      eng$ledgers[[cell_id]]$record(inputs, data, ts, list(origin = "get"))
    }
    list(data = data, status = "ready", error = NULL)
  }

  eng
}
