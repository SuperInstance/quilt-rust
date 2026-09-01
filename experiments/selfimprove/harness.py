#!/usr/bin/env python3
"""selfimprove harness — Liquid-model-driven mutation loop over the quilt runtime.

Loop (one generation):
  1. MUTATE   Liquid-LFM2.5-2.6B proposes K candidate test scenarios (JSON).
              Mode alternates: blind (spec + examples only) vs targeted
              (also shown live-mutant diffs to aim at).
  2. FILTER   LiquidAI/lfm2.5-1.2b-instruct triages each candidate
              (keep_for_eval + reason) — cheap gate.
  3. EVALUATE candidates run against the CLEAN probe binary (the real
              QuiltEngine). A candidate is VALID if the model's `expect`
              matches engine reality (final read, numeric-tolerant;
              expect=="ERROR" matches a failing final step).
  4. VALUE    a valid candidate is KEPT iff it kills >= 1 live mutant the
              current corpus does not (divergent `reads` on a mutant probe).
  5. LOG      every candidate lands in results/gen-N.json; every generation
              appends to GENERATIONS.md; corpus changes commit to the branch.

Safety: all work in this worktree, branch selfimprove-harness. Never pushes.
Mutant application always reverts in `finally`. subprocess = list form only.
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import time
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parent          # experiments/selfimprove
WORKTREE = ROOT.parents[1]                       # quilt worktree root
PROBE_DIR = ROOT / "probe"
BIN = ROOT / "bin"
CORPUS = ROOT / "corpus.json"
MUTANTS = ROOT / "mutants.json"
RESULTS = ROOT / "results"
STATE = ROOT / "state.json"
GENLOG = ROOT / "GENERATIONS.md"
SETUP = ROOT / "setup.json"

OLLAMA = os.environ.get("OLLAMA_URL", "http://127.0.0.1:11434")
MUTATOR = os.environ.get("MUTATOR_MODEL", "Liquid-LFM2.5-2.6B:latest")
TRIAGER = os.environ.get("TRIAGER_MODEL", "LiquidAI/lfm2.5-1.2b-instruct:latest")

TOL = 1e-9
CANDIDATES_PER_GEN = 4


# ---------------------------------------------------------------- utilities

def run(cmd: list[str], timeout: int = 120, cwd: Path | None = None,
        input_text: str | None = None) -> subprocess.CompletedProcess:
    """List-form subprocess only. No shell, ever."""
    return subprocess.run(cmd, cwd=cwd, input=input_text, capture_output=True,
                          text=True, timeout=timeout)


def atomic_write(path: Path, text: str) -> None:
    tmp = path.with_suffix(path.suffix + f".tmp{uuid.uuid4().hex[:8]}")
    tmp.write_text(text)
    os.replace(tmp, path)


def jdump(path: Path, obj) -> None:
    atomic_write(path, json.dumps(obj, indent=1, ensure_ascii=False) + "\n")


def chat(model: str, system: str, user: str, json_mode: bool, temp: float,
         max_retries: int = 2, num_predict: int = 1600) -> str:
    body = {
        "model": model,
        "messages": [{"role": "system", "content": system},
                     {"role": "user", "content": user}],
        "stream": False,
        "options": {"temperature": temp, "num_ctx": 8192,
                    "num_predict": num_predict},
    }
    if json_mode:
        body["format"] = "json"
    req = urllib.request.Request(
        OLLAMA + "/api/chat",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    last_err = None
    for _ in range(max_retries + 1):
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                return json.loads(r.read().decode())["message"]["content"]
        except Exception as e:  # noqa: BLE001 — log & retry
            last_err = e
            time.sleep(3)
    raise RuntimeError(f"ollama chat failed ({model}): {last_err}")


def extract_json(text: str):
    """Pull the outermost JSON array or object out of a model reply."""
    text = re.sub(r"```(json)?", "", text)
    for a, b in (("[", "]"), ("{", "}")):
        i, j = text.find(a), text.rfind(b)
        if i != -1 and j > i:
            try:
                return json.loads(text[i:j + 1])
            except json.JSONDecodeError:
                continue
    return None


# ---------------------------------------------------------------- compare

def val_eq(a, b) -> bool:
    """JSON-equal with numeric tolerance (int/float cross-equal)."""
    if isinstance(a, bool) or isinstance(b, bool):
        return a is b if isinstance(a, bool) and isinstance(b, bool) else a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return abs(a - b) <= TOL * max(1.0, abs(a), abs(b))
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(val_eq(x, y) for x, y in zip(a, b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(val_eq(a[k], b[k]) for k in a)
    return a == b


def reads_diverge(clean_res, mut_res) -> bool:
    """A mutant diverges on a case if reads/statuses differ, it errored, or panicked."""
    if mut_res.get("error") or not mut_res.get("ok", False):
        return True  # error/panic on a case the clean engine handled
    if not val_eq(clean_res.get("reads", []), mut_res.get("reads", [])):
        return True
    return not val_eq(clean_res.get("statuses", []), mut_res.get("statuses", []))


# ---------------------------------------------------------------- probe

def run_probe(binary: Path, cases: list[dict]) -> list[dict]:
    payload = [{"id": c["id"], "cells": c.get("cells", []),
                "script": c.get("script", [])} for c in cases]
    p = run([str(binary)], timeout=120, input_text=json.dumps(payload))
    if p.returncode != 0:
        raise RuntimeError(f"probe {binary.name} failed: {p.stderr[:400]}")
    return {r["id"]: r for r in json.loads(p.stdout)}


def case_valid(case: dict, clean_res: dict) -> tuple[bool, str]:
    """Model's expectation vs engine reality (final read)."""
    expect = case.get("expect", None)
    reads = clean_res.get("reads", [])
    if expect == "ERROR":
        ok = (not clean_res.get("ok", False))
        return ok, "expect ERROR" if ok else "engine succeeded, model expected error"
    if not reads:
        return False, "no read ops in script"
    if not clean_res.get("ok", False) and expect is not None:
        return False, "engine errored on case"
    return val_eq(reads[-1], expect), "match" if val_eq(reads[-1], expect) else \
        f"engine={json.dumps(reads[-1])[:120]} model_expect={json.dumps(expect)[:120]}"


