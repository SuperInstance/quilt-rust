// Package expr is the formula evaluator for quilt-go. The Rust tier uses
// rhai and the TS tier uses `new Function`; Go is stdlib-only, so this is
// a small lexer + recursive-descent parser + tree-walking evaluator for
// the expression subset the sheet format actually uses:
//
//   - literals: numbers (int / float), 'single' and "double" quoted
//     strings, true / false / null
//   - cell references: dotted identifiers (ambient.light)
//   - arithmetic: + - * / %   (+ concatenates when either side is a string)
//   - comparison: < > <= >= == !=
//   - logic: && || !  and the ternary  cond ? a : b
//   - functions: abs min max clamp floor ceil round sqrt pow
//   - parentheses
//
// Numeric semantics (documented divergence, pinned for the golden vectors):
// + - * % on two Ints stay Int; / always yields Float (TS semantics, not
// rhai's truncating integer division); mixed Int/Float promotes to Float.
// Truthiness: bools as-is, numbers nonzero, strings non-empty, null false.
package expr

import (
	"fmt"
	"math"
	"strings"

	"quilt-go/internal/value"
)

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

type tokKind int

const (
	tEOF tokKind = iota
	tNum
	tStr
	tIdent
	tOp
	tLParen
	tRParen
	tComma
	tQuestion
	tColon
)

type token struct {
	kind tokKind
	text string // operator text, ident text
	num  value.Value
	str  string
}

func lex(body string) ([]token, error) {
	var toks []token
	i := 0
	for i < len(body) {
		c := body[i]
		switch {
		case c == ' ' || c == '\t' || c == '\n' || c == '\r':
			i++
		case c >= '0' && c <= '9' || (c == '.' && i+1 < len(body) && body[i+1] >= '0' && body[i+1] <= '9'):
			j := i
			isFloat := false
			for j < len(body) && (body[j] >= '0' && body[j] <= '9' || body[j] == '.' || body[j] == 'e' || body[j] == 'E' ||
				((body[j] == '+' || body[j] == '-') && j > i && (body[j-1] == 'e' || body[j-1] == 'E'))) {
				if body[j] == '.' || body[j] == 'e' || body[j] == 'E' {
					isFloat = true
				}
				j++
			}
			text := body[i:j]
			if isFloat {
				f, err := parseFloat(text)
				if err != nil {
					return nil, err
				}
				toks = append(toks, token{kind: tNum, num: value.FloatV(f)})
			} else {
				var iv int64
				if _, err := fmt.Sscan(text, &iv); err != nil {
					return nil, fmt.Errorf("bad number %q", text)
				}
				toks = append(toks, token{kind: tNum, num: value.IntV(iv)})
			}
			i = j
		case c == '\'' || c == '"':
			str, next, err := lexString(body, i)
			if err != nil {
				return nil, err
			}
			toks = append(toks, token{kind: tStr, str: str})
			i = next
		case isIdentStart(c):
			j := i
			for j < len(body) && isIdentPart(body[j]) {
				j++
			}
			// dotted cell references: ambient.light
			for j < len(body) && body[j] == '.' && j+1 < len(body) && isIdentStart(body[j+1]) {
				j++
				for j < len(body) && isIdentPart(body[j]) {
					j++
				}
			}
			toks = append(toks, token{kind: tIdent, text: body[i:j]})
			i = j
		default:
			two := ""
			if i+1 < len(body) {
				two = body[i : i+2]
			}
			switch two {
			case "<=", ">=", "==", "!=", "&&", "||":
				toks = append(toks, token{kind: tOp, text: two})
				i += 2
				continue
			}
			switch c {
			case '+', '-', '*', '/', '%', '<', '>', '!':
				toks = append(toks, token{kind: tOp, text: string(c)})
			case '(':
				toks = append(toks, token{kind: tLParen})
			case ')':
				toks = append(toks, token{kind: tRParen})
			case ',':
				toks = append(toks, token{kind: tComma})
			case '?':
				toks = append(toks, token{kind: tQuestion})
			case ':':
				toks = append(toks, token{kind: tColon})
			default:
				return nil, fmt.Errorf("unexpected character %q in expression", string(c))
			}
			i++
		}
	}
	toks = append(toks, token{kind: tEOF})
	return toks, nil
}

func isIdentStart(c byte) bool {
	return c == '_' || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
}

func isIdentPart(c byte) bool {
	return isIdentStart(c) || (c >= '0' && c <= '9')
}

func parseFloat(s string) (float64, error) {
	var f float64
	if _, err := fmt.Sscan(s, &f); err != nil {
		return 0, fmt.Errorf("bad number %q", s)
	}
	return f, nil
}

