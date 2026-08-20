// Package sheet parses the quilt sheet format's YAML subset — stdlib only,
// so this is a minimal indentation-based parser covering exactly what
// value/formula sheets use:
//
//   - top-level keys: id, version, title, description, cells
//   - cells: a block sequence of maps ("- id: x" followed by deeper keys)
//   - scalars: ints, floats, booleans, null, bare strings, 'single' and
//     "double" quoted strings
//   - inline flow lists: [a, b, c]
//   - comments (#) and blank lines
//
// Only the cell kinds in scope for this tier are accepted: value and
// formula. Anything else is a parse-time error (documented scope, not a
// silent skip).
package sheet

import (
	"fmt"
	"strconv"
	"strings"

	"quilt-go/internal/value"
)

// CellDef is one cell entry in a sheet.
type CellDef struct {
	ID          string
	Kind        string // "value" | "formula"
	Value       value.Value
	HasValue    bool
	Expr        string
	Description string
	Deps        []string
}

// SheetDef is a parsed sheet.
type SheetDef struct {
	ID          string
	Version     string
	Title       string
	Description string
	Cells       []CellDef
}

// ---------------------------------------------------------------------------
// Generic YAML-subset tree
// ---------------------------------------------------------------------------

type nodeKind int

const (
	nScalar nodeKind = iota
	nMap
	nList
)

type node struct {
	kind   nodeKind
	scalar string // raw scalar text (unquoted later)
	quoted bool
	keys   []string
	m      map[string]*node
	items  []*node
}

type line struct {
	indent int
	text   string
}

// stripComment removes a trailing " #..." comment, honoring quotes.
func stripComment(s string) string {
	var q byte
	for i := 0; i < len(s); i++ {
		c := s[i]
		if q != 0 {
			if c == q {
				q = 0
			}
			continue
		}
		switch c {
		case '\'', '"':
			q = c
		case '#':
			if i == 0 || s[i-1] == ' ' || s[i-1] == '\t' {
				return s[:i]
			}
		}
	}
	return s
}

func splitLines(src string) []line {
	var out []line
	for _, raw := range strings.Split(src, "\n") {
		raw = strings.TrimRight(stripComment(raw), " \t\r")
		if strings.TrimSpace(raw) == "" {
			continue
		}
		indent := 0
		for indent < len(raw) && raw[indent] == ' ' {
			indent++
		}
		out = append(out, line{indent, raw[indent:]})
	}
	return out
}

// parseBlock parses lines[start:] at the given indent into a node,
// returning the next unconsumed line index.
func parseBlock(lines []line, start, indent int) (*node, int, error) {
	if start >= len(lines) {
		return &node{kind: nScalar, scalar: ""}, start, nil
	}
	first := lines[start]
	if first.indent < indent {
		return &node{kind: nScalar, scalar: ""}, start, nil
	}
	if strings.HasPrefix(first.text, "- ") || first.text == "-" {
		return parseList(lines, start, first.indent)
	}
	return parseMap(lines, start, first.indent)
}

func splitKV(text string) (key, val string, ok bool) {
	// Find the first ": " or trailing ":" outside quotes.
	var q byte
	for i := 0; i < len(text); i++ {
		c := text[i]
		if q != 0 {
			if c == q {
				q = 0
			}
			continue
		}
		switch c {
		case '\'', '"':
			q = c
		case ':':
			if i+1 >= len(text) {
				return strings.TrimSpace(text[:i]), "", true
			}
			if text[i+1] == ' ' {
				return strings.TrimSpace(text[:i]), strings.TrimSpace(text[i+2:]), true
			}
		}
	}
	return "", "", false
}

func parseMap(lines []line, start, indent int) (*node, int, error) {
	n := &node{kind: nMap, m: map[string]*node{}}
	i := start
	for i < len(lines) {
		ln := lines[i]
		if ln.indent < indent || strings.HasPrefix(ln.text, "- ") || ln.text == "-" {
			break
		}
		if ln.indent > indent {
			return nil, 0, fmt.Errorf("unexpected indentation at %q", ln.text)
		}
		key, val, ok := splitKV(ln.text)
		if !ok {
			return nil, 0, fmt.Errorf("expected key: value, got %q", ln.text)
		}
		if val != "" {
			n.keys = append(n.keys, key)
			n.m[key] = scalarNode(val)
			i++
			continue
		}
		// Nested block on following deeper lines, or null.
		if i+1 < len(lines) && lines[i+1].indent > indent {
			child, next, err := parseBlock(lines, i+1, lines[i+1].indent)
			if err != nil {
				return nil, 0, err
			}
			n.keys = append(n.keys, key)
			n.m[key] = child
			i = next
		} else {
			n.keys = append(n.keys, key)
			n.m[key] = scalarNode("null")
			i++
		}
	}
	return n, i, nil
}

