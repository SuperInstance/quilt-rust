package expr

import (
	"testing"

	"quilt-go/internal/value"
)

func eval(t *testing.T, src string, env map[string]value.Value) value.Value {
	t.Helper()
	v, err := Eval(src, env)
	if err != nil {
		t.Fatalf("Eval(%q): %v", src, err)
	}
	return v
}

func TestArithmetic(t *testing.T) {
	env := map[string]value.Value{"temp": value.IntV(21)}
	if v := eval(t, "=temp * 9 / 5 + 32", env); v.K != value.Float || v.F != 69.8 {
		t.Errorf("got %s, want 69.8", value.Canonical(v))
	}
	// Int-preserving ops.
	if v := eval(t, "=1 + 2 * 3", nil); v.K != value.Int || v.I != 7 {
		t.Errorf("got %s, want 7 (int)", value.Canonical(v))
	}
	if v := eval(t, "=(540 - 180) % 360", nil); v.K != value.Int || v.I != 0 {
		t.Errorf("got %s, want 0 (int)", value.Canonical(v))
	}
	// Division is always float.
	if v := eval(t, "=10 / 4", nil); v.K != value.Float || v.F != 2.5 {
		t.Errorf("got %s, want 2.5", value.Canonical(v))
	}
}

func TestTernaryAndStrings(t *testing.T) {
	env := map[string]value.Value{"is_dark": value.BoolV(true)}
	if v := eval(t, "=is_dark ? 'ON' : 'OFF'", env); v.S != "ON" {
		t.Errorf("got %s", value.Canonical(v))
	}
	env2 := map[string]value.Value{"hello": value.StrV("Hello")}
	if v := eval(t, "=hello + ', world!'", env2); v.S != "Hello, world!" {
		t.Errorf("got %s", value.Canonical(v))
	}
}

func TestComparisonAndLogic(t *testing.T) {
	env := map[string]value.Value{
		"ambient.light": value.IntV(400),
		"threshold":     value.IntV(200),
	}
	if v := eval(t, "=ambient.light < threshold", env); !v.B {
		// 400 < 200 is false — check it is false, not error
	} else {
		t.Errorf("got true, want false")
	}
	if v := eval(t, "=ambient.light > threshold && threshold > 0", env); !v.B {
		t.Errorf("got false, want true")
	}
	if v := eval(t, "=!(ambient.light < threshold)", env); !v.B {
		t.Errorf("got false, want true")
	}
}

func TestFunctions(t *testing.T) {
	env := map[string]value.Value{"error": value.FloatV(5.0)}
	if v := eval(t, "=clamp(error * 0.5, -30, 30)", env); v.F != 2.5 {
		t.Errorf("got %s, want 2.5", value.Canonical(v))
	}
	if v := eval(t, "=abs(-3)", nil); v.F != 3.0 {
		t.Errorf("got %s", value.Canonical(v))
	}
	if v := eval(t, "=max(1, 9, 4)", nil); v.F != 9.0 {
		t.Errorf("got %s", value.Canonical(v))
	}
	if v := eval(t, "=min(1, 9, 4)", nil); v.F != 1.0 {
		t.Errorf("got %s", value.Canonical(v))
	}
}

func TestIdentifiers(t *testing.T) {
	ids := Identifiers("=clamp(error * 0.5, -30, 30) + ambient.light")
	want := map[string]bool{"clamp": false, "error": true, "ambient.light": true}
	got := map[string]bool{}
	for _, id := range ids {
		got[id] = true
	}
	if got["clamp"] {
		t.Errorf("function name reported as identifier")
	}
	for id, w := range want {
		if w && !got[id] {
			t.Errorf("missing identifier %q", id)
		}
	}
}