func lexString(body string, start int) (string, int, error) {
	quote := body[start]
	var sb strings.Builder
	i := start + 1
	for i < len(body) {
		c := body[i]
		if c == quote {
			// YAML-style '' escape inside single-quoted strings
			if quote == '\'' && i+1 < len(body) && body[i+1] == '\'' {
				sb.WriteByte('\'')
				i += 2
				continue
			}
			return sb.String(), i + 1, nil
		}
		if c == '\\' && quote == '"' && i+1 < len(body) {
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
			i++
			continue
		}
		sb.WriteByte(c)
		i++
	}
	return "", 0, fmt.Errorf("unterminated string literal")
}

// ---------------------------------------------------------------------------
// Parser (recursive descent)
// ---------------------------------------------------------------------------

type parser struct {
	toks []token
	pos  int
}

func (p *parser) peek() token { return p.toks[p.pos] }
func (p *parser) next() token { t := p.toks[p.pos]; p.pos++; return t }

// node is a parsed expression tree, evaluated against an environment of
// cell values.
type node interface {
	eval(env map[string]value.Value) (value.Value, error)
}

type litNode struct{ v value.Value }

func (n litNode) eval(map[string]value.Value) (value.Value, error) { return n.v, nil }

type identNode struct{ name string }

func (n identNode) eval(env map[string]value.Value) (value.Value, error) {
	v, ok := env[n.name]
	if !ok {
		return value.Value{}, fmt.Errorf("unknown cell reference %q", n.name)
	}
	return v, nil
}

type unaryNode struct {
	op string
	x  node
}

func (n unaryNode) eval(env map[string]value.Value) (value.Value, error) {
	x, err := n.x.eval(env)
	if err != nil {
		return value.Value{}, err
	}
	switch n.op {
	case "-":
		if x.K == value.Int {
			return value.IntV(-x.I), nil
		}
		if x.K == value.Float {
			return value.FloatV(-x.F), nil
		}
		return value.Value{}, fmt.Errorf("unary - on non-number")
	case "!":
		return value.BoolV(!truthy(x)), nil
	}
	return value.Value{}, fmt.Errorf("bad unary op %q", n.op)
}

type binaryNode struct {
	op   string
	l, r node
}

type ternaryNode struct{ c, t, f node }

func (n ternaryNode) eval(env map[string]value.Value) (value.Value, error) {
	c, err := n.c.eval(env)
	if err != nil {
		return value.Value{}, err
	}
	if truthy(c) {
		return n.t.eval(env)
	}
	return n.f.eval(env)
}

type callNode struct {
	fn   string
	args []node
}

func truthy(v value.Value) bool {
	switch v.K {
	case value.Null:
		return false
	case value.Bool:
		return v.B
	case value.Int:
		return v.I != 0
	case value.Float:
		return v.F != 0
	case value.String:
		return v.S != ""
	default:
		return true
	}
}

func numeric(op string, l, r value.Value) (value.Value, error) {
	if !l.IsNumber() || !r.IsNumber() {
		return value.Value{}, fmt.Errorf("operator %s requires numbers", op)
	}
	// Int-preserving ops, per package doc.
	if l.K == value.Int && r.K == value.Int {
		switch op {
		case "+":
			return value.IntV(l.I + r.I), nil
		case "-":
			return value.IntV(l.I - r.I), nil
		case "*":
			return value.IntV(l.I * r.I), nil
		case "%":
			if r.I == 0 {
				return value.Value{}, fmt.Errorf("modulo by zero")
			}
			return value.IntV(l.I % r.I), nil
		}
	}
	lf, rf := l.AsFloat(), r.AsFloat()
	switch op {
	case "+":
		return value.FloatV(lf + rf), nil
	case "-":
		return value.FloatV(lf - rf), nil
	case "*":
		return value.FloatV(lf * rf), nil
	case "/":
		return value.FloatV(lf / rf), nil // / is always float (TS semantics)
	case "%":
		if rf == 0 {
			return value.Value{}, fmt.Errorf("modulo by zero")
		}
		return value.FloatV(math.Mod(lf, rf)), nil
	}
	return value.Value{}, fmt.Errorf("bad numeric op %q", op)
}

