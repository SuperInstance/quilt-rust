package ledger

import (
	"testing"

	"quilt-go/internal/value"
)

func TestRecordEdgeSemantics(t *testing.T) {
	l := WithGenesis("bilge.level", value.FloatV(40.0), 1000)
	e := l.Record(value.FloatV(85.0), 2000, ProvenanceInput{Origin: "push", Caller: "bilge.adapter"})
	if e.Delta != 45.0 {
		t.Errorf("delta: got %v, want 45.0", e.Delta)
	}
	if e.Imbalance == nil || *e.Imbalance != 45.0 {
		t.Errorf("imbalance: got %v, want 45.0 (persistence prior)", e.Imbalance)
	}
	if len(e.Provenance) != 64 || len(e.Chain) != 64 {
		t.Errorf("hashes wrong length: %d %d", len(e.Provenance), len(e.Chain))
	}
	if l.ChainHash() != e.Chain {
		t.Errorf("chain head mismatch")
	}
}

func TestNoPriorClaimsNoSurprise(t *testing.T) {
	l := New("fresh")
	e := l.Record(value.IntV(1), 1, ProvenanceInput{Origin: "set"})
	if e.Imbalance != nil {
		t.Errorf("imbalance: got %v, want null (no prior)", *e.Imbalance)
	}
	e2 := l.Record(value.IntV(4), 2, ProvenanceInput{Origin: "set"})
	if e2.Imbalance == nil || *e2.Imbalance != 3.0 {
		t.Errorf("second edge imbalance: got %v, want 3.0", e2.Imbalance)
	}
}

func TestChainVerification(t *testing.T) {
	l := WithGenesis("x", value.IntV(0), 0)
	for i := int64(1); i <= 3; i++ {
		l.Record(value.IntV(i), i, ProvenanceInput{Origin: "set"})
	}
	if n, ok := l.VerifyChain(); !ok || n != 3 {
		t.Errorf("verify chain: verified=%d ok=%v", n, ok)
	}
	if !l.VerifySeals() {
		t.Error("verify seals failed")
	}
	// Tamper: editing an entry breaks its seal (recomputed from the body).
	l.entries[1].After = value.IntV(99)
	if _, ok := l.VerifyChain(); ok {
		t.Error("tampered chain still verifies")
	}
}

func TestGenesisCommitStable(t *testing.T) {
	a := WithGenesis("x", value.IntV(0), 0).GenesisCommit()
	b := WithGenesis("x", value.IntV(0), 0).GenesisCommit()
	c := WithGenesis("x", value.IntV(1), 0).GenesisCommit()
	if a != b {
		t.Error("same genesis, different commits")
	}
	if a == c {
		t.Error("different genesis, same commit")
	}
}
