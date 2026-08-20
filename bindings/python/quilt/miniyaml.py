"""miniyaml — a stdlib-only YAML subset parser, sufficient for quilt sheets.

quilt sheets are a small, regular YAML dialect: a top-level mapping
(`id`, `version`, `title`, `description`, `axes`, ...) whose `cells`
key holds a list of flat mappings. This parser handles exactly that
shape plus the bits real sheets use:

* block mappings and block sequences by indentation,
* `- key: value` list items with continuation keys,
* inline flow sequences `[a, b]` and flow mappings `{name: boat}`,
* block scalars `|` (literal, used by program cells / descriptions),
* comments (`#` outside quotes), blank lines, quoted scalars,
* scalar typing: int / float / bool / null / string.

It is NOT a general YAML parser (no anchors, tags, multi-line flow
collections, or exotic block chomping). Unknown shapes ride along as
raw strings; the engine only consumes `id`, `kind`, `value`, `expr`,
`default`, `source`, and `watch`.
"""

from __future__ import annotations

import re


class ParseError(Exception):
    """Raised when the YAML subset is violated."""


_INT_RE = re.compile(r"^[+-]?\d+$")
_FLOAT_RE = re.compile(
    r"^[+-]?(\d+\.\d*|\.\d+|\d+)([eE][+-]?\d+)?$"
)


def _scalar(text: str):
    """Type a scalar token the way YAML does for the sheet dialect."""
    s = text.strip()
    if s == "":
        return None
    if len(s) >= 2 and s[0] == s[-1] and s[0] in ("'", '"'):
        inner = s[1:-1]
        if s[0] == '"':
            inner = (
                inner.replace('\\"', '"')
                .replace("\\\\", "\\")
                .replace("\\n", "\n")
                .replace("\\t", "\t")
            )
        else:
            inner = inner.replace("''", "'")
        return inner
    if s in ("null", "~", "Null", "NULL"):
        return None
    if s in ("true", "True", "TRUE"):
        return True
    if s in ("false", "False", "FALSE"):
        return False
    if _INT_RE.match(s):
        return int(s)
    if _FLOAT_RE.match(s) and any(c in s for c in ".eE"):
        return float(s)
    return s


def _split_flow(text: str):
    """Split a flow collection body on top-level commas."""
    parts, buf, depth, quote = [], [], 0, None
    for ch in text:
        if quote:
            buf.append(ch)
            if ch == quote:
                quote = None
            continue
        if ch in ("'", '"'):
            quote = ch
            buf.append(ch)
        elif ch in "[{":
            depth += 1
            buf.append(ch)
        elif ch in "]}":
            depth -= 1
            buf.append(ch)
        elif ch == "," and depth == 0:
            parts.append("".join(buf))
            buf = []
        else:
            buf.append(ch)
    if "".join(buf).strip():
        parts.append("".join(buf))
    return parts


def _parse_flow(text: str):
    s = text.strip()
    if s.startswith("[") and s.endswith("]"):
        return [_parse_flow(p) for p in _split_flow(s[1:-1])]
    if s.startswith("{") and s.endswith("}"):
        out = {}
        for part in _split_flow(s[1:-1]):
            if ":" not in part:
                raise ParseError(f"bad flow mapping entry: {part!r}")
            k, v = part.split(":", 1)
            out[_scalar(k)] = _parse_flow(v)
        return out
    return _scalar(s)


def _strip_comment(line: str) -> str:
    """Drop a trailing # comment that is not inside quotes."""
    quote = None
    for i, ch in enumerate(line):
        if quote:
            if ch == quote:
                quote = None
        elif ch in ("'", '"'):
            quote = ch
        elif ch == "#" and (i == 0 or line[i - 1] in " \t"):
            return line[:i]
    return line


class _Line:
    __slots__ = ("indent", "content", "num")

    def __init__(self, indent: int, content: str, num: int):
        self.indent = indent
        self.content = content
        self.num = num


def _lex(source: str) -> list[_Line]:
    raw_lines = source.splitlines()
    lines: list[_Line] = []
    i = 0
    while i < len(raw_lines):
        raw = raw_lines[i]
        num = i + 1
        stripped = _strip_comment(raw).rstrip()
        i += 1
        if not stripped.strip():
            continue
        if stripped.strip() == "---":
            continue
        indent = len(stripped) - len(stripped.lstrip(" "))
        if "\t" in stripped[: indent + 1]:
            raise ParseError(f"line {num}: tabs are not valid indentation")
        lines.append(_Line(indent, stripped.strip(), num))
        # A block scalar header: keep the following deeper lines RAW
        # (no comment stripping — `#` is legal inside code blocks).
        if stripped.rstrip().endswith(("|", "|-", "|+")):
            while i < len(raw_lines):
                nxt = raw_lines[i]
                if nxt.strip() == "":
                    # Blank lines belong to the block only if a deeper
                    # line follows; simplest: keep them, filter later.
                    lines.append(_Line(indent + 2, "", i + 1))
                    i += 1
                    continue
                n_indent = len(nxt) - len(nxt.lstrip(" "))
                if n_indent <= indent:
                    break
                lines.append(_Line(n_indent, nxt.strip(), i + 1))
                i += 1
            while lines and lines[-1].content == "":
                lines.pop()
    return lines


