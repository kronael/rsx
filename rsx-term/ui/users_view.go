package ui

import (
	"fmt"
	"strconv"
	"strings"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/support"
)

// The USERS screen: rsx-auth's identity+role directory (real — list/detail/
// set-role) and the gateway's spec-7 account/wallet data (STUB/fixture until
// the gateway ships it; see rsx-term/support/http.go). List → detail → a
// single-line action prompt, the terminal's existing minimal-widget idiom
// (mirrors the news search buffer / assistant input line) rather than a form
// widget. Every write goes through the real client and shows the real
// response — success or an honest error, never assumed.

// userDetail is the drilled-into-one-user state: the target row plus every
// fetched slice (nil until its fetch lands), and any fetch error.
type userDetail struct {
	user      support.UserRow
	loading   bool
	err       string
	account   support.AccountSummary
	positions []support.PositionRow
	fills     []support.FillRow
	funding   []support.FundingRow
	freezes   []support.FreezeRow
}

// promptKind is which single-line action prompt is open on the detail view.
type promptKind int

const (
	promptAdjustCollateral promptKind = iota
	promptFreezeUpsert
	promptFreezeDelete
	promptSetRole
	promptAddNote
)

// title is the prompt's own hint, shown where the status line normally sits.
func (k promptKind) title() string {
	switch k {
	case promptAdjustCollateral:
		return "adjust collateral — <delta> <ticket_id> <reason…>"
	case promptFreezeUpsert:
		return "freeze order — <order_id> <ticket_id> <reason…>"
	case promptFreezeDelete:
		return "unfreeze order — <order_id> <ticket_id> <reason…>"
	case promptSetRole:
		return "set role — <viewer|operator|admin|auditor> <ticket_id> <reason…>"
	default:
		return "add note — <ticket_id> <note…>"
	}
}

// usersPrompt is the open action prompt: one text buffer, enter submits, esc
// cancels — no multi-field form widget.
type usersPrompt struct {
	kind   promptKind
	target uint32
	input  string
}

// ------------------------------- messages -------------------------------

type usersLoadedMsg struct {
	users []support.UserRow
	err   error
}

type callerRoleLoadedMsg struct {
	role string
	err  error
}

type userDetailLoadedMsg struct {
	target    uint32
	user      support.UserRow
	account   support.AccountSummary
	positions []support.PositionRow
	fills     []support.FillRow
	funding   []support.FundingRow
	freezes   []support.FreezeRow
	err       error
}

type usersActionDoneMsg struct {
	target        uint32
	summary       string
	err           error
	refreshDetail bool
}

// -------------------------------- commands -------------------------------

func loadUsersCmd(roles support.RolesClient, query string) tea.Cmd {
	return func() tea.Msg {
		users, err := roles.ListUsers(query, 100, 0)
		return usersLoadedMsg{users: users, err: err}
	}
}

func loadCallerRoleCmd(roles support.RolesClient) tea.Cmd {
	return func() tea.Msg {
		role, err := roles.Me()
		return callerRoleLoadedMsg{role: role, err: err}
	}
}

// loadUserDetailCmd fetches identity+role (rsx-auth, real) and
// account/positions/fills/funding/freezes (gateway, stub/fixture) for one
// target. All five run in this one Cmd's goroutine — bubbletea already runs
// Cmds off the render loop, so there's no need for five separate messages.
func loadUserDetailCmd(roles support.RolesClient, accounts support.AccountClient, target uint32) tea.Cmd {
	return func() tea.Msg {
		user, uerr := roles.UserDetail(target)
		account, aerr := accounts.Account(target)
		positions, perr := accounts.Positions(target)
		fills, ferr := accounts.Fills(target, 0, 20)
		funding, fuerr := accounts.Funding(target, 0, 20)
		freezes, frerr := accounts.Freezes(target)
		return userDetailLoadedMsg{
			target: target, user: user, account: account,
			positions: positions, fills: fills, funding: funding, freezes: freezes,
			err: firstErr(uerr, aerr, perr, ferr, fuerr, frerr),
		}
	}
}

func firstErr(errs ...error) error {
	for _, e := range errs {
		if e != nil {
			return e
		}
	}
	return nil
}