func parseList(lines []line, start, indent int) (*node, int, error) {
	n := &node{kind: nList}
	i := start
	for i < len(lines) {
		ln := lines[i]
		if ln.indent != indent || !(strings.HasPrefix(ln.text, "- ") || ln.text == "-") {
			break
		}
		rest := strings.TrimSpace(strings.TrimPrefix(ln.text, "-"))
		if rest == "" {
			// Nested block item.
			child, next, err := parseBlock(lines, i+1, indent+2)
			if err != nil {
				return nil, 0, err
			}
			n.items = append(n.items, child)
			i = next
			continue
		}
		if key, val, ok := splitKV(rest); ok {
			// Inline-start map item: "- id: greeting" then continuation
			// keys at indent+2.
			item := &node{kind: nMap, m: map[string]*node{}}
			if val != "" {
				item.keys = append(item.keys, key)
				item.m[key] = scalarNode(val)
				i++
			} else if i+1 < len(lines) && lines[i+1].indent > indent+2 {
				child, next, err := parseBlock(lines, i+1, lines[i+1].indent)
				if err != nil {
					return nil, 0, err
				}
				item.keys = append(item.keys, key)
				item.m[key] = child
				i = next
			} else {
				item.keys = append(item.keys, key)
				item.m[key] = scalarNode("null")
				i++
			}
			for i < len(lines) && lines[i].indent == indent+2 && !strings.HasPrefix(lines[i].text, "- ") {
				k2, v2, ok := splitKV(lines[i].text)
				if !ok {
					return nil, 0, fmt.Errorf("expected key: value, got %q", lines[i].text)
				}
				if v2 != "" {
					item.keys = append(item.keys, k2)
					item.m[k2] = scalarNode(v2)
					i++
					continue
				}
				if i+1 < len(lines) && lines[i+1].indent > indent+2 {
					child, next, err := parseBlock(lines, i+1, lines[i+1].indent)
					if err != nil {
						return nil, 0, err
					}
					item.keys = append(item.keys, k2)
					item.m[k2] = child
					i = next
				} else {
					item.keys = append(item.keys, k2)
					item.m[k2] = scalarNode("null")
					i++
				}
			}
			n.items = append(n.items, item)
			continue
		}
		// Plain scalar item.
		n.items = append(n.items, scalarNode(rest))
		i++
	}
	return n, i, nil
}

func scalarNode(s string) *node {
	trimmed := strings.TrimSpace(s)
	if len(trimmed) >= 2 && (trimmed[0] == '"' || trimmed[0] == '\'') {
		return &node{kind: nScalar, scalar: trimmed, quoted: true}
	}
	// Inline flow list.
	if strings.HasPrefix(trimmed, "[") && strings.HasSuffix(trimmed, "]") {
		inner := strings.TrimSpace(trimmed[1 : len(trimmed)-1])
		lst := &node{kind: nList}
		if inner != "" {
			for _, part := range splitFlow(inner) {
				lst.items = append(lst.items, scalarNode(part))
			}
		}
		return lst
	}
	return &node{kind: nScalar, scalar: trimmed}
}

// splitFlow splits a flow list body on top-level commas, honoring quotes.
func splitFlow(s string) []string {
	var out []string
	var q byte
	depth := 0
	start := 0
	for i := 0; i < len(s); i++ {
		c := s[i]
		if q != 0 {
			if c == q {
				q = 0
			}
			continue
		}
		switch c {
		case '\'', '"':
			q = c
		case '[':
			depth++
		case ']':
			depth--
		case ',':
			if depth == 0 {
				out = append(out, strings.TrimSpace(s[start:i]))
				start = i + 1
			}
		}
	}
	out = append(out, strings.TrimSpace(s[start:]))
	return out
}

// scalarValue converts a scalar node to a value.Value.
func scalarValue(n *node) (value.Value, error) {
	s := n.scalar
	if n.quoted {
		if len(s) < 2 {
			return value.Value{}, fmt.Errorf("bad quoted scalar %q", s)
		}
		body := s[1 : len(s)-1]
		if s[0] == '\'' {
			return value.StrV(strings.ReplaceAll(body, "''", "'")), nil
		}
		var sb strings.Builder
		for i := 0; i < len(body); i++ {
			if body[i] == '\\' && i+1 < len(body) {
				i++
				switch body[i] {
				case 'n':
					sb.WriteByte('\n')
				case 't':
					sb.WriteByte('\t')
				case 'r':
					sb.WriteByte('\r')
				case '"':
					sb.WriteByte('"')
				case '\\':
					sb.WriteByte('\\')
				default:
					sb.WriteByte(body[i])
				}
				continue
			}
			sb.WriteByte(body[i])
		}
		return value.StrV(sb.String()), nil
	}
	switch s {
	case "true", "True":
		return value.BoolV(true), nil
	case "false", "False":
		return value.BoolV(false), nil
	case "null", "~", "":
		return value.NullV(), nil
	}
	if i, err := strconv.ParseInt(s, 10, 64); err == nil {
		return value.IntV(i), nil
	}
	if strings.ContainsAny(s, ".eE") {
		if f, err := strconv.ParseFloat(s, 64); err == nil {
			return value.FloatV(f), nil
		}
	}
	return value.StrV(s), nil
}

