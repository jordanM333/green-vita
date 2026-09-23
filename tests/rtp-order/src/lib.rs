// Run the real RTP reorder and H.264 assembly modules on the host. Only the
// hardware submission sink and its counters are substituted; assembled bytes
// and all incomplete-FU/sequence validation come from the production code.
#![allow(dead_code)]
extern crate self as rtc;
pub use rtp;
#[path = "../../../src/streaming/video/policy.rs"]
pub mod policy;

mod streaming {
    pub mod video {
        pub(crate) use crate::policy;
        pub mod trace { pub fn record(_: &'static str, _: u32, _: u64) {} }
        use std::sync::Mutex;
        pub enum SubmitResult { Submitted, QueueFull, Disconnected }
        #[derive(Default)]
        pub struct VideoDecodeWorker { pub submitted: Mutex<Vec<Vec<u8>>> }
        impl VideoDecodeWorker {
            pub fn begin_resync(&self) {}
            pub fn take_recovery_request(&self) -> bool { false }
            pub fn submit_access_unit(&self, data: Vec<u8>, _: std::time::Instant, _: u32) -> SubmitResult {
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
            if let Some(p) = self.order.push(p.header.sequence_number, p, now) {
                self.consume(assembler, p);
            }
            while let Some(p) = self.order.pop_ready(now) { self.consume(assembler, p); }
        }
    }

    #[test]
    fn reproduce_cross_frame_loss_then_recover_both_exact_access_units() {
        // The next frame overtakes the previous FU-A end packet by 1 ms.
        let packets = vec![
            packet(10, 1000, false, &[0x7c, 0x85, 0x88]),
            packet(12, 2500, true, &[0x61, 0xaa, 0xbb]),
            packet(11, 1000, true, &[0x7c, 0x45, 0x99]),
        ];
        let mut old = video_rtp::VideoRtp::new(1280, 720);
        let worker = VideoDecodeWorker::default();
        let mut keyframe = false;
        let old_drops: u32 = packets.iter().cloned()
            .map(|p| old.receive(&worker, p, &mut keyframe).dropped).sum();
        assert_eq!(old_drops, 2);
        assert!(keyframe);
        assert_eq!(worker.submitted.lock().unwrap().len(), 0);

        let mut fixed = OrderedReceiver::default();
        let mut assembler = video_rtp::VideoRtp::new(1280, 720);
        let now = Instant::now();
        for (i, p) in packets.into_iter().enumerate() {
            fixed.receive(&mut assembler, p, now + Duration::from_millis(i as u64));
        }
        assert_eq!(fixed.drops, 0);
        assert!(!fixed.keyframe);
        assert_eq!(*fixed.worker.submitted.lock().unwrap(), vec![
            vec![0, 0, 0, 1, 0x65, 0x88, 0x99], vec![0, 0, 0, 1, 0x61, 0xaa, 0xbb],
        ]);
        assert_eq!(fixed.order.stats.filled, 1);
    }

    #[test]
    fn delayed_pump_processes_available_gap_filler_before_expiring_it() {
        let mut fixed = OrderedReceiver::default();
        let mut assembler = video_rtp::VideoRtp::new(1280, 720);
        let now = Instant::now();
        fixed.receive(&mut assembler, packet(10, 1000, false, &[0x7c, 0x85, 0x88]), now);
        fixed.receive(&mut assembler, packet(12, 2500, true, &[0x61, 0xaa, 0xbb]), now);
        // The next receive batch is processed after the 6ms deadline. Packet 13
        // arrives before the gap filler within this already-received batch.
        let delayed = now + Duration::from_millis(11);
        fixed.receive(&mut assembler, packet(13, 4000, true, &[0x61, 0xcc, 0xdd]), delayed);
        fixed.receive(&mut assembler, packet(11, 1000, true, &[0x7c, 0x45, 0x99]), delayed);
        fixed.flush(&mut assembler, delayed);
        assert_eq!(fixed.drops, 0);
        assert_eq!(fixed.order.stats.missing, 0);
        assert!(!fixed.keyframe);
        assert_eq!(*fixed.worker.submitted.lock().unwrap(), vec![
            vec![0, 0, 0, 1, 0x65, 0x88, 0x99],
            vec![0, 0, 0, 1, 0x61, 0xaa, 0xbb],
            vec![0, 0, 0, 1, 0x61, 0xcc, 0xdd],
        ]);
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
            packet(13, 2500, true, &[0x61, 0xaa, 0xbb]),
        ] { fixed.receive(&mut assembler, p, now); }
        assert!(fixed.worker.submitted.lock().unwrap().is_empty());
        fixed.flush(&mut assembler, now + Duration::from_millis(6));
        assert_eq!(fixed.drops, 2);
        assert!(fixed.keyframe);
        assert_eq!(fixed.order.stats.missing, 1);
        assert!(fixed.worker.submitted.lock().unwrap().is_empty());
        // A fragment arriving after the deadline cannot resurrect the old AU.
        fixed.receive(&mut assembler, packet(11, 1000, false, &[0x7c, 0x05, 0x77]),
            now + Duration::from_millis(7));
        assert_eq!(fixed.worker.submitted.lock().unwrap().len(), 0);
        assert_eq!(fixed.order.stats.too_late, 1);
    }

