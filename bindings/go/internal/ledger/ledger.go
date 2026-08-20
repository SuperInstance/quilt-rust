// Package ledger records cell edges in the canonical schema and chains
// them with SHA-256, per the fleet design (docs/fleet-as-fractal-jepa.md):
// one ledger, many substrates; the edge (before → after, recorded from the
// perspective of the cell that changed) is the atom.
//
// Canonical edge schema (v1):
//
//	{"v":1,"cell":<id>,"ts":<millis>,"before":<value>,"after":<value>,
//	 "delta":<float>,"imbalance":<float|null>,"provenance":<hex>,
//	 "chain":<hex>}
//
// Semantics, pinned for cross-tier reproduction:
//
//   - delta     = value_distance(before, after) — the edge magnitude.
//   - imbalance = value_distance(expected, after) under the persistence
//     prior (expected = before); null when the cell has no prior (no
//     genesis and no previous entry). Never fake a number.
//   - provenance = sha256_hex(canonical_json({
//     "v":1,"cell","ts","before","after","delta","imbalance",
//     "origin","caller","trace"})) — the seal over the full edge body
//     plus who touched the cell. caller is null when unknown; trace is
//     the ancestor chain, outermost first.
//   - chain_i = sha256_hex(chain_{i-1} + ":" + provenance_i), where
//     chain_0 is the genesis commit:
//     sha256_hex(canonical_json({"kind":"quilt-edge/1","cell","genesis",
//     "genesis_ts"})).
//
// canonical_json is the pinned form from internal/value (compact, sorted
// keys, int/float distinction preserved). Timestamps are caller-supplied
// (millis): the ledger is a pure data structure with no clocks and no I/O.
package ledger

import (
	"crypto/sha256"
	"encoding/hex"

	"quilt-go/internal/value"
)

// GenesisKind is the "kind" tag of the genesis commit preimage.
const GenesisKind = "quilt-edge/1"

// SchemaVersion is the "v" field of every edge.
const SchemaVersion = 1

// ProvenanceInput — who touched the cell. Origin is one of the engine
// verbs: "get" | "set" | "call" | "push" | "system".
type ProvenanceInput struct {
	Origin string
	Caller string // "" means null
	Trace  []string
}

// Edge is one canonical ledger entry.
type Edge struct {
	Cell       string
	Ts         int64
	Before     value.Value
	After      value.Value
	Delta      float64
	Imbalance  *float64 // nil when no prior exists
	Provenance string   // sha256 hex seal of the edge body + provenance
	Chain      string   // running chain hash
}

func sha256hex(s string) string {
	sum := sha256.Sum256([]byte(s))
	return hex.EncodeToString(sum[:])
}

// Hash is sha256_hex of s — exposed for aggregate commitments (e.g. the
// sheet-level chain) that must hash canonical JSON the same way.
func Hash(s string) string { return sha256hex(s) }

// CanonicalBody renders the edge in canonical JSON, schema field order
// aside (canonical form sorts keys): the exact bytes a port must reproduce.
func (e Edge) CanonicalBody() string {
	var imb value.Value
	if e.Imbalance != nil {
		imb = value.FloatV(*e.Imbalance)
	} else {
		imb = value.NullV()
	}
	return value.Canonical(value.ObjV(map[string]value.Value{
		"after":      e.After,
		"before":     e.Before,
		"cell":       value.StrV(e.Cell),
		"chain":      value.StrV(e.Chain),
		"delta":      value.FloatV(e.Delta),
		"imbalance":  imb,
		"provenance": value.StrV(e.Provenance),
		"ts":         value.IntV(e.Ts),
		"v":          value.IntV(SchemaVersion),
	}))
}

// seal computes the provenance hash: sha256 over the canonical preimage of
// the full edge body plus provenance fields.
func seal(cell string, ts int64, before, after value.Value, delta float64, imbalance *float64, prov ProvenanceInput) string {
	var imb, caller value.Value
	if imbalance != nil {
		imb = value.FloatV(*imbalance)
	} else {
		imb = value.NullV()
	}
	if prov.Caller != "" {
		caller = value.StrV(prov.Caller)
	} else {
		caller = value.NullV()
	}
	trace := make([]value.Value, len(prov.Trace))
	for i, t := range prov.Trace {
		trace[i] = value.StrV(t)
	}
	preimage := value.Canonical(value.ObjV(map[string]value.Value{
		"v":         value.IntV(SchemaVersion),
		"cell":      value.StrV(cell),
		"ts":        value.IntV(ts),
		"before":    before,
		"after":     after,
		"delta":     value.FloatV(delta),
		"imbalance": imb,
		"origin":    value.StrV(prov.Origin),
		"caller":    caller,
		"trace":     value.ArrV(trace),
	}))
	return sha256hex(preimage)
}

