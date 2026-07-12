package main

import (
	"os"
	"path/filepath"
	"testing"
)

func writeConfig(t *testing.T, body string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "maker-config.json")
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestReadConfigOverridesGlobalMid(t *testing.T) {
	path := writeConfig(t, `{"mid_override": 51000}`)
	got := readConfigOverrides(path)
	if got.mid == nil || *got.mid != 51000 {
		t.Fatalf("mid = %v, want 51000", got.mid)
	}
	if len(got.mids) != 0 || got.depthProfile != "" {
		t.Errorf("unexpected extras: %+v", got)
	}
}

func TestReadConfigOverridesPerSymbol(t *testing.T) {
	path := writeConfig(t, `{"mids": {"10": 36000, "3": 1500000}, "depth_profile": "walls"}`)
	got := readConfigOverrides(path)
	if got.mid != nil {
		t.Errorf("global mid = %v, want nil", got.mid)
	}
	if got.mids[10] != 36000 || got.mids[3] != 1500000 {
		t.Errorf("mids = %v, want {10:36000, 3:1500000}", got.mids)
	}
	if got.depthProfile != "walls" {
		t.Errorf("depthProfile = %q, want walls", got.depthProfile)
	}
}

func TestReadConfigOverridesBadInputsAbstain(t *testing.T) {
	// Missing file, bad JSON, junk symbol keys: zero value, never a panic.
	if got := readConfigOverrides(""); got.mid != nil || got.mids != nil {
		t.Errorf("empty path: %+v", got)
	}
	if got := readConfigOverrides(writeConfig(t, `{nope`)); got.mid != nil {
		t.Errorf("bad json: %+v", got)
	}
	got := readConfigOverrides(writeConfig(t, `{"mids": {"pengu": 1, "10": 2}}`))
	if len(got.mids) != 1 || got.mids[10] != 2 {
		t.Errorf("junk keys not skipped: %v", got.mids)
	}
}

func TestMidForPrecedence(t *testing.T) {
	// Per-symbol override beats global; global beats the price source;
	// an uncovered symbol falls through to the source.
	m := newMaker(Config{}, newBBOSource())
	m.prices.(*bboSource).update(10, 29000, 31000) // mid 30000
	m.prices.(*bboSource).update(3, 1400000, 1600000)

	global := int64(42000)
	ov := configOverrides{mid: &global, mids: map[uint32]int64{10: 36000}}
	if got := m.midFor(10, ov); got != 36000 {
		t.Errorf("per-symbol: %d, want 36000", got)
	}
	if got := m.midFor(3, ov); got != 42000 {
		t.Errorf("global: %d, want 42000", got)
	}
	if got := m.midFor(3, configOverrides{}); got != 1500000 {
		t.Errorf("source: %d, want 1500000", got)
	}
}
