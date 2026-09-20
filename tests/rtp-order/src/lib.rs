// Run the real RTP reorder and H.264 assembly modules on the host. Only the
// hardware submission sink and its counters are substituted; assembled bytes
// and all incomplete-FU/sequence validation come from the production code.
#![allow(dead_code)]
extern crate self as rtc;
pub use rtp;

mod streaming {
    pub mod video {
        use std::sync::Mutex;
        pub enum SubmitResult { Submitted, QueueFull, Disconnected }
        #[derive(Default)]
        pub struct VideoDecodeWorker { pub submitted: Mutex<Vec<Vec<u8>>> }
        impl VideoDecodeWorker {
            pub fn begin_resync(&self) {}
            pub fn submit_access_unit(&self, data: Vec<u8>, _: Option<u64>) -> SubmitResult {
                self.submitted.lock().unwrap().push(data);
                SubmitResult::Submitted
            }
        }
        pub mod metrics {
            use std::sync::atomic::AtomicU64;
            pub struct Metrics {
                pub audio_rtp_gaps: AtomicU64,
                pub audio_rtp_late: AtomicU64,
                pub audio_rtp_backlog_ms: AtomicU64,
                pub audio_rtp_lost: AtomicU64,
                pub rtp_assembly_sum_us: AtomicU64,
                pub rtp_assembly_count: AtomicU64,
                pub rtp_assembly_max_us: AtomicU64,
            }
            pub static METRICS: Metrics = Metrics {
                audio_rtp_gaps: AtomicU64::new(0), audio_rtp_late: AtomicU64::new(0),
                audio_rtp_backlog_ms: AtomicU64::new(0), audio_rtp_lost: AtomicU64::new(0),
                rtp_assembly_sum_us: AtomicU64::new(0), rtp_assembly_count: AtomicU64::new(0),
                rtp_assembly_max_us: AtomicU64::new(0),
            };
        }
    }
}

#[path = "../../../src/api/streaming/rtc/reorder.rs"]
mod reorder;
#[path = "../../../src/api/streaming/rtc/rtp.rs"]
mod video_rtp;

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use std::time::{Duration, Instant};
    use streaming::video::VideoDecodeWorker;

    fn packet(seq: u16, ts: u32, marker: bool, payload: &'static [u8]) -> rtp::Packet {
        rtp::Packet {
            header: rtp::header::Header {
                sequence_number: seq, timestamp: ts, marker, ..Default::default()
            }, payload: Bytes::from_static(payload), ..Default::default()
        }
    }

    #[derive(Default)]
    struct OrderedReceiver {
        order: reorder::PacketOrder<rtp::Packet>,
        worker: VideoDecodeWorker,
        drops: u32,
        keyframe: bool,
    }

    impl OrderedReceiver {
        fn consume(&mut self, assembler: &mut video_rtp::VideoRtp, packet: rtp::Packet) {
            self.drops += assembler.receive(&self.worker, packet, &mut self.keyframe).dropped;
        }
        fn flush(&mut self, assembler: &mut video_rtp::VideoRtp, now: Instant) {
            while let Some(p) = self.order.pop(now) { self.consume(assembler, p); }
        }
        fn receive(&mut self, assembler: &mut video_rtp::VideoRtp, p: rtp::Packet, now: Instant) {
            self.flush(assembler, now);
            if let Some(p) = self.order.push(p.header.sequence_number, p, now) {
                self.consume(assembler, p);
            }
            self.flush(assembler, now);
        }
    }

    #[test]
    fn reproduce_cross_frame_loss_then_recover_both_exact_access_units() {
        // The next frame overtakes the previous FU-A end packet by 1 ms.
        let packets = vec![
            packet(10, 1000, false, &[0x7c, 0x85, 0x88]),
            packet(12, 2500, true, &[0x61, 0xaa]),
            packet(11, 1000, true, &[0x7c, 0x45, 0x99]),
        ];
        let mut old = video_rtp::VideoRtp::new(1280, 720);
        let worker = VideoDecodeWorker::default();
        let mut keyframe = false;
        let old_drops: u32 = packets.iter().cloned()
            .map(|p| old.receive(&worker, p, &mut keyframe).dropped).sum();
        assert_eq!(old_drops, 1);
        assert!(keyframe);
        assert_eq!(worker.submitted.lock().unwrap().len(), 1);

        let mut fixed = OrderedReceiver::default();
        let mut assembler = video_rtp::VideoRtp::new(1280, 720);
        let now = Instant::now();
        for (i, p) in packets.into_iter().enumerate() {
            fixed.receive(&mut assembler, p, now + Duration::from_millis(i as u64));
        }
        assert_eq!(fixed.drops, 0);
        assert!(!fixed.keyframe);
        assert_eq!(*fixed.worker.submitted.lock().unwrap(), vec![
            vec![0, 0, 0, 1, 0x65, 0x88, 0x99], vec![0, 0, 0, 1, 0x61, 0xaa],
        ]);
        assert_eq!(fixed.order.stats.filled, 1);
    }

    #[test]
    fn real_missing_fragment_still_discards_damaged_au_after_bounded_wait() {
        let mut fixed = OrderedReceiver::default();
        let mut assembler = video_rtp::VideoRtp::new(1280, 720);
        let now = Instant::now();
        for p in [
            packet(10, 1000, false, &[0x7c, 0x85, 0x88]),
            // The middle FU-A fragment, sequence 11, never arrives.
            packet(12, 1000, true, &[0x7c, 0x45, 0x99]),
            packet(13, 2500, true, &[0x61, 0xaa]),
        ] { fixed.receive(&mut assembler, p, now); }
        assert!(fixed.worker.submitted.lock().unwrap().is_empty());
        fixed.flush(&mut assembler, now + Duration::from_millis(6));
        assert_eq!(fixed.drops, 1);
        assert!(fixed.keyframe);
        assert_eq!(fixed.order.stats.missing, 1);
        assert_eq!(*fixed.worker.submitted.lock().unwrap(), vec![vec![0, 0, 0, 1, 0x61, 0xaa]]);
        // A fragment arriving after the deadline cannot resurrect the old AU.
        fixed.receive(&mut assembler, packet(11, 1000, false, &[0x7c, 0x05, 0x77]),
            now + Duration::from_millis(7));
        assert_eq!(fixed.worker.submitted.lock().unwrap().len(), 1);
        assert_eq!(fixed.order.stats.too_late, 1);
    }
}
