// Package value defines quilt-go's cell value model: a JSON-shaped value
// that preserves the integer-vs-float distinction, deep equality, the
// canonical JSON form used as the hash preimage for the edge ledger, and
// the total distance metric from docs/cell-ledger.md (value_distance).
//
// The canonical form is pinned so that every quilt tier (Rust, TS, Go)
// produces byte-identical preimages from the same data:
//
//   - compact JSON, no whitespace;
//   - object keys sorted by UTF-8 byte order;
//   - integers rendered as integers; floats in shortest-round-trip form
//     with a mandatory decimal point or exponent (ryū semantics: 40.0
//     renders "40.0", never "40");
//   - strings with standard JSON escaping (no HTML escaping, no "/" escape).
package value

import (
	"math"
	"sort"
	"strconv"
	"strings"
)

// Kind discriminates the Value union. Int and Float are distinct kinds:
// like serde_json's Number, 1 and 1.0 are numerically equal but not the
// same value (Equal says false; Distance says 0).
type Kind int

const (
	Null Kind = iota
	Bool
	Int
	Float
	String
	Array
	Object
)

// Value is a JSON-shaped cell value.
type Value struct {
	K Kind
	B bool
	I int64
	F float64
	S string
	A []Value
	O map[string]Value
}

func NullV() Value                  { return Value{K: Null} }
func BoolV(b bool) Value            { return Value{K: Bool, B: b} }
func IntV(i int64) Value            { return Value{K: Int, I: i} }
func FloatV(f float64) Value        { return Value{K: Float, F: f} }
func StrV(s string) Value           { return Value{K: String, S: s} }
func ArrV(items []Value) Value      { return Value{K: Array, A: items} }
func ObjV(m map[string]Value) Value { return Value{K: Object, O: m} }

// IsNumber reports whether v is Int or Float.
func (v Value) IsNumber() bool { return v.K == Int || v.K == Float }

// AsFloat views a numeric value as f64. Non-numbers yield 0.
func (v Value) AsFloat() float64 {
	if v.K == Int {
		return float64(v.I)
	}
	if v.K == Float {
		return v.F
	}
	return 0
}

// Equal is type-strict deep equality: Int(1) != Float(1.0), mirroring
// serde_json's derived Number equality (different variants), which the
// Rust ledger relies on for the `changed` flag.
func Equal(a, b Value) bool {
	if a.K != b.K {
		return false
	}
	switch a.K {
	case Null:
		return true
	case Bool:
		return a.B == b.B
	case Int:
		return a.I == b.I
	case Float:
		return a.F == b.F
	case String:
		return a.S == b.S
	case Array:
		if len(a.A) != len(b.A) {
			return false
		}
		for i := range a.A {
			if !Equal(a.A[i], b.A[i]) {
				return false
			}
		}
		return true
	case Object:
		if len(a.O) != len(b.O) {
			return false
		}
		for k, av := range a.O {
			bv, ok := b.O[k]
			if !ok || !Equal(av, bv) {
				return false
			}
		}
		return true
	}
	return false
}

// Distance is the total metric of docs/cell-ledger.md §3:
//
//   - numbers: |a - b| (as f64, so Int(1) vs Float(1.0) score 0)
//   - arrays: mean of element-wise distances over max length; a missing
//     element costs 1.0
//   - objects: mean over the key union; a missing key costs 1.0
//   - equal values of any other type: 0
//   - anything else (type shift, unequal strings/bools): 1.0
func Distance(a, b Value) float64 {
	if a.IsNumber() && b.IsNumber() {
		return math.Abs(a.AsFloat() - b.AsFloat())
	}
	if a.K == Array && b.K == Array {
		n := len(a.A)
		if len(b.A) > n {
			n = len(b.A)
		}
		if n == 0 {
			return 0
		}
		sum := 0.0
		for i := 0; i < n; i++ {
			if i < len(a.A) && i < len(b.A) {
				sum += Distance(a.A[i], b.A[i])
			} else {
				sum += 1.0
			}
		}
		return sum / float64(n)
	}
	if a.K == Object && b.K == Object {
		keys := map[string]bool{}
		for k := range a.O {
			keys[k] = true
		}
		for k := range b.O {
			keys[k] = true
		}
		if len(keys) == 0 {
			return 0
		}
		sum := 0.0
		for k := range keys {
			av, aok := a.O[k]
			bv, bok := b.O[k]
			if aok && bok {
				sum += Distance(av, bv)
			} else {
				sum += 1.0
			}
		}
		return sum / float64(len(keys))
	}
	if Equal(a, b) {
		return 0
	}
	return 1.0
}

// Canonical renders v in the pinned canonical JSON form (see package doc).
func Canonical(v Value) string {
	var sb strings.Builder
	writeCanonical(v, &sb)
	return sb.String()
}