// Ledger is a per-cell, append-only, hash-chained sequence of edges.
type Ledger struct {
	cellID     string
	genesis    value.Value
	genesisTs  int64
	hasGenesis bool
	state      value.Value
	entries    []Edge
	// provenance inputs per edge, kept (outside the seal) so VerifyChain
	// can recompute every seal from the edge body — tamper evidence.
	provs []ProvenanceInput
}

// New returns a ledger with no declared genesis (state null, no prior).
func New(cellID string) *Ledger {
	return &Ledger{cellID: cellID, state: value.NullV()}
}

// WithGenesis returns a ledger whose cell began life at a known state.
// The genesis is committed by the chain root and scores the first edge
// against the persistence prior.
func WithGenesis(cellID string, genesis value.Value, ts int64) *Ledger {
	return &Ledger{cellID: cellID, genesis: genesis, genesisTs: ts, hasGenesis: true, state: genesis}
}

// CellID returns the cell this ledger belongs to.
func (l *Ledger) CellID() string { return l.cellID }

// State is the cell's current state (the after of the last edge, else the
// genesis, else null).
func (l *Ledger) State() value.Value { return l.state }

// Entries returns the recorded edges in chain order.
func (l *Ledger) Entries() []Edge { return l.entries }

// GenesisCommit is the chain root: the hash that commits to the cell's
// identity and initial state before any edge exists.
func (l *Ledger) GenesisCommit() string {
	var g, gts value.Value
	if l.hasGenesis {
		g = l.genesis
		gts = value.IntV(l.genesisTs)
	} else {
		g = value.NullV()
		gts = value.NullV()
	}
	return sha256hex(value.Canonical(value.ObjV(map[string]value.Value{
		"cell":       value.StrV(l.cellID),
		"genesis":    g,
		"genesis_ts": gts,
		"kind":       value.StrV(GenesisKind),
	})))
}

// ChainHash is the head of the chain: the last edge's chain, or the
// genesis commit for an empty ledger.
func (l *Ledger) ChainHash() string {
	if len(l.entries) == 0 {
		return l.GenesisCommit()
	}
	return l.entries[len(l.entries)-1].Chain
}

// Record appends the edge (state → after) at ts with the given provenance
// and returns the sealed entry.
func (l *Ledger) Record(after value.Value, ts int64, prov ProvenanceInput) Edge {
	before := l.state
	delta := value.Distance(before, after)

	// Persistence prior: expected = before, so imbalance == delta by
	// construction. With no prior (no genesis, no entries) no surprise is
	// claimed — imbalance stays null.
	var imbalance *float64
	if l.hasGenesis || len(l.entries) > 0 {
		d := delta
		imbalance = &d
	}

	edge := Edge{
		Cell:       l.cellID,
		Ts:         ts,
		Before:     before,
		After:      after,
		Delta:      delta,
		Imbalance:  imbalance,
		Provenance: seal(l.cellID, ts, before, after, delta, imbalance, prov),
	}
	edge.Chain = sha256hex(l.ChainHash() + ":" + edge.Provenance)

	l.state = after
	l.entries = append(l.entries, edge)
	l.provs = append(l.provs, prov)
	return edge
}

// VerifyChain recomputes every seal from the edge body (using the stored
// provenance inputs) and every link from the recomputed seal. Any edit to
// any edge — body, provenance, or order — breaks verification at the
// edited edge. Returns the number of verified edges and whether the whole
// chain is intact.
func (l *Ledger) VerifyChain() (int, bool) {
	prev := l.GenesisCommit()
	for i, e := range l.entries {
		if e.Delta != value.Distance(e.Before, e.After) {
			return i, false
		}
		if seal(e.Cell, e.Ts, e.Before, e.After, e.Delta, e.Imbalance, l.provs[i]) != e.Provenance {
			return i, false
		}
		if e.Chain != sha256hex(prev+":"+e.Provenance) {
			return i, false
		}
		prev = e.Chain
	}
	return len(l.entries), true
}

// VerifySeals recomputes each edge's provenance seal from its body using
// the stored provenance inputs. (VerifyChain subsumes this; it is kept as
// a cheap seal-only check.)
func (l *Ledger) VerifySeals() bool {
	for i, e := range l.entries {
		if seal(e.Cell, e.Ts, e.Before, e.After, e.Delta, e.Imbalance, l.provs[i]) != e.Provenance {
			return false
		}
		if e.Delta != value.Distance(e.Before, e.After) {
			return false
		}
	}
	return true
}
