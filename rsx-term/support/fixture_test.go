package support

import "testing"

func TestFixtureAccountsDeterministic(t *testing.T) {
	f := NewFixtureAccounts()
	a, err := f.Account(1)
	if err != nil {
		t.Fatalf("Account: %v", err)
	}
	if a.Collateral != 100_000_000 {
		t.Fatalf("collateral = %d, want a fixed seeded value", a.Collateral)
	}
	positions, _ := f.Positions(1)
	if len(positions) != 1 || positions[0].SymbolID != 10 {
		t.Fatalf("unexpected positions: %+v", positions)
	}
}

func TestFixtureAccountsUnknownUserIsFlatNotError(t *testing.T) {
	f := NewFixtureAccounts()
	a, err := f.Account(999)
	if err != nil {
		t.Fatalf("an unseeded fixture user must not error: %v", err)
	}
	if a != (AccountSummary{}) {
		t.Fatalf("unseeded user should be flat zero, got %+v", a)
	}
}

func TestFixtureAccountsAdjustCollateralMutatesInMemory(t *testing.T) {
	f := NewFixtureAccounts()
	before, _ := f.Account(1)
	after, err := f.AdjustCollateral(1, 5_000_000, "test", "T-1")
	if err != nil {
		t.Fatalf("AdjustCollateral: %v", err)
	}
	if after.Collateral != before.Collateral+5_000_000 {
		t.Fatalf("collateral after adjust = %d, want %d", after.Collateral, before.Collateral+5_000_000)
	}
	// The mutation sticks — a second read sees it (in-memory, not a
	// database, but exercises the write→read path end-to-end).
	again, _ := f.Account(1)
	if again.Collateral != after.Collateral {
		t.Fatalf("adjustment did not persist in the fixture: %+v", again)
	}
}

func TestFixtureAccountsFreezeUpsertThenDelete(t *testing.T) {
	f := NewFixtureAccounts()
	row, err := f.FreezeUpsert(1, 555, "investigating", "T-9")
	if err != nil {
		t.Fatalf("FreezeUpsert: %v", err)
	}
	freezes, _ := f.Freezes(1)
	if len(freezes) != 1 || freezes[0].OrderID != row.OrderID {
		t.Fatalf("freeze not recorded: %+v", freezes)
	}
	if err := f.FreezeDelete(1, 555, "resolved", "T-9"); err != nil {
		t.Fatalf("FreezeDelete: %v", err)
	}
	freezes, _ = f.Freezes(1)
	if len(freezes) != 0 {
		t.Fatalf("freeze should be gone: %+v", freezes)
	}
	if err := f.FreezeDelete(1, 555, "resolved", "T-9"); err == nil {
		t.Fatal("deleting a freeze that no longer exists must error, not silently succeed")
	}
}
