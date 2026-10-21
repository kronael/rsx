package soft_test

import (
	"image"
	"testing"

	"rsx-glass"
	"rsx-glass/soft"
)

const accountsGridW, accountsGridH = 72, 12

// renderAccountsGrid mirrors renderGrid (soft_test.go) for the read-only
// accounts overview panel instead of the heatmap.
func renderAccountsGrid(t *testing.T, rows []glass.AccountRow) *image.RGBA {
	t.Helper()
	g := glass.NewGrid(accountsGridW, accountsGridH)
	glass.ComposeAccounts(g, rows)
	r, err := soft.New()
	if err != nil {
		t.Fatalf("soft.New: %v", err)
	}
	if err := r.Present(g); err != nil {
		t.Fatalf("Present: %v", err)
	}
	return r.Image()
}

// TestGoldenAccounts byte-locks the read-only accounts/wallet overview panel
// (the read-only USERS screen of specs/08-screen-users.md — no CRUD, see accounts.go).
func TestGoldenAccounts(t *testing.T) {
	goldenPNG(t, "accounts.png", renderAccountsGrid(t, glass.FixtureAccountRows()))
}

// TestGoldenAccountsEmpty locks the honest empty/offline state — no rows
// must never render as a table of fabricated zeros.
func TestGoldenAccountsEmpty(t *testing.T) {
	goldenPNG(t, "accounts_empty.png", renderAccountsGrid(t, nil))
}

// TestComposeAccountsAllocFree matches TestPresentAllocFree's guarantee for
// the heatmap: rasterising an already-built grid allocates nothing.
func TestComposeAccountsAllocFree(t *testing.T) {
	g := glass.NewGrid(accountsGridW, accountsGridH)
	rows := glass.FixtureAccountRows()
	glass.ComposeAccounts(g, rows)
	r, err := soft.New()
	if err != nil {
		t.Fatalf("soft.New: %v", err)
	}
	if err := r.Present(g); err != nil {
		t.Fatal(err)
	}
	allocs := testing.AllocsPerRun(200, func() { _ = r.Present(g) })
	if allocs != 0 {
		t.Fatalf("Present allocates %.0f times per frame, want 0", allocs)
	}
}
