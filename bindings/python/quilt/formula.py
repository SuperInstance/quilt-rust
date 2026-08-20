"""formula — the quilt formula expression language, evaluated in pure Python.

Formulas are JavaScript-flavored expressions (the TS engine evaluates
them with `new Function`; the Rust engine with rhai). This module is a
small recursive-descent evaluator for the shared core::

    literals     42  2.5  1e3  'str'  "str"  true  false  null
    cell refs    a  compass.heading      (bare cell ids, resolved by the engine)
    operators    + - * / %  ( )  < > <= >= == !=  && || !  ?:
    helpers      abs(x)  min(a,b)  max(a,b)  clamp(n, lo, hi)

Conformance decisions (documented for the compat contract):

* **Division is real (IEEE) division** — `21 * 9 / 5 + 32` is `69.8`,
  matching the TS engine and the sheet examples. (rhai would
  integer-divide; that is a known Rust-side quirk, not the reference.)
* **`%` is the JS/rhai truncated remainder** (sign of the dividend,
  `math.fmod`), NOT Python's floored `%`: `-7 % 360 == -7`.
* **Int/float distinction is preserved** — `2 + 3` is int `5`, but
  `2.0 + 3` is float `5.0`. This matters on-chain: the canonical JSON
  renders `5` and `5.0` differently (see ledger.fmt_float and
  docs/cell-ledger.md §4's float-vs-int hazard note).
* `+` with a string operand concatenates (JS coercion for the common
  `=hello + ', world!'` idiom); `==` is strict about bool/number/string
  kinds; `&&`/`||` short-circuit and return the deciding operand.
* Division by zero is an evaluation error (rhai behavior; JS would
  return Infinity — both "didn't crash" per the Rust engine's own
  test for this case).
"""

from __future__ import annotations

import math


class FormulaError(Exception):
    """A formula could not be parsed or evaluated."""


# ---------------------------------------------------------------------------
# Tokenizer
# ---------------------------------------------------------------------------

_TWO_CHAR = ("<=", ">=", "==", "!=", "&&", "||")
_ONE_CHAR = set("+-*/%()<>,!?:")


def tokenize(src: str) -> list[tuple]:
    """Tokenize an expression into (kind, value) tuples.

    kinds: num | str | id | op. Cell references arrive as `id` tokens;
    the engine resolves them.
    """
    toks: list[tuple] = []
    i, n = 0, len(src)
    while i < n:
        ch = src[i]
        if ch in " \t\r\n":
            i += 1
            continue
        if ch in ("'", '"'):
            j = i + 1
            buf = []
            while j < n and src[j] != ch:
                if src[j] == "\\" and j + 1 < n:
                    esc = src[j + 1]
                    buf.append({"n": "\n", "t": "\t", "r": "\r"}.get(esc, esc))
                    j += 2
                else:
                    buf.append(src[j])
                    j += 1
            if j >= n:
                raise FormulaError(f"unterminated string in formula: {src!r}")
            toks.append(("str", "".join(buf)))
            i = j + 1
            continue
        if ch.isdigit() or (ch == "." and i + 1 < n and src[i + 1].isdigit()):
            j = i
            seen_dot = seen_exp = False
            while j < n:
                c = src[j]
                if c.isdigit():
                    j += 1
                elif c == "." and not seen_dot and not seen_exp:
                    seen_dot = True
                    j += 1
                elif c in "eE" and not seen_exp and j > i:
                    seen_exp = True
                    j += 1
                    if j < n and src[j] in "+-":
                        j += 1
                else:
                    break
            text = src[i:j]
            if seen_dot or (seen_exp and "e" in text.lower()):
                toks.append(("num", float(text)))
            else:
                toks.append(("num", int(text)))
            i = j
            continue
        if ch.isalpha() or ch == "_":
            j = i
            while j < n and (src[j].isalnum() or src[j] in "_."):
                j += 1
            toks.append(("id", src[i:j]))
            i = j
            continue
        if src[i : i + 2] in _TWO_CHAR:
            toks.append(("op", src[i : i + 2]))
            i += 2
            continue
        if ch in _ONE_CHAR:
            toks.append(("op", ch))
            i += 1
            continue
        raise FormulaError(f"unexpected character {ch!r} in formula: {src!r}")
    return toks


