package sheet

import "testing"

func TestParseGoldenSheet(t *testing.T) {
	s, err := Parse(`id: demo
version: "1"
title: Demo
cells:
  - id: temp
    kind: value
    value: 21
    description: "Current temp"
  - id: fahrenheit
    kind: formula
    expr: =temp * 9 / 5 + 32
  - id: light.state
    kind: formula
    expr: =is_dark ? 'ON' : 'OFF'
`)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if s.ID != "demo" || len(s.Cells) != 3 {
		t.Fatalf("got id=%q cells=%d", s.ID, len(s.Cells))
	}
	if s.Cells[0].Value.I != 21 {
		t.Errorf("temp value: got %v", s.Cells[0].Value)
	}
	if s.Cells[1].Expr != "=temp * 9 / 5 + 32" {
		t.Errorf("expr: got %q", s.Cells[1].Expr)
	}
	if s.Cells[2].ID != "light.state" {
		t.Errorf("dotted id: got %q", s.Cells[2].ID)
	}
}

func TestParseScalars(t *testing.T) {
	s, err := Parse(`id: scalars
cells:
  - id: i
    kind: value
    value: 42
  - id: f
    kind: value
    value: 40.5
  - id: b
    kind: value
    value: true
  - id: s
    kind: value
    value: "quoted # not a comment"
  - id: bare
    kind: value
    value: hello
  - id: list
    kind: value
    value: [1, 2, 3]
`)
	if err != nil {
		t.Fatalf("parse: %v", err)
	}
	if s.Cells[0].Value.I != 42 || s.Cells[1].Value.F != 40.5 || !s.Cells[2].Value.B {
		t.Errorf("scalars wrong: %+v", s.Cells)
	}
	if s.Cells[3].Value.S != "quoted # not a comment" {
		t.Errorf("quoted string: got %q", s.Cells[3].Value.S)
	}
	if s.Cells[4].Value.S != "hello" {
		t.Errorf("bare string: got %q", s.Cells[4].Value.S)
	}
	if len(s.Cells[5].Value.A) != 3 || s.Cells[5].Value.A[2].I != 3 {
		t.Errorf("flow list: got %+v", s.Cells[5].Value.A)
	}
}

func TestValidation(t *testing.T) {
	if _, err := Parse("id: x\ncells:\n  - id: a\n    kind: value\n    value: 1\n  - id: a\n    kind: value\n    value: 2\n"); err == nil {
		t.Error("duplicate ids accepted")
	}
	if _, err := Parse("id: x\ncells:\n  - id: a\n    kind: formula\n"); err == nil {
		t.Error("formula without expr accepted")
	}
	if _, err := Parse("id: x\ncells:\n  - id: a\n    kind: api\n    endpoint: http://x\n"); err == nil {
		t.Error("out-of-scope kind accepted")
	}
}