// ---------------------------------------------------------------------------
// Tree -> SheetDef
// ---------------------------------------------------------------------------

// Parse parses a sheet from a YAML-subset string and validates it.
func Parse(src string) (*SheetDef, error) {
	lines := splitLines(src)
	if len(lines) == 0 {
		return nil, fmt.Errorf("empty sheet")
	}
	root, next, err := parseBlock(lines, 0, lines[0].indent)
	if err != nil {
		return nil, err
	}
	if next != len(lines) {
		return nil, fmt.Errorf("trailing content at %q", lines[next].text)
	}
	if root.kind != nMap {
		return nil, fmt.Errorf("sheet must be a mapping")
	}

	sheet := &SheetDef{}
	getStr := func(key string) string {
		if n, ok := root.m[key]; ok && n.kind == nScalar {
			v, err := scalarValue(n)
			if err == nil && v.K == value.String {
				return v.S
			}
			if err == nil {
				return value.Display(v)
			}
		}
		return ""
	}
	sheet.ID = getStr("id")
	sheet.Version = getStr("version")
	sheet.Title = getStr("title")
	sheet.Description = getStr("description")

	cellsNode, ok := root.m["cells"]
	if !ok || cellsNode.kind != nList {
		return nil, fmt.Errorf("sheet requires a `cells` list")
	}
	for _, item := range cellsNode.items {
		if item.kind != nMap {
			return nil, fmt.Errorf("each cell must be a mapping")
		}
		def := CellDef{}
		for _, key := range item.keys {
			cn := item.m[key]
			switch key {
			case "id":
				v, err := scalarValue(cn)
				if err != nil {
					return nil, err
				}
				def.ID = value.Display(v)
			case "kind":
				v, err := scalarValue(cn)
				if err != nil {
					return nil, err
				}
				def.Kind = value.Display(v)
			case "value":
				if cn.kind == nList {
					var items []value.Value
					for _, sub := range cn.items {
						v, err := scalarValue(sub)
						if err != nil {
							return nil, err
						}
						items = append(items, v)
					}
					def.Value = value.ArrV(items)
				} else {
					v, err := scalarValue(cn)
					if err != nil {
						return nil, err
					}
					def.Value = v
				}
				def.HasValue = true
			case "expr":
				v, err := scalarValue(cn)
				if err != nil {
					return nil, err
				}
				def.Expr = value.Display(v)
			case "description":
				v, err := scalarValue(cn)
				if err != nil {
					return nil, err
				}
				def.Description = value.Display(v)
			case "deps":
				if cn.kind == nList {
					for _, sub := range cn.items {
						v, err := scalarValue(sub)
						if err != nil {
							return nil, err
						}
						def.Deps = append(def.Deps, value.Display(v))
					}
				}
			default:
				// Unknown keys are ignored: sheets from richer tiers carry
				// fields this tier does not need (unit, source, ...).
			}
		}
		sheet.Cells = append(sheet.Cells, def)
	}

	if err := validate(sheet); err != nil {
		return nil, err
	}
	return sheet, nil
}

func validate(sheet *SheetDef) error {
	if sheet.ID == "" {
		return fmt.Errorf("sheet requires an `id`")
	}
	seen := map[string]bool{}
	for _, c := range sheet.Cells {
		if strings.TrimSpace(c.ID) == "" {
			return fmt.Errorf("cell id cannot be empty")
		}
		if seen[c.ID] {
			return fmt.Errorf("duplicate cell id %q", c.ID)
		}
		seen[c.ID] = true
		switch c.Kind {
		case "value":
			if !c.HasValue {
				return fmt.Errorf("value cell %q requires `value` field", c.ID)
			}
		case "formula":
			if c.Expr == "" {
				return fmt.Errorf("formula cell %q requires `expr` field", c.ID)
			}
		default:
			return fmt.Errorf("cell %q: kind %q is out of scope for quilt-go (value/formula only)", c.ID, c.Kind)
		}
	}
	return nil
}
