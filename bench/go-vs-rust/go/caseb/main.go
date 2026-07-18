// CASE B — CPU + allocation-bound parallel transform with a live window.
// 8 worker goroutines (no cross-worker communication: isolates compute, the
// allocator, and GC behavior). Per message: build a 64..320-element []uint64
// from splitmix64, xor-fold it, sort it, binary-probe a rolling 512-message
// window (live heap the GC must repeatedly mark), record per-message latency.
// Rust twin: rust/src/bin/caseb.rs.
package main

import (
	"slices"
	"sync"
	"time"

	"benchgovsrust/benchutil"
)

const (
	workers        = 8
	msgsPerWorker  = 1_000_000
	warmupMsgs     = 50_000
	nMin           = 64
	nSpan          = 257 // n in [64, 320]
	ringSize       = 512
	probes         = 16
	seedMultiplier = 0x9E3779B97F4A7C15
)

func worker(id int, start *sync.WaitGroup, lags *[]uint32, sum *uint64, wg *sync.WaitGroup) {
	defer wg.Done()
	rng := benchutil.Splitmix64{State: uint64(id+1) * seedMultiplier}
	ring := make([][]uint64, ringSize)
	checksum := uint64(0)
	start.Wait()
	for m := 0; m < msgsPerWorker; m++ {
		t0 := time.Now()
		n := nMin + int(rng.Next()%nSpan)
		values := make([]uint64, 0, n)
		fold := uint64(0)
		for i := 0; i < n; i++ {
			r := rng.Next()
			values = append(values, r)
			fold ^= r
		}
		slices.Sort(values)
		median := values[n/2]
		for i := 0; i < probes; i++ {
			target := rng.Next()
			slot := int(rng.Next() % ringSize)
			entry := ring[slot]
			if len(entry) > 0 {
				pos, _ := slices.BinarySearch(entry, target)
				checksum ^= uint64(pos)
			}
		}
		checksum ^= fold ^ median
		ring[m%ringSize] = values // displaces the old message: garbage here, free() in Rust
		if m >= warmupMsgs {
			*lags = append(*lags, uint32(time.Since(t0).Nanoseconds()))
		}
	}
	*sum = checksum
}

func main() {
	var start sync.WaitGroup
	start.Add(1)
	var wg sync.WaitGroup
	lags := make([][]uint32, workers)
	sums := make([]uint64, workers)
	for id := 0; id < workers; id++ {
		lags[id] = make([]uint32, 0, msgsPerWorker-warmupMsgs)
		wg.Add(1)
		go worker(id, &start, &lags[id], &sums[id], &wg)
	}
	time.Sleep(50 * time.Millisecond) // workers parked at the start gate

	cpu0 := benchutil.ReadCPUTicks()
	t0 := time.Now()
	start.Done()
	wg.Wait()
	wallS := time.Since(t0).Seconds()
	cpuTicks := benchutil.ReadCPUTicks() - cpu0

	samples := make([]uint32, 0, workers*(msgsPerWorker-warmupMsgs))
	checksum := uint64(0)
	for id := 0; id < workers; id++ {
		samples = append(samples, lags[id]...)
		checksum ^= sums[id]
	}
	benchutil.PrintResult("B", "go", workers, uint64(workers*msgsPerWorker), wallS, cpuTicks, samples, checksum)
}