# ---------------------------------------------------------------- mutants

def apply_mutant(m: dict) -> None:
    f = WORKTREE / m["file"]
    src = f.read_text()
    if src.count(m["find"]) != 1:
        raise RuntimeError(f"find-string not unique in {m['file']} "
                           f"(count={src.count(m['find'])})")
    f.write_text(src.replace(m["find"], m["replace"]))


def revert_mutant(m: dict) -> None:
    run(["git", "-C", str(WORKTREE), "checkout", "--", m["file"]], timeout=60)


def setup_mutants(force: bool = False) -> dict:
    """Build probe binaries for every mutant + measure battery kills.

    Battery kill = `cargo test -p quilt-core` fails (or fails to build)
    with the mutant applied. Probe binaries cached in bin/.
    """
    BIN.mkdir(exist_ok=True)
    mutants = json.loads(MUTANTS.read_text())["mutants"]
    setup = json.loads(SETUP.read_text()) if SETUP.exists() else {}
    # clean probe first
    clean_bin = BIN / "clean"
    if force or not clean_bin.exists():
        p = run(["cargo", "build", "--release"], cwd=PROBE_DIR, timeout=900)
        if p.returncode != 0:
            raise RuntimeError("clean probe build failed:\n" + p.stderr[-800:])
        clean_bin.write_bytes((PROBE_DIR / "target/release/selfimprove-probe").read_bytes())
        clean_bin.chmod(0o755)
    setup.setdefault("_clean", {"built": True})

    for m in mutants:
        mid = m["id"]
        if not force and mid in setup and setup[mid].get("done"):
            continue
        entry = {"done": False, "id": mid, "desc": m["description"]}
        t0 = time.time()
        try:
            apply_mutant(m)
            try:
                p = run(["cargo", "build", "--release"], cwd=PROBE_DIR, timeout=900)
                if p.returncode != 0:
                    entry.update(battery_killed=True, battery_reason="probe-compile-fail",
                                 probe=False)
                else:
                    (BIN / mid).write_bytes(
                        (PROBE_DIR / "target/release/selfimprove-probe").read_bytes())
                    (BIN / mid).chmod(0o755)
                    entry.update(probe=True)
                    prev = setup.get(mid, {})
                    if os.environ.get("SKIP_BATTERY") and "battery_killed" in prev:
                        entry.update(battery_killed=prev["battery_killed"],
                                     battery_reason=prev.get("battery_reason", "reused"))
                    else:
                        # battery verdict on the real test suite
                        t = run(["cargo", "test", "-p", "quilt-core", "--quiet"],
                                cwd=WORKTREE, timeout=900)
                        failed = t.returncode != 0
                        tail = (t.stdout + t.stderr).strip().splitlines()
                        entry.update(battery_killed=failed,
                                     battery_reason=("tests-fail: " + "; ".join(
                                         [l for l in tail if "FAILED" in l or "panicked" in l][:3])
                                         or "output-differs") if failed else "tests-pass",
                                     battery_tail=tail[-6:])
            finally:
                revert_mutant(m)
            entry["done"] = True
        except Exception as e:  # noqa: BLE001
            entry.update(done=True, error=str(e)[:300])
            with open(os.devnull, "w") as devnull:  # ensure revert attempted
                pass
            try:
                revert_mutant(m)
            except Exception:
                pass
        entry["seconds"] = round(time.time() - t0, 1)
        setup[mid] = entry
        jdump(SETUP, setup)
        print(f"[setup] {mid}: battery_killed={entry.get('battery_killed')} "
              f"probe={entry.get('probe')} ({entry['seconds']}s)", flush=True)
    return setup


