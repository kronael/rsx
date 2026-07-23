// Package support is the RSX user-management client: identity/RBAC against
// rsx-auth (real — rsx-auth/src/rsx_auth/app.py's /roles/* surface) and
// account/wallet trading state against the gateway's spec-7 support surface
// (specs/2/7-dashboard.md). The gateway side is STUB/FIXTURE ONLY today — the
// gateway is mid-rewrite and hasn't shipped `/v1/api/support/*` yet. See
// http.go's doc comment for the exact contract this client expects and
// fixture.go for the offline data it renders against meanwhile; the plan is
// tracked in .ship/48-USER-WALLET-MGMT/PLAN.md.
package support

import "errors"

// errOffline is returned by every call on an unconfigured (offline) client —
// never a fabricated user, role, or balance.
var errOffline = errors.New("support: not configured (offline)")

// UserRow is one identity+role directory entry (rsx-auth GET /roles/users).
type UserRow struct {
	UserID      uint32
	Login       string
	Email       string
	Role        string // "" = no management role (spec 7 §4 deny-by-default)
	CreatedAt   string
	LastLoginAt string
}

// AccountSummary is a user's account state (gateway GET .../account).
type AccountSummary struct {
	Collateral    int64
	Equity        int64
	UnrealizedPnl int64
	InitialMargin int64
	MaintMargin   int64
	Available     int64
}

// PositionRow is one non-zero position (gateway GET .../positions). Side
// mirrors the wire encoding (0 buy/long, 1 sell/short — specs/2/49-webproto.md).
type PositionRow struct {
	SymbolID      uint32
	Side          int32
	Qty           int64
	EntryPx       int64
	MarkPx        int64
	UnrealizedPnl int64
	LiqPx         int64
}

// FillRow is one historical fill (gateway GET .../fills).
type FillRow struct {
	Oid      uint64
	SymbolID uint32
	Px       int64
	Qty      int64
	Side     int32
	Fee      int64
	IsMaker  bool
	TsNs     uint64
}

// FundingRow is one funding settlement (gateway GET .../funding).
type FundingRow struct {
	SymbolID uint32
	Amount   int64
	RateBps  int32
	TsNs     uint64
}

// FreezeRow is one support-imposed order freeze (gateway GET .../freezes,
// POST .../freezes/upsert|delete) — a tracked flag + reason/ticket, distinct
// from rsx-risk's own margin-reservation bookkeeping (spec 7 explicitly
// defers "direct frozen-margin set").
type FreezeRow struct {
	UserID    uint32
	OrderID   uint64
	Reason    string
	TicketID  string
	CreatedAt string
}

// RolesClient is rsx-auth's identity + RBAC surface. Real — see
// HTTPRolesClient in http.go.
type RolesClient interface {
	// Enabled reports whether the client will dial (safe on a nil receiver).
	Enabled() bool
	// Me returns the caller's own management role ("" = none).
	Me() (role string, err error)
	ListUsers(query string, limit, cursor int) ([]UserRow, error)
	UserDetail(userID uint32) (UserRow, error)
	SetRole(userID uint32, role, reason, ticketID string) (string, error)
}

// AccountClient is the gateway's spec-7 account/wallet data surface.
// STUB/FIXTURE until the gateway ships it (see http.go and fixture.go).
type AccountClient interface {
	Enabled() bool
	Account(userID uint32) (AccountSummary, error)
	Positions(userID uint32) ([]PositionRow, error)
	Fills(userID uint32, symbolID uint32, limit int) ([]FillRow, error)
	Funding(userID uint32, symbolID uint32, limit int) ([]FundingRow, error)
	Freezes(userID uint32) ([]FreezeRow, error)
	AdjustCollateral(userID uint32, delta int64, reason, ticketID string) (AccountSummary, error)
	FreezeUpsert(userID uint32, orderID uint64, reason, ticketID string) (FreezeRow, error)
	FreezeDelete(userID uint32, orderID uint64, reason, ticketID string) error
	AddNote(userID uint32, note, reason, ticketID string) error
}

// OfflineRoles is the zero-value RolesClient — mirrors news.Off / a nil
// assistant.Client: Enabled() is false and every call is an honest
// "not configured" error, never a fabricated user or role.
type OfflineRoles struct{}

func (OfflineRoles) Enabled() bool { return false }

func (OfflineRoles) Me() (string, error) { return "", errOffline }

func (OfflineRoles) ListUsers(query string, limit, cursor int) ([]UserRow, error) {
	return nil, errOffline
}

func (OfflineRoles) UserDetail(userID uint32) (UserRow, error) {
	return UserRow{}, errOffline
}

func (OfflineRoles) SetRole(userID uint32, role, reason, ticketID string) (string, error) {
	return "", errOffline
}

// OfflineAccounts is the zero-value AccountClient (same rationale).
type OfflineAccounts struct{}

func (OfflineAccounts) Enabled() bool { return false }

func (OfflineAccounts) Account(userID uint32) (AccountSummary, error) {
	return AccountSummary{}, errOffline
}

func (OfflineAccounts) Positions(userID uint32) ([]PositionRow, error) {
	return nil, errOffline
}

func (OfflineAccounts) Fills(userID uint32, symbolID uint32, limit int) ([]FillRow, error) {
	return nil, errOffline
}

func (OfflineAccounts) Funding(userID uint32, symbolID uint32, limit int) ([]FundingRow, error) {
	return nil, errOffline
}

func (OfflineAccounts) Freezes(userID uint32) ([]FreezeRow, error) {
	return nil, errOffline
}

func (OfflineAccounts) AdjustCollateral(userID uint32, delta int64, reason, ticketID string) (AccountSummary, error) {
	return AccountSummary{}, errOffline
}

func (OfflineAccounts) FreezeUpsert(userID uint32, orderID uint64, reason, ticketID string) (FreezeRow, error) {
	return FreezeRow{}, errOffline
}

func (OfflineAccounts) FreezeDelete(userID uint32, orderID uint64, reason, ticketID string) error {
	return errOffline
}

func (OfflineAccounts) AddNote(userID uint32, note, reason, ticketID string) error {
	return errOffline
}
