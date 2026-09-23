//! Header-only traffic accounting before decryption. Never used to accept or
//! reject a packet; authentication and protocol demux remain the RTC stack's job.
use std::time::{Duration, Instant};

fn class(packet: &[u8]) -> usize {
    if packet.len() >= 20 && packet[0] & 0xc0 == 0
        && packet[4..8] == [0x21, 0x12, 0xa4, 0x42]
    {
        // STUN request/indication vs success/error response (class C1 bit).
        return if packet[0] & 1 == 0 { 3 } else { 4 };
    }
    if packet.len() >= 13 && (20..=63).contains(&packet[0]) { return 2; }
    if packet.len() >= 8 && packet[0] >> 6 == 2 {
        if (192..=223).contains(&packet[1]) { return 1; }
        if packet.len() >= 12 { return 0; }
    }
    5
}

#[derive(Default, Clone, Copy)]
struct Counter { bytes: u64, packets: u64, work_us: u64, max_us: u64 }

pub(crate) struct Traffic {
    started: Instant,
    rx: [Counter; 6],
    tx: [Counter; 6],
}

impl Traffic {
    pub(crate) fn new() -> Self {
        Self { started: Instant::now(), rx: [Counter::default(); 6], tx: [Counter::default(); 6] }
    }
    pub(crate) fn received(&mut self, packet: &[u8], processing: Duration) {
        let c = &mut self.rx[class(packet)];
        c.bytes += packet.len() as u64;
        c.packets += 1;
        let us = processing.as_micros() as u64;
        c.work_us += us;
        c.max_us = c.max_us.max(us);
    }
    pub(crate) fn sent(&mut self, packet: &[u8]) {
        let c = &mut self.tx[class(packet)];
        c.bytes += packet.len() as u64;
        c.packets += 1;
    }
    pub(crate) fn take_summary(&mut self, now: Instant) -> String {
        let elapsed_ms = now.saturating_duration_since(self.started).as_millis().max(1) as u64;
        let mut out = String::from("Wire RX kbps/pk/workMs/maxUs:");
        for (name, c) in ["RTP", "RTCP", "DTLS", "STUNq", "STUNr", "other"].iter().zip(&self.rx) {
            out.push_str(&format!(" {name}:{}/{}/{}/{}", c.bytes * 8 / elapsed_ms,
                c.packets, c.work_us / 1000, c.max_us));
        }
        out.push_str("\nWire TX kbps/pk:");
        for (name, c) in ["RTP", "RTCP", "DTLS", "STUNq", "STUNr", "other"].iter().zip(&self.tx) {
            out.push_str(&format!(" {name}:{}/{}", c.bytes * 8 / elapsed_ms, c.packets));
        }
        self.rx = [Counter::default(); 6];
        self.tx = [Counter::default(); 6];
        self.started = now;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiplexed_datagrams_are_attributed_without_payload_access() {
        let mut p = [0; 40];
        assert_eq!(class(&[]), 5);
        p[0] = 0x80; p[1] = 102; assert_eq!(class(&p), 0);
        p[1] = 102 | 0x80; assert_eq!(class(&p), 0); // RTP marker
        p[1] = 200; assert_eq!(class(&p), 1);
        p[0] = 0x8f; p[1] = 205; assert_eq!(class(&p), 1); // encrypted TWCC
        p[0] = 23; assert_eq!(class(&p), 2);
        p[0] = 0; p[1] = 1; p[4..8].copy_from_slice(&[0x21,0x12,0xa4,0x42]);
        assert_eq!(class(&p), 3);
        p[0] = 1; assert_eq!(class(&p), 4);
        p[4] = 0; assert_eq!(class(&p), 5);
        assert_eq!(class(&[0x80, 102]), 5);
    }
    #[test]
    fn windows_keep_processing_cost_and_reset_counts() {
        let mut t = Traffic::new();
        let mut p = [0; 100]; p[0] = 0x80; p[1] = 102;
        t.received(&p, Duration::from_micros(2100));
        t.received(&p, Duration::from_micros(1200));
        t.sent(&p);
        let at = t.started + Duration::from_millis(1000);
        let s = t.take_summary(at);
        assert!(s.contains("RTP:1/2/3/2100"), "{s}");
        assert!(s.contains("Wire TX kbps/pk: RTP:0/1"), "{s}");
        assert!(t.take_summary(at + Duration::from_secs(1)).contains("RTP:0/0/0/0"));
    }
}
