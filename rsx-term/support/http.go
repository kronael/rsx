package support

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"
)

// fetchTimeout bounds every support call: long enough for a local service,
// short enough that a dead endpoint doesn't hang the terminal (mirrors
// conn.fetchTimeout).
const fetchTimeout = 3 * time.Second

// httpDo marshals body (nil for none), sends the request with a bearer token,
// and decodes the response into out (nil to discard the body). A non-2xx
// status is a plain error carrying the response body, never swallowed.
func httpDo(client *http.Client, method, url, token string, body, out any) error {
	var reader io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return fmt.Errorf("support: marshal %s %s: %w", method, url, err)
		}
		reader = bytes.NewReader(b)
	}
	req, err := http.NewRequest(method, url, reader)
	if err != nil {
		return fmt.Errorf("support: build %s %s: %w", method, url, err)
	}
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("support: %s %s: %w", method, url, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 300 {
		msg, _ := io.ReadAll(resp.Body)
		return fmt.Errorf("support: %s %s: status %d: %s", method, url, resp.StatusCode, strings.TrimSpace(string(msg)))
	}
	if out == nil {
		return nil
	}
	return json.NewDecoder(resp.Body).Decode(out)
}

// ---------------------------------------------------------------------
// RolesClient — real. Calls rsx-auth's finished /roles/* surface (see
// rsx-auth/src/rsx_auth/app.py).
// ---------------------------------------------------------------------

// HTTPRolesClient calls rsx-auth's identity + RBAC endpoints. token is an
// ALREADY-ISSUED rsx-auth session JWT (RSX_TERM_SUPPORT_TOKEN — obtained via
// the normal GitHub OAuth login, e.g. by an ops person copying their
// rsx_token cookie once). The terminal carries this token; it never mints
// its own admin-role token the way conn/jwt.go mints a trading token for the
// gateway — a self-minted token could assert any role via the shared HMAC
// secret, and this surface must never trust a client-supplied identity.
type HTTPRolesClient struct {
	baseURL string
	token   string
	client  *http.Client
}

// NewHTTPRolesClient builds a client against rsx-auth's base URL
// (e.g. http://127.0.0.1:8082). Does no I/O.
func NewHTTPRolesClient(baseURL, token string) *HTTPRolesClient {
	return &HTTPRolesClient{
		baseURL: strings.TrimRight(baseURL, "/"),
		token:   token,
		client:  &http.Client{Timeout: fetchTimeout},
	}
}

// Enabled is safe on a nil receiver (the offline default before a URL/token
// is configured).
func (c *HTTPRolesClient) Enabled() bool {
	return c != nil && c.baseURL != "" && c.token != ""
}

type roleWire struct {
	UserID      uint32  `json:"user_id"`
	Login       string  `json:"login"`
	Email       string  `json:"email"`
	Role        *string `json:"role"`
	CreatedAt   string  `json:"created_at"`
	LastLoginAt string  `json:"last_login_at"`
}

func (w roleWire) toRow() UserRow {
	role := ""
	if w.Role != nil {
		role = *w.Role
	}
	return UserRow{
		UserID: w.UserID, Login: w.Login, Email: w.Email, Role: role,
		CreatedAt: w.CreatedAt, LastLoginAt: w.LastLoginAt,
	}
}

func (c *HTTPRolesClient) Me() (string, error) {
	if !c.Enabled() {
		return "", errOffline
	}
	var out struct {
		Role *string `json:"role"`
	}
	if err := httpDo(c.client, http.MethodGet, c.baseURL+"/roles/me", c.token, nil, &out); err != nil {
		return "", err
	}
	if out.Role == nil {
		return "", nil
	}
	return *out.Role, nil
}

func (c *HTTPRolesClient) ListUsers(query string, limit, cursor int) ([]UserRow, error) {
	if !c.Enabled() {
		return nil, errOffline
	}
	q := url.Values{}
	if query != "" {
		q.Set("query", query)
	}
	if limit > 0 {
		q.Set("limit", strconv.Itoa(limit))
	}
	if cursor > 0 {
		q.Set("cursor", strconv.Itoa(cursor))
	}
	u := c.baseURL + "/roles/users"
	if enc := q.Encode(); enc != "" {
		u += "?" + enc
	}
	var out struct {
		Users []roleWire `json:"users"`
	}
	if err := httpDo(c.client, http.MethodGet, u, c.token, nil, &out); err != nil {
		return nil, err
	}
	rows := make([]UserRow, 0, len(out.Users))
	for _, w := range out.Users {
		rows = append(rows, w.toRow())
	}
	return rows, nil
}

func (c *HTTPRolesClient) UserDetail(userID uint32) (UserRow, error) {
	if !c.Enabled() {
		return UserRow{}, errOffline
	}
	var out roleWire
	u := fmt.Sprintf("%s/roles/users/%d", c.baseURL, userID)
	if err := httpDo(c.client, http.MethodGet, u, c.token, nil, &out); err != nil {
		return UserRow{}, err
	}
	return out.toRow(), nil
}

