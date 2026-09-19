//! Sender RTP/RTCP clock probe. This only measures timestamps; it never delays media.
use rtcp::sender_report::SenderReport;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const NTP_UNIX_SECONDS: i128 = 2_208_988_800;
const MAX_REPORT_DISTANCE_SECONDS: i64 = 120;

pub(super) struct RtpClockProbe {
    clock_rate: i64,
    report: Option<(u32, i128)>,
    last_report_at: Option<Instant>,
    last_timestamp: Option<u32>,
    latest_age_ms: Option<i64>,
    baseline_age_ms: Option<i64>,
    report_count: u64,
}

impl RtpClockProbe {
    pub(super) fn new(clock_rate: i64) -> Self {
        Self {
            clock_rate,
            report: None,
            last_report_at: None,
            last_timestamp: None,
            latest_age_ms: None,
            baseline_age_ms: None,
            report_count: 0,
        }
    }

    pub(super) fn sender_report(&mut self, sr: &SenderReport) {
        let seconds = i128::from(sr.ntp_time >> 32);
        if seconds < NTP_UNIX_SECONDS {
            return;
        }
        let fraction = i128::from(sr.ntp_time as u32);
        let unix_ms = (seconds - NTP_UNIX_SECONDS) * 1_000 + (fraction * 1_000 >> 32);
        self.report = Some((sr.rtp_time, unix_ms));
        self.last_report_at = Some(Instant::now());
        self.report_count += 1;
    }

    pub(super) fn receive(&mut self, timestamp: u32) {
        if self.last_timestamp == Some(timestamp) {
            return;
        }
        self.last_timestamp = Some(timestamp);
        let Some((report_rtp, report_unix_ms)) = self.report else {
            return;
        };
        let ticks = i64::from(timestamp.wrapping_sub(report_rtp) as i32);
        if ticks.abs() > self.clock_rate * MAX_REPORT_DISTANCE_SECONDS {
            return;
        }
        let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return;
        };
        let sender_time_ms = report_unix_ms + i128::from(ticks * 1_000 / self.clock_rate);
        let age_ms = now.as_millis() as i128 - sender_time_ms;
        let Ok(age_ms) = i64::try_from(age_ms) else {
            return;
        };
        self.latest_age_ms = Some(age_ms);
        self.baseline_age_ms.get_or_insert(age_ms);
    }

    pub(super) fn summary(&self, now: Instant) -> String {
        if self.report_count == 0 {
            return "? SR:0".to_owned();
        }
        let report_age = self
            .last_report_at
            .map(|time| now.saturating_duration_since(time).as_secs())
            .unwrap_or_default();
        let Some(age_ms) = self.latest_age_ms else {
            return format!("? SR:{}/{}s", self.report_count, report_age);
        };
        let drift = age_ms - self.baseline_age_ms.unwrap_or(age_ms);
        // Absolute age assumes Xbox and Vita have synchronized clocks. Drift does not.
        format!("{age_ms}ms d{drift:+} SR:{}/{}s", self.report_count, report_age)
    }

    pub(super) fn age_ms(&self) -> Option<i64> {
        self.latest_age_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_report_ntp_and_rtp_wrap_are_mapped_to_unix_time() {
        let mut clock = RtpClockProbe::new(90_000);
        let second = NTP_UNIX_SECONDS as u64 + 1_700_000_000;
        clock.sender_report(&SenderReport {
            ntp_time: second << 32,
            rtp_time: u32::MAX - 44_999,
            ..Default::default()
        });
        assert_eq!(clock.report, Some((u32::MAX - 44_999, 1_700_000_000_000)));
        assert_eq!(i64::from(45_000_u32.wrapping_sub(u32::MAX - 44_999) as i32), 90_000);
    }

    #[test]
    fn zero_ntp_is_ignored() {
        let mut clock = RtpClockProbe::new(48_000);
        clock.sender_report(&SenderReport::default());
        assert!(clock.report.is_none());
    }
}
