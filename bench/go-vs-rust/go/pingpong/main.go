// PROBE P1 — massive-concurrency message passing.
// 50k actor pairs (100k goroutines). Each pair plays ROUNDS ping-pongs over
// two unbuffered channels (rendezvous handoff — the goroutine-native idiom;
// strict alternation makes it equivalent to the Rust twin's mpsc(1)).
// Rust twin: rust/src/bin/pingpong.rs.
package main

import (
	"sync"
	"time"

	"benchgovsrust/benchutil"
)

const (
	pairs        = 50_000
	rounds       = 200
	warmupRounds = 16
	sampleEvery  = 4
)

func pinger(start <-chan struct{}, ping chan<- uint64, pong <-chan uint64, lags *[]uint32, final *uint64, wg *sync.WaitGroup) {
	defer wg.Done()
	<-start
	v := uint64(0)
	for round := 0; round < rounds; round++ {
		sampled := round >= warmupRounds && (round-warmupRounds)%sampleEvery == 0
		if sampled {
			t0 := time.Now()
			ping <- v
			v = <-pong
			*lags = append(*lags, uint32(time.Since(t0).Nanoseconds()))
		} else {
			ping <- v
			v = <-pong
		}
	}
	close(ping)
	*final = v
}

func ponger(ping <-chan uint64, pong chan<- uint64, wg *sync.WaitGroup) {
	defer wg.Done()
	for v := range ping {
		pong <- v + 1
	}
}

func main() {
	start := make(chan struct{})
	var wg sync.WaitGroup
	perPair := (rounds-warmupRounds)/sampleEvery + 1
	lags := make([][]uint32, pairs)
	finals := make([]uint64, pairs)
	for p := 0; p < pairs; p++ {
		lags[p] = make([]uint32, 0, perPair)
		ping := make(chan uint64)
		pong := make(chan uint64)
		wg.Add(2)
		go pinger(start, ping, pong, &lags[p], &finals[p], &wg)
		go ponger(ping, pong, &wg)
	}
	// Let all goroutines reach the start gate so wall time measures the
	// exchange, not spawn cost.
	time.Sleep(300 * time.Millisecond)

	cpu0 := benchutil.ReadCPUTicks()
	t0 := time.Now()
	close(start)
	wg.Wait()
	wallS := time.Since(t0).Seconds()
	cpuTicks := benchutil.ReadCPUTicks() - cpu0

	samples := make([]uint32, 0, pairs*perPair)
	checksum := uint64(0)
	for p := 0; p < pairs; p++ {
		samples = append(samples, lags[p]...)
		checksum += finals[p]
	}
	benchutil.PrintResult("P1", "go", pairs*2, uint64(pairs*rounds), wallS, cpuTicks, samples, checksum)
}