func compare(op string, l, r value.Value) (value.Value, error) {
	if l.IsNumber() && r.IsNumber() {
		lf, rf := l.AsFloat(), r.AsFloat()
		switch op {
		case "<":
			return value.BoolV(lf < rf), nil
		case ">":
			return value.BoolV(lf > rf), nil
		case "<=":
			return value.BoolV(lf <= rf), nil
		case ">=":
			return value.BoolV(lf >= rf), nil
		}
	}
	if l.K == value.String && r.K == value.String {
		switch op {
		case "<":
			return value.BoolV(l.S < r.S), nil
		case ">":
			return value.BoolV(l.S > r.S), nil
		case "<=":
			return value.BoolV(l.S <= r.S), nil
		case ">=":
			return value.BoolV(l.S >= r.S), nil
		}
	}
	return value.Value{}, fmt.Errorf("operator %s on incompatible values", op)
}

func (n binaryNode) eval(env map[string]value.Value) (value.Value, error) {
	// Short-circuit logic.
	if n.op == "&&" || n.op == "||" {
		l, err := n.l.eval(env)
		if err != nil {
			return value.Value{}, err
		}
		if n.op == "&&" && !truthy(l) {
			return value.BoolV(false), nil
		}
		if n.op == "||" && truthy(l) {
			return value.BoolV(true), nil
		}
		r, err := n.r.eval(env)
		if err != nil {
			return value.Value{}, err
		}
		return value.BoolV(truthy(r)), nil
	}
	l, err := n.l.eval(env)
	if err != nil {
		return value.Value{}, err
	}
	r, err := n.r.eval(env)
	if err != nil {
		return value.Value{}, err
	}
	switch n.op {
	case "==":
		if l.IsNumber() && r.IsNumber() {
			return value.BoolV(l.AsFloat() == r.AsFloat()), nil
		}
		return value.BoolV(value.Equal(l, r)), nil
	case "!=":
		if l.IsNumber() && r.IsNumber() {
			return value.BoolV(l.AsFloat() != r.AsFloat()), nil
		}
		return value.BoolV(!value.Equal(l, r)), nil
	case "<", ">", "<=", ">=":
		return compare(n.op, l, r)
	case "+":
		if l.K == value.String || r.K == value.String {
			return value.StrV(value.Display(l) + value.Display(r)), nil
		}
		return numeric(n.op, l, r)
	default:
		return numeric(n.op, l, r)
	}
}

func (n callNode) eval(env map[string]value.Value) (value.Value, error) {
	args := make([]value.Value, len(n.args))
	for i, a := range n.args {
		v, err := a.eval(env)
		if err != nil {
			return value.Value{}, err
		}
		args[i] = v
	}
	floats := func() ([]float64, error) {
		out := make([]float64, len(args))
		for i, a := range args {
			if !a.IsNumber() {
				return nil, fmt.Errorf("%s() requires numeric arguments", n.fn)
			}
			out[i] = a.AsFloat()
		}
		return out, nil
	}
	switch n.fn {
	case "abs":
		f, err := floats()
		if err != nil || len(f) != 1 {
			return value.Value{}, fmt.Errorf("abs(x) takes 1 number")
		}
		return value.FloatV(math.Abs(f[0])), nil
	case "min", "max":
		f, err := floats()
		if err != nil || len(f) < 1 {
			return value.Value{}, fmt.Errorf("%s() takes >= 1 numbers", n.fn)
		}
		best := f[0]
		for _, x := range f[1:] {
			if n.fn == "min" && x < best || n.fn == "max" && x > best {
				best = x
			}
		}
		return value.FloatV(best), nil
	case "clamp":
		f, err := floats()
		if err != nil || len(f) != 3 {
			return value.Value{}, fmt.Errorf("clamp(x, lo, hi) takes 3 numbers")
		}
		return value.FloatV(math.Min(math.Max(f[0], f[1]), f[2])), nil
	case "floor", "ceil", "round", "sqrt":
		f, err := floats()
		if err != nil || len(f) != 1 {
			return value.Value{}, fmt.Errorf("%s(x) takes 1 number", n.fn)
		}
		switch n.fn {
		case "floor":
			return value.FloatV(math.Floor(f[0])), nil
		case "ceil":
			return value.FloatV(math.Ceil(f[0])), nil
		case "round":
			return value.FloatV(math.Round(f[0])), nil
		default:
			return value.FloatV(math.Sqrt(f[0])), nil
		}
	case "pow":
		f, err := floats()
		if err != nil || len(f) != 2 {
			return value.Value{}, fmt.Errorf("pow(x, y) takes 2 numbers")
		}
		return value.FloatV(math.Pow(f[0], f[1])), nil
	}
	return value.Value{}, fmt.Errorf("unknown function %q", n.fn)
}