# ---------------------------------------------------------------------------
# AST — tuples: ('num', v) ('str', v) ('ref', name) ('un', op, a)
#   ('bin', op, a, b) ('ter', c, a, b) ('call', name, [args])
# ---------------------------------------------------------------------------


def parse(src: str):
    """Parse an expression source (with optional leading `=`)."""
    body = src[1:] if src.startswith("=") else src
    toks = tokenize(body)
    if not toks:
        raise FormulaError("empty formula")
    p = _Parser(toks)
    ast = p.ternary()
    p.expect_end()
    return ast


class _Parser:
    def __init__(self, toks):
        self.toks = toks
        self.i = 0

    def peek(self):
        return self.toks[self.i] if self.i < len(self.toks) else (None, None)

    def next(self):
        tok = self.peek()
        self.i += 1
        return tok

    def eat_op(self, *ops):
        kind, val = self.peek()
        if kind == "op" and val in ops:
            self.i += 1
            return val
        return None

    def expect_end(self):
        if self.i < len(self.toks):
            raise FormulaError(f"trailing tokens at {self.toks[self.i]!r}")

    # precedence: ternary < or < and < equality < relational < additive
    #             < multiplicative < unary < primary
    def ternary(self):
        cond = self.or_()
        if self.eat_op("?"):
            a = self.ternary()
            if not self.eat_op(":"):
                raise FormulaError("expected ':' in ternary")
            b = self.ternary()
            return ("ter", cond, a, b)
        return cond

    def or_(self):
        left = self.and_()
        while self.eat_op("||"):
            left = ("bin", "||", left, self.and_())
        return left

    def and_(self):
        left = self.equality()
        while self.eat_op("&&"):
            left = ("bin", "&&", left, self.equality())
        return left

    def equality(self):
        left = self.relational()
        while True:
            op = self.eat_op("==", "!=")
            if not op:
                return left
            left = ("bin", op, left, self.relational())

    def relational(self):
        left = self.additive()
        while True:
            op = self.eat_op("<", ">", "<=", ">=")
            if not op:
                return left
            left = ("bin", op, left, self.additive())

    def additive(self):
        left = self.multiplicative()
        while True:
            op = self.eat_op("+", "-")
            if not op:
                return left
            left = ("bin", op, left, self.multiplicative())

    def multiplicative(self):
        left = self.unary()
        while True:
            op = self.eat_op("*", "/", "%")
            if not op:
                return left
            left = ("bin", op, left, self.unary())

    def unary(self):
        if self.eat_op("!"):
            return ("un", "!", self.unary())
        if self.eat_op("-"):
            return ("un", "-", self.unary())
        if self.eat_op("+"):
            return self.unary()
        return self.primary()

    def primary(self):
        kind, val = self.next()
        if kind == "num":
            return ("num", val)
        if kind == "str":
            return ("str", val)
        if kind == "id":
            if val == "true":
                return ("num", True)
            if val == "false":
                return ("num", False)
            if val == "null":
                return ("num", None)
            if self.eat_op("("):
                args = []
                if not self.eat_op(")"):
                    while True:
                        args.append(self.ternary())
                        if self.eat_op(","):
                            continue
                        if self.eat_op(")"):
                            break
                        raise FormulaError("expected ',' or ')' in call")
                return ("call", val, args)
            return ("ref", val)
        if kind == "op" and val == "(":
            inner = self.ternary()
            if not self.eat_op(")"):
                raise FormulaError("expected ')'")
            return inner
        raise FormulaError(f"unexpected token {(kind, val)!r}")


def compile_expr(src: str):
    """Parse once, return an evaluator closure `f(resolve) -> value`.

    `resolve(name)` maps a cell id to its current value (raises or
    returns None for unknown cells — the engine decides).
    """
    ast = parse(src)

    def evaluate(resolve):
        return _eval(ast, resolve)

    return evaluate


# ---------------------------------------------------------------------------
# Evaluation — JS-flavored semantics
# ---------------------------------------------------------------------------


def _is_num(v) -> bool:
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def _truthy(v) -> bool:
    if v is None or v is False:
        return False
    if v == 0 and _is_num(v):
        return False
    if isinstance(v, str) and v == "":
        return False
    return True


