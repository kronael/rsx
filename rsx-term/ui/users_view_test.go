package ui

import (
	"fmt"
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/conn"
	"rsx-term/support"
)

// fakeRoles is a fixed in-memory RolesClient for view tests — no network,
// mirrors fakeNews' role in news_view_test.go. userID 1/2 line up with
// support.NewFixtureAccounts' seeded accounts so detail view tests exercise
// both clients together.
type fakeRoles struct {
	me    string
	users []support.UserRow
	set   []struct {
		userID          uint32
		role, reason, t string
	}
}

func (f *fakeRoles) Enabled() bool { return true }
func (f *fakeRoles) Me() (string, error) {
	return f.me, nil
}
func (f *fakeRoles) ListUsers(query string, limit, cursor int) ([]support.UserRow, error) {
	if query == "" {
		return f.users, nil
	}
	var out []support.UserRow
	for _, u := range f.users {
		if strings.Contains(strings.ToLower(u.Login), strings.ToLower(query)) {
			out = append(out, u)
		}
	}
	return out, nil
}
func (f *fakeRoles) UserDetail(userID uint32) (support.UserRow, error) {
	for _, u := range f.users {
		if u.UserID == userID {
			return u, nil
		}
	}
	return support.UserRow{}, fmt.Errorf("no such user %d", userID)
}
func (f *fakeRoles) SetRole(userID uint32, role, reason, ticketID string) (string, error) {
	f.set = append(f.set, struct {
		userID          uint32
		role, reason, t string
	}{userID, role, reason, ticketID})
	for i, u := range f.users {
		if u.UserID == userID {
			f.users[i].Role = role
		}
	}
	return role, nil
}

func defaultFakeRoles() *fakeRoles {
	return &fakeRoles{
		me: "admin",
		users: []support.UserRow{
			{UserID: 1, Login: "alice", Email: "alice@rsx.dev", Role: "admin"},
			{UserID: 2, Login: "bob", Email: "bob@rsx.dev", Role: ""},
		},
	}
}

// runCmd executes cmd (and any further Cmd its result folds back in) —
// unlike the shared press/apply helpers (update_test.go), which deliberately
// leave a returned Cmd un-run (right for binTickCmd's scheduler and the
// assistant's externally-pumped channel). The USERS screen's Cmds are plain
// synchronous request/response (an HTTP call in its own goroutine per
// bubbletea's contract), so running them inline is the correct simulation of
// "wait for the reply" in a test. tea.Batch's Cmd returns a tea.BatchMsg — a
// []tea.Cmd, not a foldable message — so that case recurses instead of
// reaching Update.
func runCmd(m Model, cmd tea.Cmd) Model {
	if cmd == nil {
		return m
	}
	msg := cmd()
	if batch, ok := msg.(tea.BatchMsg); ok {
		for _, c := range batch {
			m = runCmd(m, c)
		}
		return m
	}
	updated, next := m.Update(msg)
	m = updated.(Model)
	return runCmd(m, next)
}

// pressRun is press, but runs any Cmd the keypress returns (see runCmd) —
// every USERS-screen test uses this instead of press.
func pressRun(m Model, key string) Model {
	updated, cmd := m.Update(keyMsg(key))
	return runCmd(updated.(Model), cmd)
}

// usersModel is a stream model on the USERS screen, wired to fakeRoles + a
// fixture AccountClient — mirrors newsModel/streamModel.
func usersModel(t *testing.T, roles support.RolesClient, accounts support.AccountClient) Model {
	t.Helper()
	m := New(Config{
		Symbol: "PENGU-PERP", SymbolID: 10, Sub: &conn.MockGateway{},
		PriceDec: 6, QtyDec: 4, Tick: 1, Stream: true,
		Roles: roles, Accounts: accounts,
	})
	m = apply(m, tea.WindowSizeMsg{Width: 100, Height: 32})
	m = pressRun(m, "tab") // book → news
	m = pressRun(m, "tab") // news → llm
	m = pressRun(m, "tab") // llm → users (fires the list + caller-role load)
	return m
}

func TestUsersScreenOfflineByDefault(t *testing.T) {
	m := usersModel(t, nil, nil)
	view := stripANSI(m.View())
	if !strings.Contains(view, "rsx-auth not configured") {
		t.Fatalf("an unconfigured Roles client must say so honestly:\n%s", view)
	}
	if m.screen != screenUsers {
		t.Fatalf("expected screenUsers, got %v", m.screen)
	}
}

