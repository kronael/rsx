//! Shared measurement helpers. Mirrored 1:1 by go/benchutil — any change here
//! must land there too, or the cross-language checksum/percentile comparison
//! breaks.

/// splitmix64 PRNG — identical constants in the Go twin so both sides
/// generate byte-identical workloads from the same seed.
pub struct Splitmix64(pub u64);

impl Splitmix64 {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
}

/// Nearest-rank percentile over an ascending-sorted slice.
pub fn percentile(sorted: &[u32], q: f64) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    let mut idx = (q * sorted.len() as f64).ceil() as usize;
    idx = idx.saturating_sub(1);
    if idx >= sorted.len() {
        idx = sorted.len() - 1;
    }
    sorted[idx]
}

/// Peak resident set size (VmHWM) in kB, from /proc/self/status.
pub fn read_peak_rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            let kb = rest.trim().trim_end_matches(" kB").trim();
            return kb.parse().expect("parse VmHWM");
        }
    }
    0
}

/// Process CPU time (utime+stime) in clock ticks, from /proc/self/stat.
/// Linux USER_HZ is 100 on this class of kernel; the runner treats ticks/100
/// as seconds on both sides, so any drift cancels in the comparison.
pub fn read_cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").expect("read /proc/self/stat");
    let after_comm = &stat[stat.rfind(')').expect("comm paren") + 2..];
    let fields: Vec<&str> = after_comm.split_whitespace().collect();
    // after ')': state=0, ..., utime=11, stime=12
    let utime: u64 = fields[11].parse().expect("utime");
    let stime: u64 = fields[12].parse().expect("stime");
    utime + stime
}

/// One benchmark run's identity + aggregate numbers, for the RESULT line.
pub struct RunInfo {
    pub case: &'static str,
    pub tasks: usize,
    pub ops: u64,
    pub wall_s: f64,
    pub cpu_ticks: u64,
    pub checksum: u64,
}

/// Sort samples and print the single RESULT line the runner parses.
pub fn print_result(info: &RunInfo, samples: &mut [u32]) {
    samples.sort_unstable();
    let thr = info.ops as f64 / info.wall_s;
    let cpu_cores = (info.cpu_ticks as f64 / 100.0) / info.wall_s;
    println!(
        "RESULT case={} lang=rust tasks={} ops={} wall_s={:.3} \
         thr_ops_s={thr:.0} p50_ns={} p99_ns={} p999_ns={} max_ns={} \
         rss_peak_mb={:.1} cpu_cores={cpu_cores:.2} samples={} checksum={:x}",
        info.case,
        info.tasks,
        info.ops,
        info.wall_s,
        percentile(samples, 0.50),
        percentile(samples, 0.99),
        percentile(samples, 0.999),
        samples.last().copied().unwrap_or(0),
        read_peak_rss_kb() as f64 / 1024.0,
        samples.len(),
        info.checksum,
    );
}