    #[test]
    fn damage_blocks_dependent_frames_until_a_complete_idr_is_submitted() {
        let mut assembler = video_rtp::VideoRtp::new(1280, 720);
        let worker = VideoDecodeWorker::default();
        let mut request = false;
        assembler.receive(&worker, packet(10, 1000, false, &[0x7c, 0x81, 0x88]), &mut request);
        let damage = assembler.receive(&worker, packet(12, 2500, true, &[0x61, 0xaa, 0xbb]), &mut request);
        assert_eq!(damage.dropped, 2);
        assert!(assembler.waiting_for_keyframe());
        assert!(worker.submitted.lock().unwrap().is_empty());
        let idr = assembler.receive(&worker, packet(13, 4000, true, &[0x65, 0xaa, 0xbb]), &mut request);
        assert_eq!(idr.submitted, 1);
        assert!(!assembler.waiting_for_keyframe());
        let next = assembler.receive(&worker, packet(14, 5500, true, &[0x61, 0xaa, 0xbb]), &mut request);
        assert_eq!(next.submitted, 1);
        assert_eq!(next.post_damage_submitted, 0);
    }
}

#[cfg(test)]
mod repair_integration {
    use super::*;
    use bytes::Bytes;
    use std::time::{Duration, Instant};
    fn packet(seq: u16, ts: u32, marker: bool, bytes: &'static [u8]) -> rtp::Packet {
        rtp::Packet { header: rtp::header::Header { sequence_number: seq, timestamp: ts, marker,
            ..Default::default() }, payload: Bytes::from_static(bytes), ..Default::default() }
    }
    #[test]
    fn recovered_tail_keeps_the_h264_reference_chain_instead_of_entering_idr_wait() {
        let t = Instant::now();
        let mut order = reorder::PacketOrder::default(); order.enable_repair(true);
        let mut video = video_rtp::VideoRtp::new(1280, 720);
        let worker = streaming::video::VideoDecodeWorker::default(); let mut keyframe = false;
        let first = packet(10, 1000, false, &[0x7c, 0x85, 0x88]);
        let next = packet(12, 2500, true, &[0x61, 0xaa, 0xbb]);
        video.receive(&worker, order.push(10, first, t).unwrap(), &mut keyframe);
        assert!(order.push(12, next, t).is_none());
        assert_eq!(order.missing_for_nack(t + Duration::from_millis(2)), vec![11]);
        assert!(order.pop(t + Duration::from_millis(30)).is_none());
        let tail = packet(11, 1000, true, &[0x7c, 0x45, 0x99]);
        let now = t + Duration::from_millis(46);
        video.receive(&worker, order.push(11, tail, now).unwrap(), &mut keyframe);
        video.receive(&worker, order.pop_ready(now).unwrap(), &mut keyframe);
        assert!(!keyframe);
        assert_eq!(*worker.submitted.lock().unwrap(), vec![vec![0,0,0,1,0x65,0x88,0x99], vec![0,0,0,1,0x61,0xaa,0xbb]]);
    }
    #[test]
    fn manual_video_repair_discards_partial_au_and_waits_for_complete_idr() {
        let mut video = video_rtp::VideoRtp::new(1280,720);
        let worker = streaming::video::VideoDecodeWorker::default(); let mut keyframe = false;
        video.receive(&worker, packet(10, 1000, false, &[0x7c,0x85,0x88]), &mut keyframe);
        video.refresh(&worker);
        assert!(video.waiting_for_keyframe());
        video.receive(&worker, packet(20, 2500, true, &[0x61,0xaa,0xbb]), &mut keyframe);
        assert!(worker.submitted.lock().unwrap().is_empty());
        video.receive(&worker, packet(21, 4000, true, &[0x65,0xbb,0xcc]), &mut keyframe);
        assert!(!video.waiting_for_keyframe());
        video.receive(&worker, packet(22, 5500, true, &[0x61,0xcc,0xdd]), &mut keyframe);
        assert_eq!(*worker.submitted.lock().unwrap(), vec![vec![0,0,0,1,0x65,0xbb,0xcc],vec![0,0,0,1,0x61,0xcc,0xdd]]);
    }
}
