// Package feed carries the small, transport-agnostic messages the UI folds:
// gateway / marketdata link-state transitions, a measured round-trip latency
// sample, and the Submitter interface the order form drives. It decouples the
// ui package from the conn package — either one can depend on feed without
// depending on the other.
//
// feed deliberately does NOT import bubbletea: these are plain structs that
// callers wrap (alongside the raw wire.* market-data / private-event types)
// as tea.Msg. Mirrors the GwEvent surface of rsx-tui/src/conn.rs. See
// specs/2/55-terminal.md.
package feed

import (
	"rsx-term/book"
	"rsx-term/wire"
)

// GwUp / GwDown are the private (order) gateway link transitions.
type GwUp struct{}

// GwDown signals the private gateway link went down.
type GwDown struct{}

// MdUp / MdDown are the public marketdata link transitions.
type MdUp struct{}

// MdDown signals the public marketdata link went down.
type MdDown struct{}

// Latency is one measured round-trip. The offline mock/demo supplies full leg
// splits; the live wire only ever fills TotalNs, with the leg fields left at
// book.NsUnknown — webproto-49 carries no gateway-side timing stamps.
type Latency struct {
	Sample book.Sample
}

// Submitter is how the UI sends orders: implemented by the offline mock now
// and by the live gateway conn in a future increment.
type Submitter interface {
	Submit(o wire.OrderReq) error
	Cancel(cid string) error
}
