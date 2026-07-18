// PROBE P2 — 50k concurrent periodic tasks (timer + fan-in).
// Each goroutine ticks on an absolute deadline schedule, records wakeup lag,
// sends an 8-byte event into one shared fan-in channel. Rust twin:
// rust/src/bin/sleepers.rs.
package main

import (
	"sync"
	"time"

	"benchgovsrust/benchutil"
)

const (
	tasks       = 50_000
	ticks       = 30
	warmupTicks = 5
	intervalMS  = 100
	fanInCap    = 4096
)

func ticker(id int, start <-chan struct{}, events chan<- uint64, lags *[]uint32, wg *sync.WaitGroup) {
	defer wg.Done()
	<-start
	interval := intervalMS * time.Millisecond
	offset := interval * time.Duration(id) / tasks
	deadline := time.Now().Add(offset)
	for tick := 0; tick < ticks; tick++ {
		deadline = deadline.Add(interval)
		time.Sleep(time.Until(deadline))
		if tick >= warmupTicks {
			*lags = append(*lags, uint32(time.Since(deadline).Nanoseconds()))
		}
		events <- uint64(id)<<32 | uint64(tick)
	}
}

func main() {
	start := make(chan struct{})
	events := make(chan uint64, fanInCap)
	var wg sync.WaitGroup
	lags := make([][]uint32, tasks)
	for id := 0; id < tasks; id++ {
		lags[id] = make([]uint32, 0, ticks-warmupTicks)
		wg.Add(1)
		go ticker(id, start, events, &lags[id], &wg)
	}
	checksum := uint64(0)
	collected := make(chan struct{})
	go func() {
		for event := range events {
			checksum += event
		}
		close(collected)
	}()
	time.Sleep(300 * time.Millisecond) // tickers parked at the start gate

	cpu0 := benchutil.ReadCPUTicks()
	t0 := time.Now()
	close(start)
	wg.Wait()
	close(events)
	<-collected
	wallS := time.Since(t0).Seconds()
	cpuTicks := benchutil.ReadCPUTicks() - cpu0

	samples := make([]uint32, 0, tasks*(ticks-warmupTicks))
	for id := 0; id < tasks; id++ {
		samples = append(samples, lags[id]...)
	}
	benchutil.PrintResult("P2", "go", tasks, uint64(tasks*ticks), wallS, cpuTicks, samples, checksum)
}
