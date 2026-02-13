use anyhow::Result;
use hdrhistogram::Histogram;
use std::fs::File;
use std::time::Duration;

pub struct Metrics {
    pub histogram: Histogram<u64>,
    pub submitted: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub errors: u64,
    csv_writer: Option<csv::Writer<File>>,
    start_time: std::time::Instant,
}

#[derive(Debug)]
pub struct Summary {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub total: u64,
    pub rate: f64,
    pub accepted: u64,
    pub rejected: u64,
    pub errors: u64,
}

impl Metrics {
    pub fn new(csv_path: Option<String>) -> Result<Self> {
        let csv_writer = if let Some(path) = csv_path {
            let file = File::create(&path)?;
            let mut writer = csv::Writer::from_writer(file);
            writer.write_record(&["timestamp", "oid", "latency_us", "status"])?;
            Some(writer)
        } else {
            None
        };

        Ok(Self {
            histogram: Histogram::new(3)?,
            submitted: 0,
            accepted: 0,
            rejected: 0,
            errors: 0,
            csv_writer,
            start_time: std::time::Instant::now(),
        })
    }

    pub fn record_latency(&mut self, duration: Duration, status: &str, oid: &str) -> Result<()> {
        let micros = duration.as_micros() as u64;
        self.histogram.record(micros)?;

        if let Some(writer) = &mut self.csv_writer {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            writer.write_record(&[
                &timestamp.to_string(),
                oid,
                &micros.to_string(),
                status,
            ])?;
        }

        Ok(())
    }

    pub fn record_submitted(&mut self) {
        self.submitted += 1;
    }

    pub fn record_accepted(&mut self) {
        self.accepted += 1;
    }

    pub fn record_rejected(&mut self) {
        self.rejected += 1;
    }

    pub fn record_error(&mut self) {
        self.errors += 1;
    }

    pub fn summary(&self) -> Summary {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let rate = if elapsed > 0.0 {
            self.submitted as f64 / elapsed
        } else {
            0.0
        };

        Summary {
            p50: self.histogram.value_at_percentile(50.0),
            p95: self.histogram.value_at_percentile(95.0),
            p99: self.histogram.value_at_percentile(99.0),
            total: self.submitted,
            rate,
            accepted: self.accepted,
            rejected: self.rejected,
            errors: self.errors,
        }
    }

    pub fn flush(&mut self) -> Result<()> {
        if let Some(writer) = &mut self.csv_writer {
            writer.flush()?;
        }
        Ok(())
    }
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "submitted={} accepted={} rejected={} errors={} rate={:.1}/s p50={}us p95={}us p99={}us",
            self.total, self.accepted, self.rejected, self.errors,
            self.rate, self.p50, self.p95, self.p99
        )
    }
}
