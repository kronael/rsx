package glass

import "testing"

// cellText reads back the glyphs of row y starting at x — a small helper
// for asserting what ComposeAccounts wrote, mirroring heat_test.go's style.
func cellText(g *Grid, x, y, n int) string {
	var out []rune
	for i := 0; i < n; i++ {
		c := g.At(x+i, y)
		if c == nil {
			break
		}
		out = append(out, c.Glyph)
	}
	return string(out)
}

func TestComposeAccountsRendersRows(t *testing.T) {
	g := NewGrid(72, 12)
	ComposeAccounts(g, FixtureAccountRows())
	if got := cellText(g, 1, 3, 6); got != "1     " {
		t.Fatalf("row 0 id column = %q, want \"1     \"", got)
	}
	if got := cellText(g, 8, 3, 5); got != "alice" {
		t.Fatalf("row 0 login column = %q, want alice", got)
	}
}

func TestComposeAccountsColoursUpnlBySign(t *testing.T) {
	g := NewGrid(72, 12)
	ComposeAccounts(g, []AccountRow{
		{UserID: 1, Login: "up", UnrealizedPnl: 100},
		{UserID: 2, Login: "down", UnrealizedPnl: -100},
		{UserID: 3, Login: "flat", UnrealizedPnl: 0},
	})
	upCell := g.At(58, 3)
	downCell := g.At(58, 4)
	flatCell := g.At(58, 5)
	if upCell.Fg != Live {
		t.Fatalf("positive uPnL should be Live-coloured, got %v", upCell.Fg)
	}
	if downCell.Fg != Ask {
		t.Fatalf("negative uPnL should be Ask-coloured, got %v", downCell.Fg)
	}
	if flatCell.Fg != Text {
		t.Fatalf("flat uPnL should be plain Text-coloured, got %v", flatCell.Fg)
	}
}

func TestComposeAccountsDashesNoRole(t *testing.T) {
	g := NewGrid(72, 12)
	ComposeAccounts(g, []AccountRow{{UserID: 9, Login: "noone", Role: ""}})
	if got := cellText(g, 23, 3, 1); got != "—" {
		t.Fatalf("a user with no management role should show a dash, got %q", got)
	}
}

func TestComposeAccountsHonestWhenEmpty(t *testing.T) {
	g := NewGrid(72, 12)
	ComposeAccounts(g, nil)
	if got := cellText(g, 1, 3, 10); got != "no account" {
		t.Fatalf("empty rows should show an honest offline note, got %q", got)
	}
}

func TestComposeAccountsTooSmallGridIsNoop(t *testing.T) {
	g := NewGrid(10, 3)
	ComposeAccounts(g, FixtureAccountRows()) // must not panic on a too-small grid
}