# ---------------------------------------------------------------- kill model

def corpus_kills(corpus: list[dict], mutants: list[dict], setup: dict):
    """Which mutants does the corpus kill? Only valid cases count."""
    clean = run_probe(BIN / "clean", corpus)
    valid_ids = set()
    for c in corpus:
        ok, _ = case_valid(c, clean[c["id"]])
        if ok:
            valid_ids.add(c["id"])
    kills: dict[str, list[str]] = {}
    for m in mutants:
        mid = m["id"]
        if not setup.get(mid, {}).get("probe"):
            continue
        if setup.get(mid, {}).get("battery_killed"):
            continue  # dead to the battery: no value in killing it again
        res = run_probe(BIN / mid, corpus)
        killed_by = [c["id"] for c in corpus
                     if c["id"] in valid_ids
                     and reads_diverge(clean[c["id"]], res[c["id"]])]
        if killed_by:
            kills[mid] = killed_by
    return kills, valid_ids, clean


def signature(c: dict) -> str:
    return json.dumps([c.get("cells"), c.get("script")], sort_keys=True)


# ---------------------------------------------------------------- prompts

SPEC = """You write test cases for a reactive spreadsheet engine (Quilt). Cells are value cells (static JSON value) or formula cells (rhai expression). A formula references other cells by bare id; ids are auto-rewritten to cells["id"]. You may also write cells["id"] explicitly. Available expression features: + - * / % == != > < >= <= && || ! if/else blocks used as expressions, string concatenation with +, array literals [a, b, c], and helper functions abs(x), min(a,b), max(a,b), min([..]), max([..]), clamp(x, lo, hi). IMPORTANT engine semantics learned so far: many failure modes (null arithmetic, division by zero, type mismatches) produce NULL results rather than errors — reserve expect "ERROR" for parse/compile failures like unclosed brackets, and prefer predicting the actual value or null. Integer overflow wraps silently in i64. Integer division returns an integer.

A test case is JSON:
{"id": "...", "cells": [{"id":"a","kind":"value","value":2},{"id":"f","kind":"formula","expr":"a * 3","deps":["a"]}], "script": [{"op":"get","id":"f"},{"op":"set","id":"a","value":5},{"op":"call","id":"f"}], "expect": <final read value, or "ERROR">, "why": "one line"}
ops: get (evaluate+read), set (write a value, triggers recompute of dependents), call (like get but uses the caller-context cache), peek (reads the STORED value and status without evaluating — statuses are "ready", "stale", "error"). The LAST get/call/peek in the script is compared to "expect"."""


