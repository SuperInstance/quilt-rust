package value

import "testing"

func TestCanonicalIntFloatDistinction(t *testing.T) {
	if got := Canonical(IntV(40)); got != "40" {
		t.Errorf("int 40: got %q", got)
	}
	if got := Canonical(FloatV(40.0)); got != "40.0" {
		t.Errorf("float 40.0: got %q, want 40.0", got)
	}
	if got := Canonical(FloatV(2.5)); got != "2.5" {
		t.Errorf("float 2.5: got %q", got)
	}
	if got := Canonical(FloatV(69.8)); got != "69.8" {
		t.Errorf("float 69.8: got %q", got)
	}
}

func TestCanonicalKeyOrderAndEscapes(t *testing.T) {
	v := ObjV(map[string]Value{"b": IntV(1), "a": StrV("x\"y")})
	if got := Canonical(v); got != `{"a":"x\"y","b":1}` {
		t.Errorf("got %q", got)
	}
}

func TestDistance(t *testing.T) {
	if d := Distance(IntV(40), IntV(85)); d != 45.0 {
		t.Errorf("numeric: got %v", d)
	}
	if d := Distance(IntV(1), FloatV(1.0)); d != 0.0 {
		t.Errorf("int vs equal float: got %v, want 0", d)
	}
	if Equal(IntV(1), FloatV(1.0)) {
		t.Errorf("Equal(Int 1, Float 1.0) should be false (serde_json variant rule)")
	}
	arr := ArrV([]Value{IntV(1), IntV(2), IntV(3)})
	if d := Distance(arr, ArrV([]Value{IntV(1), IntV(2)})); d < 0.3333 || d > 0.3334 {
		t.Errorf("array missing element: got %v, want 1/3", d)
	}
	if d := Distance(StrV("a"), StrV("a")); d != 0.0 {
		t.Errorf("equal strings: got %v", d)
	}
	if d := Distance(StrV("a"), StrV("b")); d != 1.0 {
		t.Errorf("unequal strings: got %v", d)
	}
	if d := Distance(NullV(), IntV(1)); d != 1.0 {
		t.Errorf("null vs number: got %v", d)
	}
}
