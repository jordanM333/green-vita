//! Work around rtc 0.20.0-rc.2 missing the SSRC binding when the codec is learned
//! from the first packet. This interceptor only sees authenticated/decrypted RTP.
use rtc::interceptor::{Interceptor, NoopInterceptor, Packet, ReceiverReportBuilder,
    ReceiverReportInterceptor, StreamInfo, TaggedPacket};
use rtc::sansio::Protocol;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static BOUND: AtomicU64 = AtomicU64::new(0);
static GENERATED: AtomicU64 = AtomicU64::new(0);
static SENDER_REPORTS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn summary() -> String {
    format!("RR tracks:{} made:{} SRseen:{} REMB:disabled",
        BOUND.load(Ordering::Relaxed), GENERATED.load(Ordering::Relaxed),
        SENDER_REPORTS.load(Ordering::Relaxed))
}

pub(crate) struct ReceiveReports {
    inner: ReceiverReportInterceptor<NoopInterceptor>,
    // Provider-supplied codec clocks, matching the payload types in its offer.
    clocks: Vec<(u8, u32)>,
    bound: HashMap<u32, u32>,
}

impl ReceiveReports {
    pub(crate) fn new(inner: NoopInterceptor, clocks: Vec<(u8, u32)>) -> Self {
        BOUND.store(0, Ordering::Relaxed);
        GENERATED.store(0, Ordering::Relaxed);
        SENDER_REPORTS.store(0, Ordering::Relaxed);
        Self {
            inner: ReceiverReportBuilder::new().with_interval(Duration::from_secs(1)).build()(inner),
            clocks,
            bound: HashMap::new(),
        }
    }

    fn bind_once(&mut self, info: &StreamInfo) {
        if info.clock_rate == 0 || self.bound.get(&info.ssrc) == Some(&info.clock_rate) {
            return;
        }
        self.inner.bind_remote_stream(info);
        self.bound.insert(info.ssrc, info.clock_rate);
        BOUND.store(self.bound.len() as u64, Ordering::Relaxed);
    }
}

impl Protocol<TaggedPacket, TaggedPacket, ()> for ReceiveReports {
    type Rout = TaggedPacket;
    type Wout = TaggedPacket;
    type Eout = ();
    type Error = rtc::shared::error::Error;
    type Time = Instant;

