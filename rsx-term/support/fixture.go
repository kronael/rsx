package support

import "fmt"

// FixtureAccounts is a deterministic offline AccountClient: canned data
// keyed by user_id, no network. Stands in for the gateway's not-yet-shipped
// spec-7 support surface (see http.go) so the Users/Wallets screen has
// something real to render in the default/mock/test/golden path — mirrors
// conn.MockGateway's role for order flow. Writes are accepted (mutate the
// in-memory fixture) and returned, exactly like a real backend would, so the
// screen's write path is exercised end-to-end; nothing here talks to a
// network or a database.
type FixtureAccounts struct {
	accounts  map[uint32]AccountSummary
	positions map[uint32][]PositionRow
	fills     map[uint32][]FillRow
	funding   map[uint32][]FundingRow
	freezes   map[uint32][]FreezeRow
}

// NewFixtureAccounts seeds a small, plausible book for the demo user set
// (matches the playground's PENGU-PERP=10 / SOL-PERP=3 convention).
func NewFixtureAccounts() *FixtureAccounts {
	return &FixtureAccounts{
		accounts: map[uint32]AccountSummary{
			1: {Collateral: 100_000_000, Equity: 104_500_000, UnrealizedPnl: 4_500_000,
				InitialMargin: 12_000_000, MaintMargin: 6_000_000, Available: 88_000_000},
			2: {Collateral: 25_000_000, Equity: 21_200_000, UnrealizedPnl: -3_800_000,
				InitialMargin: 8_000_000, MaintMargin: 4_000_000, Available: 17_000_000},
		},
		positions: map[uint32][]PositionRow{
			1: {{SymbolID: 10, Side: 0, Qty: 500_0000, EntryPx: 9_950_000, MarkPx: 10_040_000, UnrealizedPnl: 4_500_000, LiqPx: 8_100_000}},
			2: {{SymbolID: 3, Side: 1, Qty: 20_0000, EntryPx: 148_500_00, MarkPx: 149_690_00, UnrealizedPnl: -3_800_000, LiqPx: 172_000_00}},
		},
		fills: map[uint32][]FillRow{
			1: {{Oid: 9001, SymbolID: 10, Px: 9_950_000, Qty: 500_0000, Side: 0, Fee: 4_975, IsMaker: false, TsNs: 1_753_200_000_000_000_000}},
			2: {{Oid: 9002, SymbolID: 3, Px: 148_500_00, Qty: 20_0000, Side: 1, Fee: 2_970, IsMaker: true, TsNs: 1_753_100_000_000_000_000}},
		},
		funding: map[uint32][]FundingRow{
			1: {{SymbolID: 10, Amount: -1_200, RateBps: 1, TsNs: 1_753_200_000_000_000_000}},
			2: {{SymbolID: 3, Amount: 800, RateBps: -1, TsNs: 1_753_100_000_000_000_000}},
		},
		freezes: map[uint32][]FreezeRow{},
	}
}

func (f *FixtureAccounts) Enabled() bool { return f != nil }

func (f *FixtureAccounts) Account(userID uint32) (AccountSummary, error) {
	if a, ok := f.accounts[userID]; ok {
		return a, nil
	}
	return AccountSummary{}, nil // an unknown fixture user is honestly flat, not an error
}

func (f *FixtureAccounts) Positions(userID uint32) ([]PositionRow, error) {
	return f.positions[userID], nil
}

func (f *FixtureAccounts) Fills(userID uint32, symbolID uint32, limit int) ([]FillRow, error) {
	rows := f.fills[userID]
	if symbolID != 0 {
		var filtered []FillRow
		for _, r := range rows {
			if r.SymbolID == symbolID {
				filtered = append(filtered, r)
			}
		}
		rows = filtered
	}
	if limit > 0 && len(rows) > limit {
		rows = rows[:limit]
	}
	return rows, nil
}

func (f *FixtureAccounts) Funding(userID uint32, symbolID uint32, limit int) ([]FundingRow, error) {
	rows := f.funding[userID]
	if symbolID != 0 {
		var filtered []FundingRow
		for _, r := range rows {
			if r.SymbolID == symbolID {
				filtered = append(filtered, r)
			}
		}
		rows = filtered
	}
	if limit > 0 && len(rows) > limit {
		rows = rows[:limit]
	}
	return rows, nil
}

func (f *FixtureAccounts) Freezes(userID uint32) ([]FreezeRow, error) {
	return f.freezes[userID], nil
}

func (f *FixtureAccounts) AdjustCollateral(userID uint32, delta int64, reason, ticketID string) (AccountSummary, error) {
	a := f.accounts[userID]
	a.Collateral += delta
	a.Equity += delta
	a.Available += delta
	f.accounts[userID] = a
	return a, nil
}

func (f *FixtureAccounts) FreezeUpsert(userID uint32, orderID uint64, reason, ticketID string) (FreezeRow, error) {
	row := FreezeRow{UserID: userID, OrderID: orderID, Reason: reason, TicketID: ticketID, CreatedAt: "fixture"}
	rows := f.freezes[userID]
	for i, r := range rows {
		if r.OrderID == orderID {
			rows[i] = row
			f.freezes[userID] = rows
			return row, nil
		}
	}
	f.freezes[userID] = append(rows, row)
	return row, nil
}

func (f *FixtureAccounts) FreezeDelete(userID uint32, orderID uint64, reason, ticketID string) error {
	rows := f.freezes[userID]
	for i, r := range rows {
		if r.OrderID == orderID {
			f.freezes[userID] = append(rows[:i], rows[i+1:]...)
			return nil
		}
	}
	return fmt.Errorf("support: no freeze for user %d order %d", userID, orderID)
}

func (f *FixtureAccounts) AddNote(userID uint32, note, reason, ticketID string) error {
	return nil // notes aren't read back by this screen in v1; accepted, not stored
}