func adjustCollateralCmd(accounts support.AccountClient, target uint32, delta int64, reason, ticket string) tea.Cmd {
	return func() tea.Msg {
		_, err := accounts.AdjustCollateral(target, delta, reason, ticket)
		return usersActionDoneMsg{target: target, summary: fmt.Sprintf("adjust-collateral %+d", delta), err: err, refreshDetail: err == nil}
	}
}

func freezeUpsertCmd(accounts support.AccountClient, target uint32, orderID uint64, reason, ticket string) tea.Cmd {
	return func() tea.Msg {
		_, err := accounts.FreezeUpsert(target, orderID, reason, ticket)
		return usersActionDoneMsg{target: target, summary: fmt.Sprintf("freeze order %d", orderID), err: err, refreshDetail: err == nil}
	}
}

func freezeDeleteCmd(accounts support.AccountClient, target uint32, orderID uint64, reason, ticket string) tea.Cmd {
	return func() tea.Msg {
		err := accounts.FreezeDelete(target, orderID, reason, ticket)
		return usersActionDoneMsg{target: target, summary: fmt.Sprintf("unfreeze order %d", orderID), err: err, refreshDetail: err == nil}
	}
}

func setRoleCmd(roles support.RolesClient, target uint32, role, reason, ticket string) tea.Cmd {
	return func() tea.Msg {
		_, err := roles.SetRole(target, role, reason, ticket)
		return usersActionDoneMsg{target: target, summary: "set role " + role, err: err, refreshDetail: err == nil}
	}
}

func addNoteCmd(accounts support.AccountClient, target uint32, note, ticket string) tea.Cmd {
	return func() tea.Msg {
		err := accounts.AddNote(target, note, note, ticket)
		return usersActionDoneMsg{target: target, summary: "note added", err: err}
	}
}

// ---------------------------------- view ----------------------------------

// viewUsers renders the screen as a fixed grid, mirroring viewNews/viewLLM:
// header + mode line, then the list or the drilled-into detail, then status
// (or the open prompt) + hint.
func (m Model) viewUsers() string {
	lines := make([]string, 0, m.height)
	lines = append(lines, m.streamHeader(), m.modeLine())

	if m.usersDetail != nil {
		lines = append(lines, m.userDetailLines()...)
	} else {
		lines = append(lines, m.usersListLines(maxInt(m.height-6, 3))...)
	}

	for len(lines) < m.height-2 {
		lines = append(lines, "")
	}
	lines = append(lines, StyleTextBright.Render(" "+m.usersStatusLine()))
	lines = append(lines, m.hintLine())
	return strings.Join(lines, "\n")
}

// usersStatusLine shows the open prompt (if any) in place of the ordinary
// status line — the same "typing takes over the status area" idiom the
// assistant input uses.
func (m Model) usersStatusLine() string {
	if m.usersPrompt != nil {
		return m.usersPrompt.kind.title() + ": " + m.usersPrompt.input + "_"
	}
	return m.status
}

// usersListHeader carries the search buffer and the caller's own role (so
// the operator always sees what they're allowed to do before they try).
func (m Model) usersListHeader() string {
	title := " ── users "
	if m.usersSearch || m.usersQuery != "" {
		q := m.usersQuery
		if m.usersSearch {
			q += "_"
		}
		return StyleMuted.Render(title+"── search: ") + StyleTextBright.Bold(true).Render(q) + StyleMuted.Render(" ──")
	}
	you := "you: —"
	if m.callerRole != "" {
		you = "you: " + m.callerRole
	} else if !m.roles.Enabled() {
		you = "you: — (rsx-auth not configured)"
	}
	return StyleMuted.Render(title + "── / search · j/k select · enter detail · g refresh · " + you + " ──")
}