    fn handle_read(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
        match &msg.message {
            Packet::Rtp(packet) => {
                if let Some((_, clock_rate)) = self.clocks.iter()
                    .find(|(pt, _)| *pt == packet.header.payload_type)
                {
                    self.bind_once(&StreamInfo {
                        ssrc: packet.header.ssrc,
                        payload_type: packet.header.payload_type,
                        clock_rate: *clock_rate,
                        ..Default::default()
                    });
                }
            }
            Packet::Rtcp(packets) => {
                for packet in packets {
                    if packet.as_any().is::<rtc::rtcp::sender_report::SenderReport>() {
                        SENDER_REPORTS.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
        self.inner.handle_read(msg)
    }

    fn poll_read(&mut self) -> Option<Self::Rout> { self.inner.poll_read() }
    fn handle_write(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> { self.inner.handle_write(msg) }
    fn poll_write(&mut self) -> Option<Self::Wout> {
        let msg = self.inner.poll_write()?;
        if let Packet::Rtcp(packets) = &msg.message {
            for packet in packets {
                if packet.as_any().is::<rtc::rtcp::receiver_report::ReceiverReport>() {
                    GENERATED.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        Some(msg)
    }
    fn handle_event(&mut self, evt: ()) -> Result<(), Self::Error> { self.inner.handle_event(evt) }
    fn poll_event(&mut self) -> Option<Self::Eout> { self.inner.poll_event() }
    fn handle_timeout(&mut self, now: Instant) -> Result<(), Self::Error> { self.inner.handle_timeout(now) }
    fn poll_timeout(&mut self) -> Option<Instant> { self.inner.poll_timeout() }
    fn close(&mut self) -> Result<(), Self::Error> { self.inner.close() }
}

impl Interceptor for ReceiveReports {
    fn bind_local_stream(&mut self, info: &StreamInfo) { self.inner.bind_local_stream(info); }
    fn unbind_local_stream(&mut self, info: &StreamInfo) { self.inner.unbind_local_stream(info); }
    fn bind_remote_stream(&mut self, info: &StreamInfo) { self.bind_once(info); }
    fn unbind_remote_stream(&mut self, info: &StreamInfo) {
        self.bound.remove(&info.ssrc);
        BOUND.store(self.bound.len() as u64, Ordering::Relaxed);
        self.inner.unbind_remote_stream(info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(ssrc: u32, pt: u8, seq: u16, now: Instant) -> TaggedPacket {
        TaggedPacket { now, transport: Default::default(), message: Packet::Rtp(rtc::rtp::Packet {
            header: rtc::rtp::header::Header { ssrc, payload_type: pt, sequence_number: seq,
                timestamp: u32::from(seq) * 1500, ..Default::default() },
            ..Default::default()
        }) }
    }

    fn reports(chain: &mut impl Interceptor, now: Instant) -> Vec<rtc::rtcp::receiver_report::ReceiverReport> {
        chain.handle_timeout(now).unwrap();
        let mut result = Vec::new();
        while let Some(msg) = chain.poll_write() {
            if let Packet::Rtcp(packets) = msg.message {
                for packet in packets {
                    if let Some(rr) = packet.as_any().downcast_ref::<rtc::rtcp::receiver_report::ReceiverReport>() {
                        result.push(rr.clone());
                    }
                }
            }
        }
        result
    }

    #[test]
    fn reproduce_unbound_library_then_generate_video_and_audio_reports() {
        let mut bare = ReceiverReportBuilder::new().build()(NoopInterceptor::new());
        let mut fixed = ReceiveReports::new(NoopInterceptor::new(), vec![(102, 90000), (111, 48000)]);
        let now = Instant::now();
        for (ssrc, pt, seq) in [(10, 102, 100), (10, 102, 102), (20, 111, 40), (20, 111, 41)] {
            let msg = packet(ssrc, pt, seq, now);
            let expected = msg.message.clone();
            bare.handle_read(packet(ssrc, pt, seq, now)).unwrap();
            fixed.handle_read(msg).unwrap();
            assert_eq!(fixed.poll_read().unwrap().message, expected);
        }
        assert!(reports(&mut bare, now + Duration::from_secs(1)).is_empty());
        let output = reports(&mut fixed, now + Duration::from_secs(1));
        assert_eq!(output.len(), 2);
        let video = &output.iter().find(|r| r.reports[0].ssrc == 10).unwrap().reports[0];
        assert_eq!(video.last_sequence_number, 102);
        assert_eq!(video.total_lost, 1);
        let audio = &output.iter().find(|r| r.reports[0].ssrc == 20).unwrap().reports[0];
        assert_eq!(audio.total_lost, 0);
        assert_eq!(fixed.bound.get(&20), Some(&48000));
        // Repeated binding must not erase history after the first report.
        fixed.bind_remote_stream(&StreamInfo { ssrc: 10, clock_rate: 90000, ..Default::default() });
        fixed.handle_read(packet(10, 102, 103, now + Duration::from_secs(2))).unwrap();
        let output = reports(&mut fixed, now + Duration::from_secs(2));
        let video = &output.iter().find(|r| r.reports[0].ssrc == 10).unwrap().reports[0];
        assert_eq!(video.total_lost, 1);
    }

    #[test]
    fn unknown_payload_is_not_guessed_and_unbinding_clears_report_state() {
        let mut fixed = ReceiveReports::new(NoopInterceptor::new(), vec![(102, 90000)]);
        let now = Instant::now();
        fixed.handle_read(packet(10, 99, 1, now)).unwrap();
        assert!(reports(&mut fixed, now + Duration::from_secs(1)).is_empty());
        fixed.handle_read(packet(10, 102, 2, now)).unwrap();
        fixed.unbind_remote_stream(&StreamInfo { ssrc: 10, ..Default::default() });
        assert!(reports(&mut fixed, now + Duration::from_secs(2)).is_empty());
    }
}
