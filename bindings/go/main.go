// quilt-go — the parallel/optimizing tier of the quilt runtime.
//
// Usage:
//
//	quilt-go run <sheet.yaml>      Load a sheet, evaluate every cell, print state.
//	quilt-go golden [out.json]     Run the golden-vector scenario, print the
//	                               numbers, and write golden.json (default:
//	                               golden.json in the current directory).
package main

import (
	"fmt"
	"os"
	"time"

	"quilt-go/internal/engine"
	"quilt-go/internal/golden"
	"quilt-go/internal/sheet"
	"quilt-go/internal/value"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: quilt-go run <sheet.yaml> | quilt-go golden [out.json]")
		os.Exit(1)
	}
	switch os.Args[1] {
	case "run":
		if len(os.Args) < 3 {
			fmt.Fprintln(os.Stderr, "usage: quilt-go run <sheet.yaml>")
			os.Exit(1)
		}
		if err := runSheet(os.Args[2]); err != nil {
			fmt.Fprintln(os.Stderr, "error:", err)
			os.Exit(1)
		}
	case "golden":
		out := "golden.json"
		if len(os.Args) >= 3 {
			out = os.Args[2]
		}
		if err := runGolden(out); err != nil {
			fmt.Fprintln(os.Stderr, "error:", err)
			os.Exit(1)
		}
	default:
		fmt.Fprintln(os.Stderr, "unknown command:", os.Args[1])
		os.Exit(1)
	}
}

func runSheet(path string) error {
	src, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	def, err := sheet.Parse(string(src))
	if err != nil {
		return err
	}
	eng, err := engine.LoadSheet(def)
	if err != nil {
		return err
	}
	if err := eng.EvalAll(time.Now().UnixMilli()); err != nil {
		return err
	}
	fmt.Printf("Sheet: %s (%d cells)\n", eng.SheetID, len(eng.IDs()))
	for _, id := range eng.IDs() {
		c := eng.Cell(id)
		fmt.Printf("  %-24s [%-7s] %s\n", id, c.Kind, value.Display(c.State))
	}
	return nil
}

func runGolden(out string) error {
	report, eng, err := golden.Run()
	if err != nil {
		return err
	}
	fmt.Print(value.Pretty(report))
	if err := os.WriteFile(out, []byte(value.Pretty(report)), 0o644); err != nil {
		return err
	}
	if eng.VerifyChains() {
		fmt.Println("chains verified: OK")
	} else {
		fmt.Println("chains verified: FAIL")
	}
	fmt.Println("golden.json written to", out)
	return nil
}