// usersListLines renders the identity+role directory. rows bounds how many
// user rows fit before the "N more" marker.
func (m Model) usersListLines(rows int) []string {
	out := []string{m.usersListHeader()}
	if !m.roles.Enabled() {
		out = append(out, StyleMuted.Render("  rsx-auth not configured — set RSX_TERM_AUTH_URL + RSX_TERM_SUPPORT_TOKEN"))
		return out
	}
	if m.usersLoading {
		out = append(out, StyleMuted.Render("  loading…"))
		return out
	}
	if m.usersErr != "" {
		out = append(out, StyleAsk.Render("  error: "+m.usersErr))
		return out
	}
	filtered := m.filteredUsers()
	if len(filtered) == 0 {
		out = append(out, StyleMuted.Render("  no users"))
		return out
	}
	out = append(out, StyleMuted.Render(fmt.Sprintf("  %-6s %-20s %-24s %-10s", "id", "login", "email", "role")))
	sel := clamp(m.usersSel, 0, len(filtered)-1)
	for i, u := range filtered {
		if rows > 2 && i >= rows-2 {
			out = append(out, StyleMuted.Render(fmt.Sprintf("  … %d more", len(filtered)-i)))
			break
		}
		cursor := "  "
		if i == sel {
			cursor = StyleAccent.Render("▸ ")
		}
		role := u.Role
		if role == "" {
			role = "—"
		}
		row := fmt.Sprintf("%-6d %-20s %-24s %-10s", u.UserID, truncateStr(u.Login, 20), truncateStr(u.Email, 24), role)
		out = append(out, cursor+StyleText.Render(row))
	}
	return out
}

// filteredUsers applies the search query (client-side substring, mirroring
// news' filteredNews) over the already-fetched page.
func (m Model) filteredUsers() []support.UserRow {
	if m.usersQuery == "" {
		return m.users
	}
	q := strings.ToLower(m.usersQuery)
	var out []support.UserRow
	for _, u := range m.users {
		if strings.Contains(strings.ToLower(u.Login), q) || strings.Contains(strings.ToLower(u.Email), q) {
			out = append(out, u)
		}
	}
	return out
}

// userDetailLines renders the drilled-into-one-user view: identity+role
// (real), then account/positions/fills/funding/freezes — dashed with an
// honest note while the gateway's support surface is a stub. Raw i64 units,
// not a display decimal: an account can span many symbols with different
// price/qty precision, so guessing one would risk fabricating a number.
func (m Model) userDetailLines() []string {
	d := m.usersDetail
	out := []string{StyleHeading.Bold(true).Render(fmt.Sprintf("  user %d — %s", d.user.UserID, orDash(d.user.Login)))}
	out = append(out, StyleMuted.Render("  role ")+StyleText.Render(orDash(d.user.Role))+StyleMuted.Render("   email ")+StyleText.Render(orDash(d.user.Email)))
	if d.loading {
		out = append(out, StyleMuted.Render("  loading account/positions/fills/funding/freezes…"))
		return out
	}
	if d.err != "" {
		out = append(out, StyleAsk.Render("  error: "+d.err))
	}
	if !m.accounts.Enabled() {
		out = append(out, StyleDerived.Render("  ~ account/positions/fills/funding/freezes: gateway support surface not shipped yet (stub) —"))
		out = append(out, StyleDerived.Render("    see .ship/48-USER-WALLET-MGMT/PLAN.md for the exact contract"))
	} else {
		out = append(out, m.accountDetailLines(d)...)
	}
	out = append(out, "", StyleMuted.Render("  o set-role · a adjust-collateral · z freeze · Z unfreeze · n note · esc back"))
	return out
}

// accountDetailLines renders the raw account/positions/fills/funding/freezes
// once an AccountClient (real or fixture) is wired.
func (m Model) accountDetailLines(d *userDetail) []string {
	a := d.account
	var out []string
	out = append(out, StyleMuted.Render("  ── account (raw units) ──────────────────"))
	out = append(out, StyleText.Render(fmt.Sprintf("  collateral %d  equity %d  uPnL %d  avail %d",
		a.Collateral, a.Equity, a.UnrealizedPnl, a.Available)))
	out = append(out, StyleMuted.Render(fmt.Sprintf("  ── positions (%d) ──────────────────────", len(d.positions))))
	for _, p := range d.positions {
		side := "long"
		if p.Side == 1 {
			side = "short"
		}
		out = append(out, StyleText.Render(fmt.Sprintf("  sym %d  %s qty %d @ %d  uPnL %d", p.SymbolID, side, p.Qty, p.EntryPx, p.UnrealizedPnl)))
	}
	out = append(out, StyleMuted.Render(fmt.Sprintf("  ── freezes (%d) ─────────────────────────", len(d.freezes))))
	for _, f := range d.freezes {
		out = append(out, StyleAsk.Render(fmt.Sprintf("  order %d — %s (%s)", f.OrderID, f.Reason, f.TicketID)))
	}
	return out
}