def mutate_prompt(mode: str, corpus: list[dict], live: list[dict]) -> tuple[str, str]:
    examples = json.dumps(corpus[-6:], ensure_ascii=False)
    sys_p = "You are a meticulous test engineer. Reply with a JSON array only."
    if mode == "blind":
        user = f"""{SPEC}

Existing corpus (do not duplicate, go BEYOND them — explore edge cases: operator precedence, negatives, zero, floats vs ints, null, strings with cell-id names inside, dotted cell ids like compass.heading, formulas with a leading '=', deep chains, staleness after set, cache behavior across call/set/call):
{examples}

Propose {CANDIDATES_PER_GEN} NEW test cases as a JSON array. Output ONLY the array."""
    else:
        tgt = "\n\n".join(
            f"- {m['id']}: {m['description']}\n  change: {m['find']!r} -> {m['replace']!r}"
            for m in live[:6])
        user = f"""{SPEC}

Existing corpus (do not duplicate):
{examples}

The codebase currently contains these BUGS (mutants). Write test cases whose result would DIFFER under each bug — i.e. cases that detect them:
{tgt}

Propose {CANDIDATES_PER_GEN} test cases as a JSON array, each aimed at detecting one or more of the bugs. Output ONLY the array."""
    return sys_p, user


TRIAGE_SYS = "You are a strict test reviewer. Reply with a single JSON object only."

def triage_prompt(cand: dict, sigs: set[str]) -> tuple[str, str]:
    dupe = signature(cand) in sigs
    user = f"""Candidate test case:
{json.dumps(cand, ensure_ascii=False)[:1500]}

Corpus already contains an identical case: {dupe}.
Judge: (1) is it syntactically complete (id, cells, script, expect, why)? (2) is it a duplicate or trivial variant? (3) does it exercise real engine behavior (operators, helpers, reactivity, caching, error paths)?
Reply JSON: {{"keep_for_eval": true|false, "reason": "short reason"}}"""
    return TRIAGE_SYS, user


# ---------------------------------------------------------------- generation

