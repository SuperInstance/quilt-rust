// Package engine is the reactive cell runtime for quilt-go: value and
// formula cells, lazy-looking but eager-level evaluation with goroutine
// fan-out — cells in the same topological level are independent, so they
// evaluate concurrently; results are then applied serially in sorted id
// order, keeping ledger chains deterministic bit-for-bit.
//
// Semantics mirror the Rust/TS tiers: setting a value cell records an edge
// and recomputes the transitive dependents in dependency order; every
// recompute records an edge in the cell's own ledger (the first-person
// record of the change).
package engine

import (
	"fmt"
	"sort"
	"sync"

	"quilt-go/internal/expr"
	"quilt-go/internal/ledger"
	"quilt-go/internal/sheet"
	"quilt-go/internal/value"
)

// Cell is a runtime cell.
type Cell struct {
	ID         string
	Kind       string // "value" | "formula"
	Expr       string
	Deps       []string
	Dependents []string
	State      value.Value
	Ledger     *ledger.Ledger
	level      int
}

// Engine is a loaded sheet: cells, dependency graph, levels.
type Engine struct {
	SheetID  string
	cells    map[string]*Cell
	order    []string // sorted ids, for deterministic iteration
	maxLevel int
}

// LoadSheet builds an engine from a parsed sheet.
func LoadSheet(def *sheet.SheetDef) (*Engine, error) {
	e := &Engine{SheetID: def.ID, cells: map[string]*Cell{}}
	for _, cd := range def.Cells {
		c := &Cell{ID: cd.ID, Kind: cd.Kind, Expr: cd.Expr, Deps: append([]string{}, cd.Deps...)}
		if cd.Kind == "value" {
			c.State = cd.Value
			c.Ledger = ledger.WithGenesis(cd.ID, cd.Value, 0)
		} else {
			c.State = value.NullV()
			// Formula cells declare a null genesis: the first evaluation
			// is scored against the persistence prior like any other edge.
			c.Ledger = ledger.WithGenesis(cd.ID, value.NullV(), 0)
		}
		e.cells[cd.ID] = c
	}

	// Dependency auto-detection for formulas: scan the expression for
	// identifiers that name known cells (same rule as the Rust engine's
	// expr_contains_token, over whole dotted tokens).
	for _, c := range e.cells {
		if c.Kind != "formula" {
			continue
		}
		known := map[string]bool{}
		for _, d := range c.Deps {
			known[d] = true
		}
		for _, ident := range expr.Identifiers(c.Expr) {
			if ident == c.ID {
				return nil, fmt.Errorf("cell %q references itself", c.ID)
			}
			if _, ok := e.cells[ident]; ok && !known[ident] {
				c.Deps = append(c.Deps, ident)
				known[ident] = true
			}
		}
		sort.Strings(c.Deps)
	}
	// Validate deps exist (declared deps may reference anything).
	for _, c := range e.cells {
		for _, d := range c.Deps {
			if _, ok := e.cells[d]; !ok {
				return nil, fmt.Errorf("cell %q depends on undefined cell %q", c.ID, d)
			}
		}
	}
	// Dependents (reverse edges).
	for _, c := range e.cells {
		for _, d := range c.Deps {
			dep := e.cells[d]
			dep.Dependents = append(dep.Dependents, c.ID)
		}
	}
	for _, c := range e.cells {
		sort.Strings(c.Dependents)
	}
	// Topological levels via memoized DFS; cycles are an error.
	const (
		white = 0
		gray  = 1
		black = 2
	)
	color := map[string]int{}
	var level func(id string) (int, error)
	level = func(id string) (int, error) {
		switch color[id] {
		case gray:
			return 0, fmt.Errorf("dependency cycle involving %q", id)
		case black:
			return e.cells[id].level, nil
		}
		color[id] = gray
		lvl := 0
		for _, d := range e.cells[id].Deps {
			dl, err := level(d)
			if err != nil {
				return 0, err
			}
			if dl+1 > lvl {
				lvl = dl + 1
			}
		}
		color[id] = black
		e.cells[id].level = lvl
		if lvl > e.maxLevel {
			e.maxLevel = lvl
		}
		return lvl, nil
	}
	for id := range e.cells {
		if _, err := level(id); err != nil {
			return nil, err
		}
		e.order = append(e.order, id)
	}
	sort.Strings(e.order)
	return e, nil
}

// Cell returns a cell by id.
func (e *Engine) Cell(id string) *Cell { return e.cells[id] }

// IDs returns all cell ids, sorted.
func (e *Engine) IDs() []string { return append([]string{}, e.order...) }