// setRoleReq mirrors rsx_auth.app.SetRoleReq.
type setRoleReq struct {
	Role     string `json:"role"`
	Reason   string `json:"reason"`
	TicketID string `json:"ticket_id"`
}

func (c *HTTPRolesClient) SetRole(userID uint32, role, reason, ticketID string) (string, error) {
	if !c.Enabled() {
		return "", errOffline
	}
	req, err := http.NewRequest(http.MethodPost,
		fmt.Sprintf("%s/roles/users/%d/role", c.baseURL, userID), nil)
	if err != nil {
		return "", err
	}
	body, err := json.Marshal(setRoleReq{Role: role, Reason: reason, TicketID: ticketID})
	if err != nil {
		return "", err
	}
	req.Body = io.NopCloser(bytes.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.token)
	// Idempotency-Key: one per logical action, not per retry — a fresh
	// mint here means a client-side retry of THIS call is not idempotent
	// yet (out of scope for v1: the terminal doesn't auto-retry writes).
	req.Header.Set("Idempotency-Key", fmt.Sprintf("rsx-term-role-%d-%d", userID, time.Now().UnixNano()))
	resp, err := c.client.Do(req)
	if err != nil {
		return "", fmt.Errorf("support: set role: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 300 {
		msg, _ := io.ReadAll(resp.Body)
		return "", fmt.Errorf("support: set role: status %d: %s", resp.StatusCode, strings.TrimSpace(string(msg)))
	}
	var out struct {
		Role string `json:"role"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		return "", err
	}
	return out.Role, nil
}

// ---------------------------------------------------------------------
// AccountClient — STUB. `/v1/api/support/*` is not available. Use
// FixtureAccountClient for demos, tests, and goldens.
//
// Proposed contract (mirrors specs/2/7-dashboard.md §3, adapted to a
// target user_id path param and the repo's `symbol_id`/`px`/`qty`
// naming law):
//
//	GET  /v1/api/support/users/{user_id}/account
//	     -> {"collateral":i64,"equity":i64,"unrealized_pnl":i64,
//	         "initial_margin":i64,"maint_margin":i64,"available":i64}
//	GET  /v1/api/support/users/{user_id}/positions
//	     -> {"positions":[{"symbol_id":u32,"side":i32,"qty":i64,
//	         "entry_px":i64,"mark_px":i64,"unrealized_pnl":i64,"liq_px":i64}]}
//	GET  /v1/api/support/users/{user_id}/fills?symbol_id=&limit=&before=
//	     -> {"fills":[{"oid":u64,"symbol_id":u32,"px":i64,"qty":i64,
//	         "side":i32,"fee":i64,"is_maker":bool,"ts_ns":u64}]}
//	GET  /v1/api/support/users/{user_id}/funding?symbol_id=&limit=&before=
//	     -> {"funding":[{"symbol_id":u32,"amount":i64,"rate_bps":i32,"ts_ns":u64}]}
//	GET  /v1/api/support/users/{user_id}/freezes
//	     -> {"freezes":[{"user_id":u32,"order_id":u64,"reason":string,
//	         "ticket_id":string,"created_at":string}]}
//	POST /v1/api/support/accounts/{user_id}/adjust-collateral
//	     {"delta":i64,"reason":string,"ticket_id":string}
//	     + X-Request-Id, Idempotency-Key headers -> the Account shape above
//	POST /v1/api/support/freezes/upsert
//	     {"user_id":u32,"order_id":u64,"reason":string,"ticket_id":string}
//	     -> one freezes row
//	POST /v1/api/support/freezes/delete
//	     {"user_id":u32,"order_id":u64,"reason":string,"ticket_id":string}
//	     -> {"ok":true}
//	POST /v1/api/support/notes
//	     {"user_id":u32,"note":string,"reason":string,"ticket_id":string}
//	     -> {"ok":true}
//
// Auth: Bearer <rsx-auth JWT>, same token as HTTPRolesClient. The
// gateway has no DB access to rsx-auth's role table, so it trusts the
// JWT's `role` claim (jwt_util.issue now embeds it) rather than calling
// back to rsx-auth per request.
type HTTPAccountClient struct {
	baseURL string
	token   string
	client  *http.Client
}

// NewHTTPAccountClient builds a client against the future gateway support
// base URL. Does no I/O; every call fails cleanly until the gateway exists.
func NewHTTPAccountClient(baseURL, token string) *HTTPAccountClient {
	return &HTTPAccountClient{
		baseURL: strings.TrimRight(baseURL, "/"),
		token:   token,
		client:  &http.Client{Timeout: fetchTimeout},
	}
}

func (c *HTTPAccountClient) Enabled() bool {
	return c != nil && c.baseURL != "" && c.token != ""
}

func (c *HTTPAccountClient) Account(userID uint32) (AccountSummary, error) {
	if !c.Enabled() {
		return AccountSummary{}, errOffline
	}
	var out AccountSummary
	u := fmt.Sprintf("%s/v1/api/support/users/%d/account", c.baseURL, userID)
	err := httpDo(c.client, http.MethodGet, u, c.token, nil, &out)
	return out, err
}

func (c *HTTPAccountClient) Positions(userID uint32) ([]PositionRow, error) {
	if !c.Enabled() {
		return nil, errOffline
	}
	var out struct {
		Positions []PositionRow `json:"positions"`
	}
	u := fmt.Sprintf("%s/v1/api/support/users/%d/positions", c.baseURL, userID)
	err := httpDo(c.client, http.MethodGet, u, c.token, nil, &out)
	return out.Positions, err
}

func (c *HTTPAccountClient) Fills(userID uint32, symbolID uint32, limit int) ([]FillRow, error) {
	if !c.Enabled() {
		return nil, errOffline
	}
	q := url.Values{}
	if symbolID != 0 {
		q.Set("symbol_id", strconv.FormatUint(uint64(symbolID), 10))
	}
	if limit > 0 {
		q.Set("limit", strconv.Itoa(limit))
	}
	u := fmt.Sprintf("%s/v1/api/support/users/%d/fills", c.baseURL, userID)
	if enc := q.Encode(); enc != "" {
		u += "?" + enc
	}
	var out struct {
		Fills []FillRow `json:"fills"`
	}
	err := httpDo(c.client, http.MethodGet, u, c.token, nil, &out)
	return out.Fills, err
}

func (c *HTTPAccountClient) Funding(userID uint32, symbolID uint32, limit int) ([]FundingRow, error) {
	if !c.Enabled() {
		return nil, errOffline
	}
	q := url.Values{}
	if symbolID != 0 {
		q.Set("symbol_id", strconv.FormatUint(uint64(symbolID), 10))
	}
	if limit > 0 {
		q.Set("limit", strconv.Itoa(limit))
	}
	u := fmt.Sprintf("%s/v1/api/support/users/%d/funding", c.baseURL, userID)
	if enc := q.Encode(); enc != "" {
		u += "?" + enc
	}
	var out struct {
		Funding []FundingRow `json:"funding"`
	}
	err := httpDo(c.client, http.MethodGet, u, c.token, nil, &out)
	return out.Funding, err
}

func (c *HTTPAccountClient) Freezes(userID uint32) ([]FreezeRow, error) {
	if !c.Enabled() {
		return nil, errOffline
	}
	var out struct {
		Freezes []FreezeRow `json:"freezes"`
	}
	u := fmt.Sprintf("%s/v1/api/support/users/%d/freezes", c.baseURL, userID)
	err := httpDo(c.client, http.MethodGet, u, c.token, nil, &out)
	return out.Freezes, err
}

type supportWriteReq struct {
	Delta    int64  `json:"delta,omitempty"`
	UserID   uint32 `json:"user_id,omitempty"`
	OrderID  uint64 `json:"order_id,omitempty"`
	Note     string `json:"note,omitempty"`
	Reason   string `json:"reason"`
	TicketID string `json:"ticket_id"`
}

func (c *HTTPAccountClient) AdjustCollateral(userID uint32, delta int64, reason, ticketID string) (AccountSummary, error) {
	if !c.Enabled() {
		return AccountSummary{}, errOffline
	}
	var out AccountSummary
	u := fmt.Sprintf("%s/v1/api/support/accounts/%d/adjust-collateral", c.baseURL, userID)
	err := httpDo(c.client, http.MethodPost, u, c.token,
		supportWriteReq{Delta: delta, Reason: reason, TicketID: ticketID}, &out)
	return out, err
}

func (c *HTTPAccountClient) FreezeUpsert(userID uint32, orderID uint64, reason, ticketID string) (FreezeRow, error) {
	if !c.Enabled() {
		return FreezeRow{}, errOffline
	}
	var out FreezeRow
	u := c.baseURL + "/v1/api/support/freezes/upsert"
	err := httpDo(c.client, http.MethodPost, u, c.token,
		supportWriteReq{UserID: userID, OrderID: orderID, Reason: reason, TicketID: ticketID}, &out)
	return out, err
}

func (c *HTTPAccountClient) FreezeDelete(userID uint32, orderID uint64, reason, ticketID string) error {
	if !c.Enabled() {
		return errOffline
	}
	u := c.baseURL + "/v1/api/support/freezes/delete"
	return httpDo(c.client, http.MethodPost, u, c.token,
		supportWriteReq{UserID: userID, OrderID: orderID, Reason: reason, TicketID: ticketID}, nil)
}

func (c *HTTPAccountClient) AddNote(userID uint32, note, reason, ticketID string) error {
	if !c.Enabled() {
		return errOffline
	}
	u := c.baseURL + "/v1/api/support/notes"
	return httpDo(c.client, http.MethodPost, u, c.token,
		supportWriteReq{UserID: userID, Note: note, Reason: reason, TicketID: ticketID}, nil)
}
