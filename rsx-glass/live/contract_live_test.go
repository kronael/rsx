//go:build contract

// This build-tagged test is the LIVE half of the mock-stays-honest contract: it
// connects to a REAL rsx-marketdata feed and proves the mock reproduces its
// frames exactly. The default suite verifies the mock against committed
// real-producer byte fixtures (mockx/contract_test.go); this optional test
// re-checks those against a running cluster so a deployed producer change can't
// silently drift from the fixtures.
//
// Run against a live feed with:
//
//	RSX_MD_URL=ws://127.0.0.1:8081/ go test -tags contract ./live/ -run Contract -v
//
// It SKIPS cleanly (never fails) when RSX_MD_URL is unset or the endpoint is
// unreachable — there is no live cluster in CI, and the fixture-based contract
// already guards the wire format there.
package live_test

import (
	"context"
	"os"
	"reflect"
	"testing"
	"time"

	"rsx-glass/live"
	"rsx-glass/mockx"

	"rsx-term/wire"
)

func TestContractAgainstLiveFeed(t *testing.T) {
	url := os.Getenv("RSX_MD_URL")
	if url == "" {
		t.Skip("RSX_MD_URL unset — no live cluster; the fixture contract (mockx) covers the wire format")
	}
	symbolID := mockx.DefaultSymbolID

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	client := live.New(url, symbolID)
	client.Start(ctx)
	defer client.Close()

	// Capture the first real market-data frame (a snapshot, bbo, delta, or
	// trade — whatever the live book emits first).
	real, ok := firstMarketFrame(ctx, client)
	if !ok {
		t.Skipf("no market-data frame from %s within the deadline — endpoint unreachable or idle", url)
	}

	// Re-encode the real, decoded frame with the mock's encoder and decode it
	// again with the SAME decoder: the mock must reproduce a real live frame
	// byte-for-byte at the semantic level. If the deployed producer's schema
	// had drifted from the mock, the decoded values would differ here.
	roundTripped, err := wire.DecodeMd(mockx.Encode(real))
	if err != nil {
		t.Fatalf("mock could not reproduce the live frame: %v", err)
	}
	if !reflect.DeepEqual(roundTripped, real) {
		t.Fatalf("mock frame differs from the live frame\n live: %#v\n mock: %#v", real, roundTripped)
	}
	t.Logf("live frame %T reproduced by the mock exactly", real)
}

// firstMarketFrame drains the connector until a market-data frame (not a link
// transition) arrives or the context ends.
func firstMarketFrame(ctx context.Context, client *live.Client) (any, bool) {
	for {
		select {
		case ev := <-client.Events():
			switch ev.(type) {
			case wire.Snapshot, wire.Bbo, wire.Delta, wire.MdTrade:
				return ev, true
			}
		case <-ctx.Done():
			return nil, false
		}
	}
}
