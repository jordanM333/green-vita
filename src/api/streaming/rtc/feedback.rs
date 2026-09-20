//! Measurements and negotiated REMB ceiling; not an adaptive bandwidth estimator.
use std::time::{Duration, Instant};

pub(crate) const VIDEO_CEILING_BPS: f32 = 2_000_000.0;
pub(crate) const FEEDBACK_INTERVAL: Duration = Duration::from_millis(500);

/// Restrict REMB to video payload types that the answer actually accepts.
pub(crate) fn remb_payloads(sdp: &str) -> Vec<u8> {
    let mut video = false;
    let mut offered = Vec::new();
    let mut result = Vec::new();
    for line in sdp.lines().map(str::trim) {
        if line.starts_with("m=") {
            let fields: Vec<_> = line.split_whitespace().collect();
            video = fields.first() == Some(&"m=video") && fields.get(1) != Some(&"0");
            offered = if video {
                fields.iter().skip(3).filter_map(|s| s.parse::<u8>().ok()).collect()
            } else { Vec::new() };
        } else if video {
            if let Some(value) = line.strip_prefix("a=rtcp-fb:") {
                let fields: Vec<_> = value.split_whitespace().collect();
                if fields.get(1) == Some(&"goog-remb") {
                    if fields.first() == Some(&"*") {
                        result.extend(offered.iter().copied());
                    } else if let Some(pt) = fields.first().and_then(|s| s.parse::<u8>().ok()) {
                        if offered.contains(&pt) { result.push(pt); }
                    }
                }
            }
        }
    }
    result.sort_unstable();
    result.dedup();
    result
}

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
    fn remb_requires_an_accepted_video_payload() {
        assert_eq!(remb_payloads("m=audio 9 UDP/TLS/RTP/SAVPF 111\na=rtcp-fb:* goog-remb\nm=video 9 UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:102 goog-remb\na=rtcp-fb:103 goog-remb\n"), vec![102]);
        assert!(remb_payloads("m=video 0 UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:* goog-remb").is_empty());
        assert!(remb_payloads("m=video 9 UDP/TLS/RTP/SAVPF 102\na=rtcp-fb:102 nack").is_empty());
    }

    #[test]
    fn rates_use_elapsed_time_and_report_stalls() {
        let mut rate = ReceiveRate::new();
        let start = rate.started;
        rate.receive(25_000, start);
        rate.receive(25_000, start + Duration::from_millis(100));
        assert_eq!(rate.summary(start + Duration::from_millis(200)), "2000/2000k gap:100/100ms");
        assert_eq!(rate.summary(start + Duration::from_millis(1200)), "0/0k gap:1100/1100ms");
    }

    #[test]
    fn wildcard_remb_does_not_enable_rejected_media() {
        assert_eq!(remb_payloads("m=video 9 UDP/TLS/RTP/SAVPF 102 103\na=rtcp-fb:* goog-remb\nm=audio 9 UDP/TLS/RTP/SAVPF 111\na=rtcp-fb:* goog-remb"), vec![102, 103]);
    }
}