func TestUsersListShowsFetchedUsers(t *testing.T) {
	m := usersModel(t, defaultFakeRoles(), support.NewFixtureAccounts())
	view := stripANSI(m.View())
	if !strings.Contains(view, "alice") || !strings.Contains(view, "bob") {
		t.Fatalf("list should show fetched users:\n%s", view)
	}
	if !strings.Contains(view, "you: admin") {
		t.Fatalf("list header should show the caller's own role:\n%s", view)
	}
}

func TestUsersSearchFiltersList(t *testing.T) {
	m := usersModel(t, defaultFakeRoles(), support.NewFixtureAccounts())
	m = pressRun(m, "/")
	for _, r := range "bob" {
		m = pressRun(m, string(r))
	}
	view := stripANSI(m.View())
	if strings.Contains(view, "alice") {
		t.Fatalf("search for 'bob' must filter out alice:\n%s", view)
	}
	if !strings.Contains(view, "bob") {
		t.Fatalf("search for 'bob' must still show bob:\n%s", view)
	}
}

func TestUsersDetailShowsAccountAndPositions(t *testing.T) {
	m := usersModel(t, defaultFakeRoles(), support.NewFixtureAccounts())
	m = pressRun(m, "enter") // drill into the selected (first) user — alice, user_id 1
	view := stripANSI(m.View())
	if !strings.Contains(view, "user 1") || !strings.Contains(view, "alice") {
		t.Fatalf("detail should show the target user:\n%s", view)
	}
	if !strings.Contains(view, "collateral 100000000") {
		t.Fatalf("detail should show the fixture account:\n%s", view)
	}
	if !strings.Contains(view, "sym 10") {
		t.Fatalf("detail should show the fixture position:\n%s", view)
	}
}

func TestUsersDetailHonestWhenAccountsOffline(t *testing.T) {
	m := usersModel(t, defaultFakeRoles(), nil)
	m = pressRun(m, "enter")
	view := stripANSI(m.View())
	if !strings.Contains(view, "not shipped yet") {
		t.Fatalf("an offline AccountClient must say so honestly, not fabricate zeros:\n%s", view)
	}
}

func TestUsersAdjustCollateralRoundTrip(t *testing.T) {
	accounts := support.NewFixtureAccounts()
	m := usersModel(t, defaultFakeRoles(), accounts)
	m = pressRun(m, "enter") // detail on alice (user 1)
	m = pressRun(m, "a")     // open the adjust-collateral prompt
	for _, r := range "500000 T-1 test-adjust" {
		m = pressRun(m, string(r))
	}
	m = pressRun(m, "enter")
	if !strings.Contains(m.status, "ok") {
		t.Fatalf("adjust-collateral should report ok, got status %q", m.status)
	}
	after, _ := accounts.Account(1)
	if after.Collateral != 100_000_000+500_000 {
		t.Fatalf("fixture collateral should reflect the adjustment: %+v", after)
	}
}

func TestUsersPromptRejectsMalformedInputAndStaysOpen(t *testing.T) {
	m := usersModel(t, defaultFakeRoles(), support.NewFixtureAccounts())
	m = pressRun(m, "enter")
	m = pressRun(m, "a")
	for _, r := range "notanumber T-1 reason" {
		m = pressRun(m, string(r))
	}
	m = pressRun(m, "enter")
	if m.usersPrompt == nil {
		t.Fatal("a malformed prompt submission must stay open, not silently discard the draft")
	}
	if !strings.Contains(m.status, "must be an integer") {
		t.Fatalf("expected an honest parse error in the status, got %q", m.status)
	}
}

func TestUsersEscBacksOutOneLayerAtATime(t *testing.T) {
	m := usersModel(t, defaultFakeRoles(), support.NewFixtureAccounts())
	m = pressRun(m, "enter")
	if m.usersDetail == nil {
		t.Fatal("enter should have opened detail")
	}
	m = pressRun(m, "esc")
	if m.usersDetail != nil {
		t.Fatal("esc should close detail back to the list")
	}
	if m.screen != screenUsers {
		t.Fatalf("esc from the list (no query) should exit to the book screen, got %v", m.screen)
	}
}
