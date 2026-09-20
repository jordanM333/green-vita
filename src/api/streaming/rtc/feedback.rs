//! Receive-rate diagnostics; this build sends no forced REMB ceiling.
use std::time::{Duration, Instant};

pub(crate) struct ReceiveRate {
    started: Instant,
    bytes: u64,
    bucket_start: Instant,
    bucket_bytes: u64,
    peak_kbps: u64,
    last_packet: Option<Instant>,
    max_gap_ms: u128,
}

impl ReceiveRate {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self { started: now, bytes: 0, bucket_start: now, bucket_bytes: 0,
            peak_kbps: 0, last_packet: None, max_gap_ms: 0 }
    }

    fn close_bucket(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.bucket_start).as_millis();
        if elapsed >= 100 {
            self.peak_kbps = self.peak_kbps.max(self.bucket_bytes * 8 / elapsed as u64);
            self.bucket_start = now;
            self.bucket_bytes = 0;
        }
    }

    pub(crate) fn receive(&mut self, bytes: usize, now: Instant) {
        self.close_bucket(now);
        if let Some(last) = self.last_packet {
            self.max_gap_ms = self.max_gap_ms.max(now.duration_since(last).as_millis());
        }
        self.last_packet = Some(now);
        self.bytes += bytes as u64;
        self.bucket_bytes += bytes as u64;
    }

    pub(crate) fn summary(&mut self, now: Instant) -> String {
        self.close_bucket(now);
        let elapsed = now.duration_since(self.started).as_millis().max(1);
        let kbps = self.bytes * 8 / elapsed as u64;
        let idle = self.last_packet.map(|last| now.duration_since(last).as_millis()).unwrap_or(0);
        let text = format!("{kbps}/{}k gap:{}/{}ms", self.peak_kbps, idle, self.max_gap_ms.max(idle));
        self.started = now;
        self.bytes = 0;
        self.peak_kbps = 0;
        self.max_gap_ms = 0;
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_use_elapsed_time_and_report_stalls() {
        let mut rate = ReceiveRate::new();
        let start = rate.started;
        rate.receive(25_000, start);
        rate.receive(25_000, start + Duration::from_millis(100));
        assert_eq!(rate.summary(start + Duration::from_millis(200)), "2000/2000k gap:100/100ms");
        assert_eq!(rate.summary(start + Duration::from_millis(1200)), "0/0k gap:1100/1100ms");
    }

}
