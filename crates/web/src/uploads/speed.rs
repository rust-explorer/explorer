use std::collections::VecDeque;

const WINDOW_MS: i64 = 5000;

/// Upload throughput over the last few seconds.
#[derive(Debug, Default)]
pub struct SpeedMeter {
    samples: VecDeque<(i64, u64)>,
}

impl SpeedMeter {
    /// `total_bytes` is the running total sent so far.
    pub fn record(&mut self, now_ms: i64, total_bytes: u64) {
        self.samples.push_back((now_ms, total_bytes));
        while self
            .samples
            .front()
            .is_some_and(|&(at, _)| at < now_ms - WINDOW_MS)
        {
            self.samples.pop_front();
        }
    }

    pub fn bytes_per_sec(&self) -> Option<u64> {
        let (first_ms, first_bytes) = *self.samples.front()?;
        let (last_ms, last_bytes) = *self.samples.back()?;
        let elapsed = u64::try_from(last_ms - first_ms)
            .ok()
            .filter(|&ms| ms > 0)?;
        Some(last_bytes.saturating_sub(first_bytes) * 1000 / elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_meter_window() {
        let mut meter = SpeedMeter::default();
        meter.record(0, 0);
        assert_eq!(meter.bytes_per_sec(), None);
        meter.record(2000, 10_000_000);
        assert_eq!(meter.bytes_per_sec(), Some(5_000_000));

        let mut meter = SpeedMeter::default();
        meter.record(0, 0);
        meter.record(6000, 12_000_000);
        meter.record(8000, 14_000_000);
        assert_eq!(meter.bytes_per_sec(), Some(1_000_000));

        meter.record(20_000, 20_000_000);
        assert_eq!(meter.bytes_per_sec(), None);
    }
}