func orDash(s string) string {
	if s == "" {
		return "—"
	}
	return s
}

func truncateStr(s string, n int) string {
	if len(s) <= n {
		return s
	}
	if n <= 1 {
		return s[:n]
	}
	return s[:n-1] + "…"
}

// -------------------------------- handlers --------------------------------

// maybeLoadUsers fires the list + caller-role fetch the first time the
// screen is entered (usersLoading guards a duplicate fire mid-flight; a
// non-empty users/usersErr means it already ran once — 'g' re-fires
// explicitly). Returns nil (no-op) once anything has landed.
func (m *Model) maybeLoadUsers() tea.Cmd {
	if m.screen != screenUsers || m.usersLoading || len(m.users) > 0 || m.usersErr != "" {
		return nil
	}
	m.usersLoading = true
	return tea.Batch(loadUsersCmd(m.roles, ""), loadCallerRoleCmd(m.roles))
}

// handleUsersKey is the users screen's grammar. List mode: / search, j/k
// select, enter drills in, g refreshes. Detail mode: o/a/z/Z/n open an
// action prompt (role-gated server-side, not client-side — a denied action
// surfaces the server's real 403, never a locally-guessed permission),
// esc backs out one layer at a time (prompt → detail → list → book).
func (m Model) handleUsersKey(act action, key string) (tea.Model, tea.Cmd) {
	switch act {
	case actUsersBack:
		if m.usersDetail != nil {
			m.usersDetail = nil
			return m, nil
		}
		if m.usersQuery != "" {
			m.usersQuery = ""
			return m, nil
		}
		m.screen = screenBook
		return m, nil
	case actUsersSearch:
		if m.usersDetail == nil {
			m.usersSearch = true
		}
		return m, nil
	case actUsersDown:
		if m.usersDetail == nil {
			m.usersSel = m.clampUsersSel(m.usersSel + 1)
		}
		return m, nil
	case actUsersUp:
		if m.usersDetail == nil {
			m.usersSel = m.clampUsersSel(m.usersSel - 1)
		}
		return m, nil
	case actUsersOpen:
		return m.openUserDetail()
	case actUsersRefresh:
		return m.refreshUsers()
	case actUsersAdjustCollateral:
		return m.openPrompt(promptAdjustCollateral)
	case actUsersFreezeUpsert:
		return m.openPrompt(promptFreezeUpsert)
	case actUsersFreezeDelete:
		return m.openPrompt(promptFreezeDelete)
	case actUsersSetRole:
		return m.openPrompt(promptSetRole)
	case actUsersAddNote:
		return m.openPrompt(promptAddNote)
	}
	return m, nil
}

func (m Model) clampUsersSel(i int) int {
	n := len(m.filteredUsers())
	if n == 0 {
		return 0
	}
	return clamp(i, 0, n-1)
}

// openUserDetail drills into the selected row (list mode only).
func (m Model) openUserDetail() (tea.Model, tea.Cmd) {
	filtered := m.filteredUsers()
	if m.usersDetail != nil || len(filtered) == 0 {
		return m, nil
	}
	u := filtered[clamp(m.usersSel, 0, len(filtered)-1)]
	m.usersDetail = &userDetail{user: u, loading: true}
	return m, loadUserDetailCmd(m.roles, m.accounts, u.UserID)
}

// refreshUsers re-fires the list (list mode) or the current detail (detail
// mode) — 'g', bypassing maybeLoadUsers' once-only guard.
func (m Model) refreshUsers() (tea.Model, tea.Cmd) {
	if m.usersDetail != nil {
		m.usersDetail.loading = true
		return m, loadUserDetailCmd(m.roles, m.accounts, m.usersDetail.user.UserID)
	}
	m.usersLoading = true
	return m, tea.Batch(loadUsersCmd(m.roles, m.usersQuery), loadCallerRoleCmd(m.roles))
}