def parse_yaml(source: str):
    """Parse the quilt-sheet YAML subset into dicts / lists / scalars."""
    lines = _lex(source)
    if not lines:
        return {}
    pos = 0

    def parse_block(indent: int):
        nonlocal pos
        if pos >= len(lines):
            return None
        line = lines[pos]
        if line.content.startswith("- ") or line.content == "-":
            return parse_seq(indent)
        return parse_map(indent)

    def parse_seq(indent: int):
        nonlocal pos
        items = []
        while pos < len(lines):
            line = lines[pos]
            if line.indent != indent or not (
                line.content.startswith("- ") or line.content == "-"
            ):
                break
            pos += 1
            rest = line.content[1:].strip()
            if not rest:
                items.append(parse_block(indent + 1) if _peek_deeper(indent) else None)
                continue
            # `- key: value` — an inline first key of a mapping item.
            if ":" in _split_key_span(rest):
                key, val = _split_kv(rest)
                # The item's remaining keys sit at the column where the
                # first key starts (two past the dash marker).
                item_indent = line.indent + 2
                item = {}
                if val == "":
                    if _peek_deeper(item_indent):
                        item[key] = parse_block(lines[pos].indent)
                    else:
                        item[key] = None
                else:
                    item[key] = _parse_value(val, item_indent)
                items.append(_continue_map(item, item_indent))
            else:
                items.append(_parse_flow(rest))
        return items

    def _peek_deeper(indent: int) -> bool:
        return pos < len(lines) and lines[pos].indent > indent

    def _split_key_span(text: str) -> str:
        """Return the text up to the first top-level ':' (or all of it)."""
        quote = None
        for i, ch in enumerate(text):
            if quote:
                if ch == quote:
                    quote = None
            elif ch in ("'", '"'):
                quote = ch
            elif ch == ":":
                return text[: i + 1]
        return text

    def _split_kv(text: str):
        span = _split_key_span(text)
        if span.endswith(":"):
            key = span[:-1].strip()
            return key, text[len(span):].strip()
        return text, None

    def _continue_map(item: dict, indent: int) -> dict:
        nonlocal pos
        while pos < len(lines):
            line = lines[pos]
            if line.indent != indent or line.content.startswith("- "):
                break
            key, val = _split_kv(line.content)
            if val is None and not line.content.endswith(":"):
                break
            pos += 1
            if val == "":
                if _peek_deeper(indent):
                    item[key] = parse_block(lines[pos].indent)
                else:
                    item[key] = None
            else:
                item[key] = _parse_value(val, indent)
        return item

    def parse_map(indent: int):
        nonlocal pos
        item: dict = {}
        while pos < len(lines):
            line = lines[pos]
            if line.indent != indent:
                if line.indent > indent:
                    raise ParseError(
                        f"line {line.num}: unexpected indent {line.indent} (expected {indent})"
                    )
                break
            if line.content.startswith("- ") or line.content == "-":
                break
            key, val = _split_kv(line.content)
            if val is None:
                raise ParseError(f"line {line.num}: expected 'key: value'")
            pos += 1
            if val == "|":
                item[key] = _block_scalar(indent)
            elif val == "":
                if _peek_deeper(indent):
                    item[key] = parse_block(lines[pos].indent)
                else:
                    item[key] = None
            else:
                item[key] = _parse_value(val, indent)
        return item

    def _block_scalar(indent: int) -> str:
        nonlocal pos
        body: list[str] = []
        while pos < len(lines) and lines[pos].indent > indent:
            pad = max(lines[pos].indent - (indent + 2), 0)
            body.append(" " * pad + lines[pos].content)
            pos += 1
        return "\n".join(body) + ("\n" if body else "")

    def _parse_value(val: str, indent: int):
        if val == "|":
            return _block_scalar(indent)
        if val == "":
            if _peek_deeper(indent):
                return parse_block(lines[pos].indent)
            return None
        return _parse_flow(val)

    result = parse_block(lines[0].indent)
    if pos != len(lines):
        raise ParseError(f"line {lines[pos].num}: trailing content could not be parsed")
    return result
