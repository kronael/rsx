package support

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestOfflineDefaultsNeverDial(t *testing.T) {
	var roles OfflineRoles
	if roles.Enabled() {
		t.Fatal("OfflineRoles must report disabled")
	}
	if _, err := roles.Me(); err == nil {
		t.Fatal("offline Me() must error, never fabricate a role")
	}
	var accounts OfflineAccounts
	if accounts.Enabled() {
		t.Fatal("OfflineAccounts must report disabled")
	}
	if _, err := accounts.Account(1); err == nil {
		t.Fatal("offline Account() must error, never fabricate a balance")
	}
}

func TestHTTPRolesClientDisabledWithoutTokenOrURL(t *testing.T) {
	c := NewHTTPRolesClient("", "")
	if c.Enabled() {
		t.Fatal("no baseURL/token should be disabled")
	}
	c = NewHTTPRolesClient("http://x", "")
	if c.Enabled() {
		t.Fatal("no token should be disabled")
	}
	var nilClient *HTTPRolesClient
	if nilClient.Enabled() {
		t.Fatal("nil receiver must report disabled, not panic")
	}
}

// fakeRolesServer replays rsx-auth's /roles/* response shapes exactly
// (app.py's _serialize_user_row / roles_me / roles_set_role), so the client
// is exercised against the real wire contract, not a hand-waved shape.
func fakeRolesServer(t *testing.T) *httptest.Server {
	t.Helper()
	mux := http.NewServeMux()
	mux.HandleFunc("/roles/me", func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer tok-1" {
			w.WriteHeader(http.StatusUnauthorized)
			return
		}
		json.NewEncoder(w).Encode(map[string]any{"user_id": 1, "role": "admin"})
	})
	mux.HandleFunc("/roles/users", func(w http.ResponseWriter, r *http.Request) {
		if got := r.URL.Query().Get("query"); got != "ali" {
			t.Fatalf("query param = %q, want %q", got, "ali")
		}
		json.NewEncoder(w).Encode(map[string]any{
			"users": []map[string]any{
				{"user_id": 7, "login": "alice", "email": "a@b.com", "role": nil,
					"created_at": "2026-01-01T00:00:00", "last_login_at": ""},
			},
		})
	})
	mux.HandleFunc("/roles/users/7/role", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			w.WriteHeader(http.StatusMethodNotAllowed)
			return
		}
		if r.Header.Get("Idempotency-Key") == "" {
			t.Fatal("client must send Idempotency-Key")
		}
		var body setRoleReq
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Fatalf("decode body: %v", err)
		}
		if body.Role != "operator" || body.Reason != "promo" || body.TicketID != "T-1" {
			t.Fatalf("unexpected body: %+v", body)
		}
		json.NewEncoder(w).Encode(map[string]any{"user_id": 7, "role": "operator"})
	})
	return httptest.NewServer(mux)
}

func TestHTTPRolesClientRoundTrip(t *testing.T) {
	srv := fakeRolesServer(t)
	defer srv.Close()
	c := NewHTTPRolesClient(srv.URL, "tok-1")
	if !c.Enabled() {
		t.Fatal("configured client should be enabled")
	}

	role, err := c.Me()
	if err != nil || role != "admin" {
		t.Fatalf("Me() = %q, %v", role, err)
	}

	users, err := c.ListUsers("ali", 50, 0)
	if err != nil {
		t.Fatalf("ListUsers: %v", err)
	}
	if len(users) != 1 || users[0].Login != "alice" || users[0].Role != "" {
		t.Fatalf("unexpected users: %+v", users)
	}

	newRole, err := c.SetRole(7, "operator", "promo", "T-1")
	if err != nil || newRole != "operator" {
		t.Fatalf("SetRole = %q, %v", newRole, err)
	}
}

func TestHTTPRolesClientSurfacesUnauthorized(t *testing.T) {
	srv := fakeRolesServer(t)
	defer srv.Close()
	c := NewHTTPRolesClient(srv.URL, "wrong-token")
	if _, err := c.Me(); err == nil {
		t.Fatal("a 401 from the server must surface as an error, not a fabricated role")
	}
}

func TestHTTPAccountClientDisabledOffline(t *testing.T) {
	var c *HTTPAccountClient
	if c.Enabled() {
		t.Fatal("nil receiver must report disabled")
	}
	if _, err := c.Account(1); err == nil {
		t.Fatal("disabled account client must error, never fabricate a balance")
	}
}