// Get returns the cell's current state (pure cells are always current
// after EvalAll / Set propagation).
func (e *Engine) Get(id string) (value.Value, error) {
	c, ok := e.cells[id]
	if !ok {
		return value.Value{}, fmt.Errorf("unknown cell %q", id)
	}
	return c.State, nil
}

// snapshotDeps reads the current state of a cell's dependencies.
func (e *Engine) snapshotDeps(c *Cell) map[string]value.Value {
	env := make(map[string]value.Value, len(c.Deps))
	for _, d := range c.Deps {
		env[d] = e.cells[d].State
	}
	return env
}

// evalLevel evaluates the given cells concurrently (they are pairwise
// independent within a level), then applies results in sorted id order so
// ledger appends — and therefore chain hashes — are deterministic.
func (e *Engine) evalLevel(ids []string, ts int64, origin ledger.ProvenanceInput) error {
	type result struct {
		id  string
		val value.Value
		err error
	}
	results := make([]result, len(ids))
	var wg sync.WaitGroup
	for i, id := range ids {
		wg.Add(1)
		go func(i int, id string) {
			defer wg.Done()
			c := e.cells[id]
			val, err := expr.Eval(c.Expr, e.snapshotDeps(c))
			results[i] = result{id, val, err}
		}(i, id)
	}
	wg.Wait()
	sort.Slice(results, func(i, j int) bool { return results[i].id < results[j].id })
	for _, r := range results {
		if r.err != nil {
			return fmt.Errorf("cell %q: %w", r.id, r.err)
		}
		c := e.cells[r.id]
		c.Ledger.Record(r.val, ts, origin)
		c.State = r.val
	}
	return nil
}

// EvalAll evaluates every formula cell, level by level; cells within a
// level run in their own goroutines. Value cells are already at their
// genesis state. Returns after the whole sheet is current.
func (e *Engine) EvalAll(ts int64) error {
	for lvl := 1; lvl <= e.maxLevel; lvl++ {
		var ids []string
		for _, id := range e.order {
			if e.cells[id].level == lvl && e.cells[id].Kind == "formula" {
				ids = append(ids, id)
			}
		}
		if len(ids) > 0 {
			if err := e.evalLevel(ids, ts, ledger.ProvenanceInput{Origin: "get"}); err != nil {
				return err
			}
		}
	}
	return nil
}

// Set writes a value cell, records the edge, and reactively recomputes its
// transitive dependents — level by level, parallel within a level.
func (e *Engine) Set(id string, v value.Value, ts int64) error {
	c, ok := e.cells[id]
	if !ok {
		return fmt.Errorf("unknown cell %q", id)
	}
	if c.Kind != "value" {
		return fmt.Errorf("set is only supported on value cells in quilt-go (cell %q is %s)", id, c.Kind)
	}
	c.Ledger.Record(v, ts, ledger.ProvenanceInput{Origin: "set"})
	c.State = v

	// Transitive dependents, grouped by level.
	dirty := map[string]bool{}
	queue := append([]string{}, c.Dependents...)
	for len(queue) > 0 {
		id := queue[0]
		queue = queue[1:]
		if dirty[id] {
			continue
		}
		dirty[id] = true
		queue = append(queue, e.cells[id].Dependents...)
	}
	sysProv := ledger.ProvenanceInput{Origin: "system"}
	for lvl := 1; lvl <= e.maxLevel; lvl++ {
		var ids []string
		for id := range dirty {
			if e.cells[id].level == lvl && e.cells[id].Kind == "formula" {
				ids = append(ids, id)
			}
		}
		if len(ids) > 0 {
			if err := e.evalLevel(ids, ts, sysProv); err != nil {
				return err
			}
		}
	}
	return nil
}

// SheetChainHash commits to the whole sheet state: sha256 over the
// canonical JSON of {"chains": {cell_id: chain_hash}, "sheet": sheet_id}.
func (e *Engine) SheetChainHash() string {
	chains := make(map[string]value.Value, len(e.cells))
	for _, id := range e.order {
		chains[id] = value.StrV(e.cells[id].Ledger.ChainHash())
	}
	return ledger.Hash(value.Canonical(value.ObjV(map[string]value.Value{
		"chains": value.ObjV(chains),
		"sheet":  value.StrV(e.SheetID),
	})))
}

// VerifyChains re-verifies every cell ledger (links and seals).
func (e *Engine) VerifyChains() bool {
	for _, id := range e.order {
		c := e.cells[id]
		if _, ok := c.Ledger.VerifyChain(); !ok {
			return false
		}
		if !c.Ledger.VerifySeals() {
			return false
		}
	}
	return true
}