// Grammar, loosest to tightest:
//
//	ternary := orExpr ('?' ternary ':' ternary)?
//	orExpr  := andExpr ('||' andExpr)*
//	andExpr := eqExpr  ('&&' eqExpr)*
//	eqExpr  := relExpr (('=='|'!=') relExpr)*
//	relExpr := addExpr (('<'|'>'|'<='|'>=') addExpr)*
//	addExpr := mulExpr (('+'|'-') mulExpr)*
//	mulExpr := unary   (('*'|'/'|'%') unary)*
//	unary   := ('!'|'-') unary | primary
//	primary := number | string | ident | ident '(' args ')' | '(' ternary ')'

func (p *parser) parseTernary() (node, error) {
	c, err := p.parseBinary(0)
	if err != nil {
		return nil, err
	}
	if p.peek().kind == tQuestion {
		p.next()
		t, err := p.parseTernary()
		if err != nil {
			return nil, err
		}
		if p.next().kind != tColon {
			return nil, fmt.Errorf("expected ':' in ternary")
		}
		f, err := p.parseTernary()
		if err != nil {
			return nil, err
		}
		return ternaryNode{c, t, f}, nil
	}
	return c, nil
}

var binLevels = [][]string{
	{"||"},
	{"&&"},
	{"==", "!="},
	{"<", ">", "<=", ">="},
	{"+", "-"},
	{"*", "/", "%"},
}

func (p *parser) parseBinary(level int) (node, error) {
	if level >= len(binLevels) {
		return p.parseUnary()
	}
	left, err := p.parseBinary(level + 1)
	if err != nil {
		return nil, err
	}
	for p.peek().kind == tOp && contains(binLevels[level], p.peek().text) {
		op := p.next().text
		right, err := p.parseBinary(level + 1)
		if err != nil {
			return nil, err
		}
		left = binaryNode{op, left, right}
	}
	return left, nil
}

func contains(xs []string, s string) bool {
	for _, x := range xs {
		if x == s {
			return true
		}
	}
	return false
}

func (p *parser) parseUnary() (node, error) {
	if t := p.peek(); t.kind == tOp && (t.text == "!" || t.text == "-") {
		p.next()
		x, err := p.parseUnary()
		if err != nil {
			return nil, err
		}
		return unaryNode{t.text, x}, nil
	}
	return p.parsePrimary()
}

func (p *parser) parsePrimary() (node, error) {
	t := p.next()
	switch t.kind {
	case tNum:
		return litNode{t.num}, nil
	case tStr:
		return litNode{value.StrV(t.str)}, nil
	case tIdent:
		switch t.text {
		case "true":
			return litNode{value.BoolV(true)}, nil
		case "false":
			return litNode{value.BoolV(false)}, nil
		case "null":
			return litNode{value.NullV()}, nil
		}
		if p.peek().kind == tLParen {
			p.next()
			var args []node
			if p.peek().kind != tRParen {
				for {
					a, err := p.parseTernary()
					if err != nil {
						return nil, err
					}
					args = append(args, a)
					if p.peek().kind == tComma {
						p.next()
						continue
					}
					break
				}
			}
			if p.next().kind != tRParen {
				return nil, fmt.Errorf("expected ')' after arguments to %s", t.text)
			}
			return callNode{t.text, args}, nil
		}
		return identNode{t.text}, nil
	case tLParen:
		n, err := p.parseTernary()
		if err != nil {
			return nil, err
		}
		if p.next().kind != tRParen {
			return nil, fmt.Errorf("expected ')'")
		}
		return n, nil
	}
	return nil, fmt.Errorf("unexpected token in expression")
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

// Body strips the leading '=' that marks a formula expression in a sheet.
func Body(expr string) string {
	return strings.TrimSpace(strings.TrimPrefix(strings.TrimSpace(expr), "="))
}

// Eval parses and evaluates a formula expression against env (cell id →
// current value).
func Eval(expr string, env map[string]value.Value) (value.Value, error) {
	toks, err := lex(Body(expr))
	if err != nil {
		return value.Value{}, err
	}
	p := &parser{toks: toks}
	n, err := p.parseTernary()
	if err != nil {
		return value.Value{}, err
	}
	if p.peek().kind != tEOF {
		return value.Value{}, fmt.Errorf("trailing tokens in expression %q", expr)
	}
	return n.eval(env)
}

// Identifiers returns every identifier token in the expression that is not
// a function name or keyword — the candidate cell references used for
// dependency auto-detection (the engine intersects these with known ids).
func Identifiers(expr string) []string {
	toks, err := lex(Body(expr))
	if err != nil {
		return nil
	}
	var out []string
	for i, t := range toks {
		if t.kind != tIdent {
			continue
		}
		if t.text == "true" || t.text == "false" || t.text == "null" {
			continue
		}
		if i+1 < len(toks) && toks[i+1].kind == tLParen {
			continue // function name
		}
		out = append(out, t.text)
	}
	return out
}
