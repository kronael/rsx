// CASE A — tail latency under unpredictable CPU bursts (open-loop).
// A generator goroutine releases 50 requests every 1ms (50k req/s) and spawns
// one goroutine per request. 1 in 1666 requests (PRNG hash of the id,
// identical in Rust) spins 100ms of CPU — an unpredictable expensive path
// inside an ordinary handler; the rest spin 10us. Utilization stays ~45%, so
// tails come from scheduling policy, not overload. Light-request latency is
// measured against the request's INTENDED arrival tick (open-loop: immune to
// coordinated omission). Go's runtime preempts a spinning goroutine at ~10ms,
// so bursts cannot monopolize Ps the way they block tokio workers.
// Rust twin: rust/src/bin/casea.rs.
package main

import (
	"math"
	"os"
	"strconv"
	"sync"
	"time"

	"benchgovsrust/benchutil"
)

const (
	batch             = 50 // requests released per 1ms tick = 50k req/s
	tick              = time.Millisecond
	total             = 600_000 // 12s of traffic
	warmup            = 50_000  // first 1s excluded from samples
	lightSpin         = 10 * time.Microsecond
	heavySpin         = 100 * time.Millisecond
	defaultHeavyEvery = 1_666 // ~30 heavy/s => avg ~3 bursts in flight
	heavySentinel     = math.MaxUint32
)

func heavyEvery() uint64 {
	if v := os.Getenv("BENCH_HEAVY_EVERY"); v != "" {
		n, err := strconv.ParseUint(v, 10, 64)
		if err != nil {
			panic("parse BENCH_HEAVY_EVERY: " + err.Error())
		}
		return n
	}
	return defaultHeavyEvery
}

func isHeavy(id, every uint64) bool {
	rng := benchutil.Splitmix64{State: id}
	return rng.Next()%every == 0
}

func spinFor(duration time.Duration) {
	end := time.Now().Add(duration)
	for time.Now().Before(end) {
	}
}

func handle(id int, intended time.Time, lat []uint32, every uint64, wg *sync.WaitGroup) {
	defer wg.Done()
	if isHeavy(uint64(id), every) {
		spinFor(heavySpin)
		lat[id] = heavySentinel
	} else {
		spinFor(lightSpin)
		ns := time.Since(intended).Nanoseconds()
		if ns > math.MaxUint32-1 {
			ns = math.MaxUint32 - 1
		}
		lat[id] = uint32(ns)
	}
}

func main() {
	every := heavyEvery()
	lat := make([]uint32, total)
	var wg sync.WaitGroup

	cpu0 := benchutil.ReadCPUTicks()
	t0 := time.Now()
	start := time.Now()
	for t := 0; t < total/batch; t++ {
		intended := start.Add(time.Duration(t) * tick)
		if wait := time.Until(intended); wait > 0 {
			time.Sleep(wait)
		}
		for k := 0; k < batch; k++ {
			id := t*batch + k
			wg.Add(1)
			go handle(id, intended, lat, every, &wg)
		}
	}
	wg.Wait()
	wallS := time.Since(t0).Seconds()
	cpuTicks := benchutil.ReadCPUTicks() - cpu0

	checksum := uint64(0)
	samples := make([]uint32, 0, total-warmup)
	for id := 0; id < total; id++ {
		if lat[id] == heavySentinel {
			checksum ^= uint64(id)
		} else if id >= warmup {
			samples = append(samples, lat[id])
		}
	}
	checksum ^= uint64(len(samples))

	benchutil.PrintResult("A", "go", total, uint64(total), wallS, cpuTicks, samples, checksum)
}
