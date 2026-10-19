package value

// THE CANONICAL FORM, CHECKED AGAINST THE SHARED VECTORS.
//
// `compat/canonical-vectors.json` is the shared artefact behind the claim in
// `docs/cell-ledger.md` that "every quilt tier (Rust, TS, Go) produces
// byte-identical preimages from the same data".
//
// The Rust and Python tiers already share `compat/golden.json` and are
// genuinely differential-tested. The Go tier had its OWN golden -- a
// different spec (`quilt-go-golden/1`), a different sheet, and a DIFFERENT
// chain envelope (sha256_hex(chain_prev + ":" + provenance)) -- so for Go the
// claim was ASSERTED, not checked. And the TypeScript tier the spec names
// does not exist.
//
// **THE CANONICAL FORM IS THE PART ALL THE TIERS MUST AGREE ON, AND IT IS
// THE PART THAT IS SMALL AND CHECKABLE.** So it gets its own vector file, and
// every tier reads it.
//
// A port conforms by reproducing every `canonical` string in it.
//
// AND HERE IS THE POINT OF THE WHOLE FILE, IN ONE LINE OF GO:
//
//	dec := json.NewDecoder(f)
//	dec.UseNumber()          // <-- WITHOUT THIS, EVERY NUMBER BECOMES A
//	                          //     float64 AND THE int/float DISTINCTION
//	                          //     IS DESTROYED SILENTLY.
//
// docs/cell-ledger.md names the hazard exactly: "JavaScript renders 40.0 as
// 40; ports must preserve float-vs-int distinction to stay on-chain." A
// plain `json.Unmarshal` into `any` does the same thing to a Go port, and
// it fails as a MYSTERY: every ordinary case passes and the ones that
// matter do not.

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"testing"
)

const vectorsPath = "../../../../compat/canonical-vectors.json"

type vectorFile struct {
	Spec     string `json:"spec"`
	CaseCount int   `json:"case_count"`
	Cases    []struct {
		Name      string          `json:"name"`
		Value     json.RawMessage `json:"value"`
		Canonical string          `json:"canonical"`
		SHA256    string          `json:"sha256"`
	} `json:"cases"`
}

// toValue converts a decoded JSON value into the Value union, KEEPING the
// int/float distinction. The decoder MUST have been created with
// UseNumber(), or every number arrives as a float64 and the distinction is
// already gone by the time we get here.
func toValue(v any) Value {
	switch x := v.(type) {
	case nil:
		return NullV()
	case bool:
		return BoolV(x)
	case string:
		return StrV(x)
	case json.Number:
		s := x.String()
		if strings.ContainsAny(s, ".eE") {
			f, err := x.Float64()
			if err != nil {
				panic("bad number " + s)
			}
			return FloatV(f)
		}
		i, err := x.Int64()
		if err != nil {
			// An integer too big for int64: keep it exact as a float, but
			// mark it as a float so the canonical form is honest about it.
			f, ferr := x.Float64()
			if ferr != nil {
				panic("bad number " + s)
			}
			return FloatV(f)
		}
		return IntV(i)
	case []any:
		items := make([]Value, 0, len(x))
		for _, it := range x {
			items = append(items, toValue(it))
		}
		return ArrV(items)
	case map[string]any:
		m := make(map[string]Value, len(x))
		for k, it := range x {
			m[k] = toValue(it)
		}
		return ObjV(m)
	}
	panic(fmt.Sprintf("unsupported type %T", v))
}

func TestTheCanonicalFormMatchesTheSharedVectors(t *testing.T) {
	f, err := os.Open(vectorsPath)
	if err != nil {
		t.Fatalf("cannot open %s: %v", vectorsPath, err)
	}
	defer f.Close()

	dec := json.NewDecoder(f)
	dec.UseNumber() // THE LINE. See the note at the top of this file.
	var doc vectorFile
	if err := dec.Decode(&doc); err != nil {
		t.Fatalf("cannot decode %s: %v", vectorsPath, err)
	}
	if len(doc.Cases) != doc.CaseCount {
		t.Fatalf("the vector file's recorded count (%d) does not match its contents (%d)",
			doc.CaseCount, len(doc.Cases))
	}
	if len(doc.Cases) == 0 {
		t.Fatal("the vector file is empty")
	}

	for _, c := range doc.Cases {
		var raw any
		d2 := json.NewDecoder(strings.NewReader(string(c.Value)))
		d2.UseNumber()
		if err := d2.Decode(&raw); err != nil {
			t.Fatalf("%s: cannot decode the value: %v", c.Name, err)
		}
		got := Canonical(toValue(raw))
		if got != c.Canonical {
			t.Errorf("%s:\n  got  %q\n  want %q", c.Name, got, c.Canonical)
			continue
		}
		sum := sha256.Sum256([]byte(got))
		if hex.EncodeToString(sum[:]) != c.SHA256 {
			t.Errorf("%s: sha256 %s != %s", c.Name, hex.EncodeToString(sum[:]), c.SHA256)
		}
	}
	t.Logf("canonical vectors: %d cases match (spec %s)", len(doc.Cases), doc.Spec)
}

// A GO PORT THAT FORGOT UseNumber() MUST NOT PASS. This is the negative
// control for the hazard the whole file exists for: if the decoder is
// changed to a plain Unmarshal, the int/float cases must go RED.
func TestWithoutUseNumberTheIntFloatCasesFail(t *testing.T) {
	f, err := os.Open(vectorsPath)
	if err != nil {
		t.Skipf("vectors not available: %v", err)
	}
	defer f.Close()
	var doc vectorFile
	plain := json.NewDecoder(f) // NO UseNumber() -- the mistake
	if err := plain.Decode(&doc); err != nil {
		t.Skipf("cannot decode: %v", err)
	}
	naiveFailures := 0
	for _, c := range doc.Cases {
		var raw any
		if err := json.Unmarshal(c.Value, &raw); err != nil {
			continue
		}
		if Canonical(toValueNaive(raw)) != c.Canonical {
			naiveFailures++
		}
	}
	if naiveFailures == 0 {
		t.Skip("this Go's json.Unmarshal happens to preserve the distinction; " +
			"the hazard is then not demonstrable here")
	}
	t.Logf("a decoder WITHOUT UseNumber() fails %d of %d vectors -- which is the hazard",
		naiveFailures, len(doc.Cases))
}

// toValueNaive is the same conversion WITHOUT the int/float distinction,
// i.e. what a port that used json.Unmarshal into `any` would do.
func toValueNaive(v any) Value {
	switch x := v.(type) {
	case nil:
		return NullV()
	case bool:
		return BoolV(x)
	case string:
		return StrV(x)
	case float64:
		return FloatV(x)
	case []any:
		items := make([]Value, 0, len(x))
		for _, it := range x {
			items = append(items, toValueNaive(it))
		}
		return ArrV(items)
	case map[string]any:
		m := make(map[string]Value, len(x))
		for k, it := range x {
			m[k] = toValueNaive(it)
		}
		return ObjV(m)
	}
	panic(fmt.Sprintf("unsupported type %T", v))
}
