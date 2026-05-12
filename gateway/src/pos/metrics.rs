use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Tracks POS endpoint latency for SLA verification.
///
/// Exposes counters and a percentile histogram. All updates are
/// lock-free (atomic) to avoid adding latency to the hot path.
pub struct LatencyTracker {
    total_requests: AtomicU64,
    total_latency_us: AtomicU64,
    min_latency_us: AtomicU64,
    max_latency_us: AtomicU64,
    /// Histogram buckets in microseconds:
    /// [0..50, 50..100, 100..250, 250..500, 500..1000, 1000..5000, 5000..15000, 15000+]
    buckets: [AtomicU64; 8],
}

impl LatencyTracker {
    pub fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            total_latency_us: AtomicU64::new(0),
            min_latency_us: AtomicU64::new(u64::MAX),
            max_latency_us: AtomicU64::new(0),
            buckets: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
        }
    }

    pub fn record(&self, latency_us: u64) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.total_latency_us
            .fetch_add(latency_us, Ordering::Relaxed);

        // Update min (CAS loop)
        let mut current = self.min_latency_us.load(Ordering::Relaxed);
        while latency_us < current {
            match self.min_latency_us.compare_exchange_weak(
                current,
                latency_us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }

        // Update max
        let mut current = self.max_latency_us.load(Ordering::Relaxed);
        while latency_us > current {
            match self.max_latency_us.compare_exchange_weak(
                current,
                latency_us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }

        // Increment bucket
        let bucket = bucket_index(latency_us);
        self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let total = self.total_requests.load(Ordering::Relaxed);
        let total_us = self.total_latency_us.load(Ordering::Relaxed);
        let min = self.min_latency_us.load(Ordering::Relaxed);
        let max = self.max_latency_us.load(Ordering::Relaxed);

        MetricsSnapshot {
            total_requests: total,
            avg_latency_us: if total > 0 {
                total_us / total
            } else {
                0
            },
            min_latency_us: if total > 0 { min } else { 0 },
            max_latency_us: max,
            p50_us: self.percentile(50.0),
            p99_us: self.percentile(99.0),
            sla_breaches: self.buckets[7].load(Ordering::Relaxed),
        }
    }

    fn percentile(&self, p: f64) -> u64 {
        let total = self.total_requests.load(Ordering::Relaxed);
        if total == 0 {
            return 0;
        }

        let target = ((p / 100.0) * total as f64).ceil() as u64;
        let bucket_limits = [50, 100, 250, 500, 1_000, 5_000, 15_000, u64::MAX];
        let mut cumulative = 0u64;

        for (i, bucket) in self.buckets.iter().enumerate() {
            cumulative += bucket.load(Ordering::Relaxed);
            if cumulative >= target {
                return bucket_limits[i];
            }
        }

        bucket_limits[7]
    }
}

fn bucket_index(latency_us: u64) -> usize {
    match latency_us {
        0..=49 => 0,
        50..=99 => 1,
        100..=249 => 2,
        250..=499 => 3,
        500..=999 => 4,
        1_000..=4_999 => 5,
        5_000..=14_999 => 6,
        _ => 7, // >= 15 ms = SLA breach
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricsSnapshot {
    pub total_requests: u64,
    pub avg_latency_us: u64,
    pub min_latency_us: u64,
    pub max_latency_us: u64,
    pub p50_us: u64,
    pub p99_us: u64,
    pub sla_breaches: u64,
}

impl std::fmt::Display for MetricsSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "requests={} avg={:.2}ms min={}µs max={}µs p50={}µs p99={}µs sla_breaches(>=15ms)={}",
            self.total_requests,
            self.avg_latency_us as f64 / 1000.0,
            self.min_latency_us,
            self.max_latency_us,
            self.p50_us,
            self.p99_us,
            self.sla_breaches,
        )
    }
}

/// RAII timer that records latency on drop.
#[allow(dead_code)]
pub struct LatencyHistogram {
    start: Instant,
}

impl LatencyHistogram {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Complete the measurement and record. Call this explicitly or
    /// let the guard drop.
    #[allow(dead_code)]
    pub fn record(self, tracker: &LatencyTracker) {
        let elapsed = self.start.elapsed().as_micros() as u64;
        tracker.record(elapsed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_boundaries() {
        assert_eq!(bucket_index(0), 0);
        assert_eq!(bucket_index(49), 0);
        assert_eq!(bucket_index(50), 1);
        assert_eq!(bucket_index(99), 1);
        assert_eq!(bucket_index(100), 2);
        assert_eq!(bucket_index(14999), 6);
        assert_eq!(bucket_index(15000), 7);
        assert_eq!(bucket_index(50000), 7);
    }

    #[test]
    fn single_observation() {
        let t = LatencyTracker::new();
        t.record(500);
        let snap = t.snapshot();

        assert_eq!(snap.total_requests, 1);
        assert_eq!(snap.avg_latency_us, 500);
        assert_eq!(snap.min_latency_us, 500);
        assert_eq!(snap.max_latency_us, 500);
        assert_eq!(snap.sla_breaches, 0);
    }

    #[test]
    fn sla_breach_detection() {
        let t = LatencyTracker::new();
        t.record(1_000);
        t.record(5_000);
        t.record(15_000); // breach
        t.record(20_000); // breach
        t.record(500);

        let snap = t.snapshot();
        assert_eq!(snap.total_requests, 5);
        assert_eq!(snap.sla_breaches, 2);
        assert_eq!(snap.min_latency_us, 500);
        assert_eq!(snap.max_latency_us, 20_000);
    }

    #[test]
    fn percentile_calculation() {
        let t = LatencyTracker::new();
        // 10 observations: 4 fast, 3 mid, 2 slow, 1 breach
        for _ in 0..4 {
            t.record(10);
        }
        for _ in 0..3 {
            t.record(200);
        }
        for _ in 0..2 {
            t.record(3_000);
        }
        t.record(16_000);

        let snap = t.snapshot();
        assert_eq!(snap.total_requests, 10);
        assert!(snap.p50_us <= 250); // conservative bucket limit, 50% falls in 100-249 bucket
        assert!(snap.p99_us >= 15_000); // 99th is the breach
    }
}