// openPrompt opens an action prompt (detail mode only — nothing to act on
// from the list).
func (m Model) openPrompt(kind promptKind) (tea.Model, tea.Cmd) {
	if m.usersDetail == nil {
		return m, nil
	}
	m.usersPrompt = &usersPrompt{kind: kind, target: m.usersDetail.user.UserID}
	return m, nil
}

// handleUsersSearchKey types into the list search buffer (mirrors
// handleNewsSearchKey).
func (m Model) handleUsersSearchKey(key string) (tea.Model, tea.Cmd) {
	switch {
	case key == "esc":
		m.usersSearch = false
		m.usersQuery = ""
	case key == "enter":
		m.usersSearch = false
	case key == "backspace":
		if len(m.usersQuery) > 0 {
			m.usersQuery = m.usersQuery[:len(m.usersQuery)-1]
		}
	case len(key) == 1 && key[0] >= ' ' && key[0] <= '~':
		if len(m.usersQuery) < 48 {
			m.usersQuery += key
			m.usersSel = 0
		}
	}
	return m, nil
}

// handleUsersPromptKey types into the open action prompt; enter parses +
// submits, esc cancels without sending anything.
func (m Model) handleUsersPromptKey(key string) (tea.Model, tea.Cmd) {
	p := m.usersPrompt
	switch {
	case key == "esc":
		m.usersPrompt = nil
		return m, nil
	case key == "enter":
		return m.submitUsersPrompt()
	case key == "backspace":
		if len(p.input) > 0 {
			p.input = p.input[:len(p.input)-1]
		}
	case len(key) == 1 && key[0] >= ' ' && key[0] <= '~':
		if len(p.input) < 200 {
			p.input += key
		}
	}
	return m, nil
}

// submitUsersPrompt parses the prompt's single line per its kind and fires
// the matching Cmd. A malformed line is rejected locally with an honest
// status message and the prompt STAYS OPEN (so a typo doesn't lose the rest
// of the input) — it never guesses a default for a missing field.
func (m Model) submitUsersPrompt() (tea.Model, tea.Cmd) {
	p := m.usersPrompt
	fields := strings.Fields(p.input)
	switch p.kind {
	case promptAdjustCollateral, promptFreezeUpsert, promptFreezeDelete:
		if len(fields) < 3 {
			m.status = "need: <amount> <ticket_id> <reason…>"
			return m, nil
		}
		ticket, reason := fields[1], strings.Join(fields[2:], " ")
		if p.kind == promptAdjustCollateral {
			delta, err := strconv.ParseInt(fields[0], 10, 64)
			if err != nil {
				m.status = "delta must be an integer: " + fields[0]
				return m, nil
			}
			m.usersPrompt = nil
			return m, adjustCollateralCmd(m.accounts, p.target, delta, reason, ticket)
		}
		orderID, err := strconv.ParseUint(fields[0], 10, 64)
		if err != nil {
			m.status = "order_id must be an integer: " + fields[0]
			return m, nil
		}
		m.usersPrompt = nil
		if p.kind == promptFreezeUpsert {
			return m, freezeUpsertCmd(m.accounts, p.target, orderID, reason, ticket)
		}
		return m, freezeDeleteCmd(m.accounts, p.target, orderID, reason, ticket)
	case promptSetRole:
		if len(fields) < 3 {
			m.status = "need: <role> <ticket_id> <reason…>"
			return m, nil
		}
		role := fields[0]
		valid := false
		for _, r := range []string{"viewer", "operator", "admin", "auditor"} {
			if role == r {
				valid = true
			}
		}
		if !valid {
			m.status = "role must be one of viewer/operator/admin/auditor, got " + role
			return m, nil
		}
		m.usersPrompt = nil
		return m, setRoleCmd(m.roles, p.target, role, strings.Join(fields[2:], " "), fields[1])
	default: // promptAddNote
		if len(fields) < 2 {
			m.status = "need: <ticket_id> <note…>"
			return m, nil
		}
		m.usersPrompt = nil
		return m, addNoteCmd(m.accounts, p.target, strings.Join(fields[1:], " "), fields[0])
	}
}
