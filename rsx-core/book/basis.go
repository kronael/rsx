package book

// BasisDecayShift decays a stable ramp basis by 1/256 per bin (~18s half-life
// at 100ms bins) — slow enough the view never flickers, alive enough to follow
// a regime change. Shared by rsx-term (ui.market) and rsx-glass (Feed / Synth)
// so both frontends normalize the heatmap ramp identically.
const BasisDecayShift = 8

// FoldBasis advances a stable ramp reference one bin: rise instantly to a new
// observed max, otherwise decay geometrically, floored at 1. It is what stops
// the heatmap strobing when a single large order lands or cancels — the ramp
// reference tracks the regime, not the current frame's spike.
func FoldBasis(basis, observed int64) int64 {
	if observed > basis {
		return observed
	}
	basis -= basis >> BasisDecayShift
	if basis < 1 {
		basis = 1
	}
	return basis
}