def one_generation(g: int, corpus: list[dict], mutants: list[dict], setup: dict):
    mode = "blind" if g % 2 == 1 else "targeted"
    live = [m for m in mutants
            if setup.get(m["id"], {}).get("probe")
            and not setup.get(m["id"], {}).get("battery_killed")]
    kills_before, valid_ids, clean = corpus_kills(corpus, mutants, setup)
    live_ids = [m["id"] for m in live]
    live_killed_before = [k for k in kills_before if k in live_ids]

    rec = {"gen": g, "mode": mode, "candidates": [], "kept": [],
           "live_mutants": live_ids, "killed_before": live_killed_before}

    # 1. MUTATE
    sys_p, user = mutate_prompt(mode, corpus, live)
    raw = ""
    try:
        for npred in (12000,):
            raw = chat(MUTATOR, sys_p, user, json_mode=False,
                       temp=0.7 if mode == "blind" else 0.4, num_predict=npred)
            if extract_json(raw) is not None:
                break
    except Exception as e:  # noqa: BLE001
        rec["mutate_error"] = str(e)[:300]
        return rec, corpus, kills_before
    cands = extract_json(raw)
    if not isinstance(cands, list):
        cands = [cands] if isinstance(cands, dict) else []
    rec["mutator_raw_len"] = len(raw)

    sigs = {signature(c) for c in corpus}

    # 2/3/4. FILTER -> EVALUATE -> VALUE
    kept = []
    for i, cand in enumerate(cands[:CANDIDATES_PER_GEN * 2]):
        if not isinstance(cand, dict) or "cells" not in cand or "script" not in cand:
            continue
        cand = dict(cand)
        cand["id"] = f"g{g}-{i}"
        crec = {"id": cand["id"], "summary": (cand.get("why") or "")[:140],
                "expr": next((c.get("expr") for c in cand.get("cells", [])
                              if c.get("kind") == "formula"), "")}
        # FILTER
        try:
            t_raw = chat(TRIAGER, *triage_prompt(cand, sigs), json_mode=True, temp=0.0)
            t = extract_json(t_raw) or {}
            crec["triage_keep"] = bool(t.get("keep_for_eval"))
            crec["triage_reason"] = str(t.get("reason", ""))[:160]
        except Exception as e:  # noqa: BLE001
            crec["triage_keep"] = True
            crec["triage_reason"] = f"triager-failed ({str(e)[:80]}) — default pass"
        if not crec["triage_keep"]:
            crec["filter_flagged"] = True  # advisory: eval is cheap, gate is weak
        if signature(cand) in sigs:
            crec["verdict"] = "REJECTED-duplicate"
            rec["candidates"].append(crec)
            continue
        # EVALUATE (clean)
        try:
            cres = run_probe(BIN / "clean", [cand])[cand["id"]]
        except Exception as e:  # noqa: BLE001
            crec["verdict"] = f"ERROR-probe {str(e)[:100]}"
            rec["candidates"].append(crec)
            continue
        ok, why = case_valid(cand, cres)
        crec["engine_result"] = (cres.get("reads") or [None])[-1]
        crec["valid"] = ok
        crec["validity_note"] = why[:200]
        if not ok:
            crec["verdict"] = "REJECTED-invalid-expectation"
            rec["candidates"].append(crec)
            continue
        # VALUE (kills new live mutants?)
        new_kills = []
        for mid in live_ids:
            if mid in kills_before:
                continue
            try:
                mres = run_probe(BIN / mid, [cand])[cand["id"]]
            except Exception:  # noqa: BLE001
                continue
            if reads_diverge(cres, mres):
                new_kills.append(mid)
        crec["new_kills"] = new_kills
        if new_kills:
            crec["verdict"] = "KEPT"
            entry = {"id": cand["id"], "cells": cand["cells"],
                     "script": cand["script"], "expect": cand.get("expect"),
                     "why": crec["summary"], "origin": f"gen-{g}",
                     "kills": new_kills}
            kept.append(entry)
            corpus.append(entry)
            sigs.add(signature(entry))
            for mid in new_kills:
                kills_before.setdefault(mid, []).append(cand["id"])
        else:
            crec["verdict"] = "REJECTED-no-new-kills"
        rec["candidates"].append(crec)

    rec["kept"] = [k["id"] for k in kept]
    rec["killed_after"] = [k for k in kills_before if k in live_ids]
    return rec, corpus, kills_before


# ---------------------------------------------------------------- main

def append_genlog(rec: dict, setup: dict) -> None:
    g = rec["gen"]
    lines = [
        f"\n## Generation {g} — {rec['mode']} — {time.strftime('%Y-%m-%d %H:%M:%S')}",
        f"- live mutants: {len(rec.get('live_mutants', []))} | corpus kills before: "
        f"{len(rec.get('killed_before', []))} | after: {len(rec.get('killed_after', []))}",
    ]
    for c in rec["candidates"]:
        lines.append(
            f"- `{c['id']}` [{c.get('verdict', '?')}] {c.get('expr', '')[:60]!r} — "
            f"triage={c.get('triage_keep')} ({c.get('triage_reason', '')[:80]}) — "
            f"{c.get('validity_note', '')[:100]} "
            f"{'kills=' + ','.join(c.get('new_kills', [])) if c.get('new_kills') else ''}")
    if rec.get("kept"):
        lines.append(f"- **KEPT: {', '.join(rec['kept'])}**")
    with open(GENLOG, "a") as f:
        f.write("\n".join(lines) + "\n")


