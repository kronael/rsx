package glass

import "time"

// Run drives the render loop: on each tick it presents the NEWEST published
// grid through r. Data-rate is decoupled from render-rate — a burst of grids on
// grids coalesces to the latest (never a backlog of stale frames), and a
// vsync-on backend paces itself inside Present. Each grid is immutable once
// sent (the producer allocates a fresh Grid per frame), so Run only ever holds
// the latest pointer; there is no scene graph.
//
// Run returns nil when grids closes, or the error a Present returns — a backend
// signals window-close that way, which the caller treats as a clean stop.
func Run(grids <-chan *Grid, r Renderer, tick time.Duration) error {
	t := time.NewTicker(tick)
	defer t.Stop()
	var latest *Grid
	for {
		select {
		case g, ok := <-grids:
			if !ok {
				return nil
			}
			latest = g
		case <-t.C:
			if latest == nil {
				continue
			}
			if err := r.Present(latest); err != nil {
				return err
			}
		}
	}
}
