// Package benchutil mirrors rust/src/lib.rs 1:1 — any change here must land
// there too, or the cross-language checksum/percentile comparison breaks.
package benchutil

import (
	"fmt"
	"math"
	"os"
	"runtime"
	"slices"
	"strconv"
	"strings"
)

// Splitmix64 PRNG — identical constants in the Rust twin so both sides
// generate byte-identical workloads from the same seed.
type Splitmix64 struct {
	State uint64
}

func (s *Splitmix64) Next() uint64 {
	s.State += 0x9E3779B97F4A7C15
	z := s.State
	z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9
	z = (z ^ (z >> 27)) * 0x94D049BB133111EB
	return z ^ (z >> 31)
}

// Percentile is the nearest-rank percentile over an ascending-sorted slice.
func Percentile(sorted []uint32, q float64) uint32 {
	if len(sorted) == 0 {
		return 0
	}
	idx := int(math.Ceil(q*float64(len(sorted)))) - 1
	if idx < 0 {
		idx = 0
	}
	if idx >= len(sorted) {
		idx = len(sorted) - 1
	}
	return sorted[idx]
}

// ReadPeakRSSKB returns VmHWM in kB from /proc/self/status.
func ReadPeakRSSKB() uint64 {
	status, err := os.ReadFile("/proc/self/status")
	if err != nil {
		panic("read /proc/self/status: " + err.Error())
	}
	for _, line := range strings.Split(string(status), "\n") {
		if rest, ok := strings.CutPrefix(line, "VmHWM:"); ok {
			kb := strings.TrimSpace(strings.TrimSuffix(strings.TrimSpace(rest), " kB"))
			n, err := strconv.ParseUint(kb, 10, 64)
			if err != nil {
				panic("parse VmHWM: " + err.Error())
			}
			return n
		}
	}
	return 0
}

// ReadCPUTicks returns process utime+stime in clock ticks from
// /proc/self/stat. Linux USER_HZ is 100 on this class of kernel; the runner
// treats ticks/100 as seconds on both sides, so any drift cancels.
func ReadCPUTicks() uint64 {
	stat, err := os.ReadFile("/proc/self/stat")
	if err != nil {
		panic("read /proc/self/stat: " + err.Error())
	}
	s := string(stat)
	afterComm := s[strings.LastIndexByte(s, ')')+2:]
	fields := strings.Fields(afterComm)
	// after ')': state=0, ..., utime=11, stime=12
	utime, err := strconv.ParseUint(fields[11], 10, 64)
	if err != nil {
		panic("parse utime: " + err.Error())
	}
	stime, err := strconv.ParseUint(fields[12], 10, 64)
	if err != nil {
		panic("parse stime: " + err.Error())
	}
	return utime + stime
}

// PrintResult sorts samples and prints the single RESULT line the runner
// parses, plus Go-runtime GC counters as extra fields.
func PrintResult(caseName, lang string, tasks int, ops uint64, wallS float64, cpuTicks uint64, samples []uint32, checksum uint64) {
	slices.Sort(samples)
	thr := float64(ops) / wallS
	cpuCores := (float64(cpuTicks) / 100.0) / wallS
	var stats runtime.MemStats
	runtime.ReadMemStats(&stats)
	maxNs := uint32(0)
	if len(samples) > 0 {
		maxNs = samples[len(samples)-1]
	}
	fmt.Printf(
		"RESULT case=%s lang=%s tasks=%d ops=%d wall_s=%.3f thr_ops_s=%.0f "+
			"p50_ns=%d p99_ns=%d p999_ns=%d max_ns=%d rss_peak_mb=%.1f "+
			"cpu_cores=%.2f samples=%d checksum=%x numgc=%d gc_pause_total_ms=%.1f gogc=%s\n",
		caseName, lang, tasks, ops, wallS, thr,
		Percentile(samples, 0.50), Percentile(samples, 0.99),
		Percentile(samples, 0.999), maxNs,
		float64(ReadPeakRSSKB())/1024.0, cpuCores, len(samples), checksum,
		stats.NumGC, float64(stats.PauseTotalNs)/1e6, gogcSetting())
}

func gogcSetting() string {
	if v := os.Getenv("GOGC"); v != "" {
		return v
	}
	return "100"
}