def git_commit(msg: str) -> None:
    run(["git", "-C", str(WORKTREE), "add",
         "experiments/selfimprove"], timeout=60)
    run(["git", "-C", str(WORKTREE), "commit", "-m", msg, "--no-verify",
         "--quiet"], timeout=120)


def observe_seeds(corpus: list[dict]) -> list[dict]:
    """Fill missing `expect` from clean-engine observation (fixture truth)."""
    need = [c for c in corpus if "expect" not in c]
    if not need:
        return corpus
    res = run_probe(BIN / "clean", need)
    for c in need:
        r = res[c["id"]]
        c["expect"] = "ERROR" if not r.get("ok") else \
            (r.get("reads") or [None])[-1]
        c["origin"] = "seed"
    return corpus


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "run"
    if cmd == "setup":
        setup_mutants(force="--force" in sys.argv)
        return
    if cmd == "status":
        corpus = json.loads(CORPUS.read_text())
        mutants = json.loads(MUTANTS.read_text())["mutants"]
        setup = json.loads(SETUP.read_text()) if SETUP.exists() else {}
        kills, valid, _ = corpus_kills(corpus, mutants, setup)
        live = [m["id"] for m in mutants if setup.get(m["id"], {}).get("probe")
                and not setup.get(m["id"], {}).get("battery_killed")]
        print(f"corpus={len(corpus)} valid={len(valid)} live={len(live)} "
              f"killed={sorted(kills)}")
        return

    assert cmd == "run"
    max_gens = int(os.environ.get("MAX_GENS", "40"))
    max_wall = int(os.environ.get("MAX_WALL_S", str(3600 * 4)))
    t0 = time.time()
    mutants = json.loads(MUTANTS.read_text())["mutants"]
    setup = json.loads(SETUP.read_text()) if SETUP.exists() else {}
    if not (BIN / "clean").exists() or not setup:
        print("run `harness.py setup` first", file=sys.stderr)
        sys.exit(1)

    corpus = observe_seeds(json.loads(CORPUS.read_text()))
    jdump(CORPUS, corpus)
    state = json.loads(STATE.read_text()) if STATE.exists() else {"gen": 0}

    g = state["gen"]
    while g < max_gens and time.time() - t0 < max_wall:
        g += 1
        print(f"\n=== GENERATION {g} (elapsed {time.time()-t0:.0f}s) ===", flush=True)
        try:
            rec, corpus, kills = one_generation(g, corpus, mutants, setup)
        except Exception as e:  # noqa: BLE001
            print(f"generation {g} crashed: {e}", flush=True)
            with open(GENLOG, "a") as f:
                f.write(f"\n## Generation {g} — CRASHED: {str(e)[:300]}\n")
            break
        RESULTS.mkdir(exist_ok=True)
        jdump(RESULTS / f"gen-{g}.json", rec)
        append_genlog(rec, setup)
        jdump(CORPUS, corpus)
        jdump(STATE, {"gen": g,
                      "kept_total": len([c for c in corpus if c.get("origin", "").startswith("gen")]),
                      "killed": sorted(kills)})
        keptn = len(rec.get("kept", []))
        git_commit(
            f"selfimprove(gen {g}, {rec['mode']}): kept {keptn} case(s); "
            f"corpus kills {len(rec.get('killed_before', []))}"
            f"->{len(rec.get('killed_after', []))}")
        print(f"kept={keptn} kills={len(rec.get('killed_before', []))}"
              f"->{len(rec.get('killed_after', []))}", flush=True)
    print(f"\ndone: {g} generations, {time.time()-t0:.0f}s wall", flush=True)


if __name__ == "__main__":
    main()
