// Package wire speaks the RSX client wire protocol: the gateway's
// webproto-49 JSON order/event frames (specs/2/49-webproto.md) and the
// public marketdata protobuf feed (rsx-marketdata/marketdata.proto).
// This file holds the shared order-entry types used by both the JSON
// encoder (order.go) and the UI form.
package wire

// Side is the order/fill direction. Wire values match webproto-49's
// Side enum: 0 = BUY, 1 = SELL.
type Side uint8

const (
	Buy  Side = 0
	Sell Side = 1
)

// Label renders the side for display ("BUY"/"SELL").
func (s Side) Label() string {
	if s == Sell {
		return "SELL"
	}
	return "BUY"
}

// Tif is the order's time-in-force. Wire values match webproto-49's
// Time in Force enum: 0 = GTC, 1 = IOC, 2 = FOK.
type Tif uint8

const (
	Gtc Tif = 0
	Ioc Tif = 1
	Fok Tif = 2
)

// Label renders the TIF for display ("GTC"/"IOC"/"FOK").
func (t Tif) Label() string {
	switch t {
	case Ioc:
		return "IOC"
	case Fok:
		return "FOK"
	default:
		return "GTC"
	}
}

// Next cycles GTC -> IOC -> FOK -> GTC, for the order form's 't' key.
func (t Tif) Next() Tif {
	switch t {
	case Gtc:
		return Ioc
	case Ioc:
		return Fok
	default:
		return Gtc
	}
}

// OrderReq is a new order request, in fixed-point tick/lot units
// (Px/Qty), ready to be encoded as a webproto-49 "N" frame. Symbol is
// the target symbol id; 0 means the connection's configured default
// (the single-symbol paths never set it).
type OrderReq struct {
	Symbol     uint32
	Side       Side
	Px         int64
	Qty        int64
	Tif        Tif
	ReduceOnly bool
	PostOnly   bool
}