func writeCanonical(v Value, sb *strings.Builder) {
	switch v.K {
	case Null:
		sb.WriteString("null")
	case Bool:
		if v.B {
			sb.WriteString("true")
		} else {
			sb.WriteString("false")
		}
	case Int:
		sb.WriteString(strconv.FormatInt(v.I, 10))
	case Float:
		sb.WriteString(CanonicalFloat(v.F))
	case String:
		WriteJSONString(sb, v.S)
	case Array:
		sb.WriteByte('[')
		for i, item := range v.A {
			if i > 0 {
				sb.WriteByte(',')
			}
			writeCanonical(item, sb)
		}
		sb.WriteByte(']')
	case Object:
		keys := make([]string, 0, len(v.O))
		for k := range v.O {
			keys = append(keys, k)
		}
		sort.Strings(keys) // UTF-8 byte order
		sb.WriteByte('{')
		for i, k := range keys {
			if i > 0 {
				sb.WriteByte(',')
			}
			WriteJSONString(sb, k)
			sb.WriteByte(':')
			writeCanonical(v.O[k], sb)
		}
		sb.WriteByte('}')
	}
}

// CanonicalFloat renders an f64 in shortest-round-trip form with ryū-style
// spelling: a decimal point or exponent is always present ("40.0", "2.5",
// "1e20"), and exponents carry no "+" and no leading zeros ("1e-7").
//
// Known hazard (mirrors docs/cell-ledger.md's JS warning): Go's 'g' format
// switches to exponent notation at |exp10| >= 21 while ryū uses its own
// thresholds, so extreme magnitudes may spell differently from the Rust
// tier. Golden vectors keep values in the plain-decimal range.
func CanonicalFloat(f float64) string {
	if math.IsNaN(f) || math.IsInf(f, 0) {
		return "null" // serde_json cannot represent these; pinned as null
	}
	s := strconv.FormatFloat(f, 'g', -1, 64)
	if i := strings.IndexAny(s, "eE"); i >= 0 {
		mant, exp := s[:i], s[i+1:]
		neg := strings.HasPrefix(exp, "-")
		exp = strings.TrimLeft(strings.TrimPrefix(strings.TrimPrefix(exp, "+"), "-"), "0")
		if exp == "" {
			exp = "0"
		}
		if neg {
			exp = "-" + exp
		}
		return mant + "e" + exp
	}
	if !strings.Contains(s, ".") {
		s += ".0"
	}
	return s
}

// Display renders a value for string interpolation / concatenation in// formulas (JS-like): floats that are integral render without a point.
func Display(v Value) string {
	switch v.K {
	case Null:
		return "null"
	case Bool:
		if v.B {
			return "true"
		}
		return "false"
	case Int:
		return strconv.FormatInt(v.I, 10)
	case Float:
		return strconv.FormatFloat(v.F, 'g', -1, 64)
	case String:
		return v.S
	default:
		return Canonical(v)
	}
}

// WriteJSONString writes s as a JSON string using serde_json's escaping:
// '"', '\\', and the standard short escapes for control characters;
// other control characters as \u00XX; everything else raw UTF-8.
// Notably: no HTML escaping and no '/' escaping (encoding/json defaults
// would diverge).
func WriteJSONString(sb *strings.Builder, s string) {
	sb.WriteByte('"')
	for _, r := range s {
		switch r {
		case '"':
			sb.WriteString(`\"`)
		case '\\':
			sb.WriteString(`\\`)
		case '\b':
			sb.WriteString(`\b`)
		case '\f':
			sb.WriteString(`\f`)
		case '\n':
			sb.WriteString(`\n`)
		case '\r':
			sb.WriteString(`\r`)
		case '\t':
			sb.WriteString(`\t`)
		default:
			if r < 0x20 {
				sb.WriteString(`\u00`)
				sb.WriteByte("0123456789abcdef"[r>>4])
				sb.WriteByte("0123456789abcdef"[r&0xf])
			} else {
				sb.WriteRune(r)
			}
		}
	}
	sb.WriteByte('"')
}

// Pretty renders v as human-readable JSON (2-space indent, sorted keys),
// using the same number and string rendering as Canonical — so a float
// keeps its ".0" and the int/float distinction survives in golden.json.
func Pretty(v Value) string {
	var sb strings.Builder
	writePretty(v, &sb, 0)
	sb.WriteByte('\n')
	return sb.String()
}

func writePretty(v Value, sb *strings.Builder, depth int) {
	indent := strings.Repeat("  ", depth)
	inner := strings.Repeat("  ", depth+1)
	switch v.K {
	case Array:
		if len(v.A) == 0 {
			sb.WriteString("[]")
			return
		}
		sb.WriteString("[\n")
		for i, item := range v.A {
			sb.WriteString(inner)
			writePretty(item, sb, depth+1)
			if i < len(v.A)-1 {
				sb.WriteByte(',')
			}
			sb.WriteByte('\n')
		}
		sb.WriteString(indent)
		sb.WriteByte(']')
	case Object:
		if len(v.O) == 0 {
			sb.WriteString("{}")
			return
		}
		keys := make([]string, 0, len(v.O))
		for k := range v.O {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		sb.WriteString("{\n")
		for i, k := range keys {
			sb.WriteString(inner)
			WriteJSONString(sb, k)
			sb.WriteString(": ")
			writePretty(v.O[k], sb, depth+1)
			if i < len(keys)-1 {
				sb.WriteByte(',')
			}
			sb.WriteByte('\n')
		}
		sb.WriteString(indent)
		sb.WriteByte('}')
	default:
		sb.WriteString(Canonical(v))
	}
}