def _to_str(v) -> str:
    if v is None:
        return "null"
    if v is True:
        return "true"
    if v is False:
        return "false"
    if isinstance(v, float):
        return fmt_float_js(v)
    if isinstance(v, int):
        return str(v)
    return str(v)


def fmt_float_js(x: float) -> str:
    """Render a float the way JS `String(x)` would (for `+` concat)."""
    if x == int(x) and abs(x) < 1e21:
        return str(int(x))
    return repr(x)


def _numeric(a, b, op: str):
    if not _is_num(a) or not _is_num(b):
        raise FormulaError(f"'{op}' on non-numbers: {a!r} {op} {b!r}")
    both_int = isinstance(a, int) and isinstance(b, int)
    if op == "+":
        return a + b
    if op == "-":
        return a - b
    if op == "*":
        return a * b
    if op == "/":
        if b == 0:
            raise FormulaError("division by zero")
        r = a / b  # true division always (JS semantics)
        return r
    if op == "%":
        if b == 0:
            raise FormulaError("modulo by zero")
        r = math.fmod(a, b)  # JS truncated remainder
        if both_int:
            return int(r)
        return r
    raise FormulaError(f"unknown operator {op}")


def _equals(a, b) -> bool:
    if isinstance(a, bool) or isinstance(b, bool):
        return isinstance(a, bool) and isinstance(b, bool) and a == b
    if _is_num(a) and _is_num(b):
        return float(a) == float(b)
    if type(a) is not type(b):
        return False
    return a == b


def _compare(op: str, a, b) -> bool:
    if _is_num(a) and _is_num(b):
        a, b = float(a), float(b)
    elif isinstance(a, str) and isinstance(b, str):
        pass
    else:
        raise FormulaError(f"'{op}' on incomparable values: {a!r}, {b!r}")
    if op == "<":
        return a < b
    if op == ">":
        return a > b
    if op == "<=":
        return a <= b
    return a >= b


def _call_helper(name: str, args, resolve):
    def num(v):
        if not _is_num(v):
            raise FormulaError(f"{name}() expects numbers, got {v!r}")
        return v

    if name == "abs":
        if len(args) != 1:
            raise FormulaError("abs() takes one argument")
        return abs(num(args[0]))
    if name in ("min", "max"):
        vals = [num(v) for v in args]
        if not vals:
            raise FormulaError(f"{name}() requires at least one argument")
        pick = min(vals) if name == "min" else max(vals)
        if all(isinstance(v, int) for v in vals):
            return int(pick)
        return pick
    if name == "clamp":
        if len(args) != 3:
            raise FormulaError("clamp(n, lo, hi) takes three arguments")
        n, lo, hi = num(args[0]), num(args[1]), num(args[2])
        if n < lo:
            return lo
        if n > hi:
            return hi
        return n
    raise FormulaError(f"unknown function {name}()")


def _eval(node, resolve):
    tag = node[0]
    if tag == "num":
        return node[1]
    if tag == "str":
        return node[1]
    if tag == "ref":
        return resolve(node[1])
    if tag == "un":
        val = _eval(node[2], resolve)
        if node[1] == "-":
            if not _is_num(val):
                raise FormulaError(f"unary '-' on non-number {val!r}")
            return -val
        return not _truthy(val)
    if tag == "ter":
        return _eval(node[2] if _truthy(_eval(node[1], resolve)) else node[3], resolve)
    if tag == "call":
        args = [_eval(a, resolve) for a in node[2]]
        return _call_helper(node[1], args, resolve)
    if tag == "bin":
        op = node[1]
        if op == "&&":
            left = _eval(node[2], resolve)
            return _eval(node[3], resolve) if _truthy(left) else left
        if op == "||":
            left = _eval(node[2], resolve)
            return left if _truthy(left) else _eval(node[3], resolve)
        a = _eval(node[2], resolve)
        b = _eval(node[3], resolve)
        if op == "+":
            if isinstance(a, str) or isinstance(b, str):
                return _to_str(a) + _to_str(b)
            return _numeric(a, b, "+")
        if op in ("-", "*", "/", "%"):
            return _numeric(a, b, op)
        if op == "==":
            return _equals(a, b)
        if op == "!=":
            return not _equals(a, b)
        if op in ("<", ">", "<=", ">="):
            return _compare(op, a, b)
    raise FormulaError(f"bad AST node {node!r}")
