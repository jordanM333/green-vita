// Run the real RTP reorder and H.264 assembly modules on the host. Only the
// hardware submission sink and its counters are substituted; assembled bytes
// and all incomplete-FU/sequence validation come from the production code.
#![allow(dead_code)]
extern crate self as rtc;
pub use rtp;
#[path = "../../../src/streaming/video/live_edge.rs"]
pub(crate) mod live_edge;
#[cfg(test)]
mod live_edge_replay;
#[cfg(test)]
mod recovery_regression;
#[path = "../../../src/streaming/video/policy.rs"]
pub mod policy;

mod streaming {
    pub(crate) use crate::audio_timing;
    pub mod video {
        pub(crate) use crate::live_edge;
        pub(crate) use crate::policy;
        pub mod trace {
            pub fn record(_: &'static str, _: u32, _: u64) {}
        }
        use std::sync::Mutex;
        pub enum SubmitResult {
            Submitted,
            QueueFull,
            Disconnected,
        }
        #[derive(Default)]
        pub struct VideoDecodeWorker {
            pub submitted: Mutex<Vec<Vec<u8>>>,
            pub submitted_times: Mutex<Vec<(std::time::Instant, u32)>>,
            pub cutovers: std::sync::atomic::AtomicUsize,
            pub queued: std::sync::atomic::AtomicUsize,
            pub resyncs: std::sync::atomic::AtomicUsize,
            pub edge: Mutex<live_edge::LiveEdge>,
        }
        impl VideoDecodeWorker {
            pub fn observe_media(&self, ts: u32, seq: u16, at: std::time::Instant) -> bool {
                self.edge.lock().unwrap().observe(ts, seq, at, at)
            }
            pub fn poll_media(&self, now: std::time::Instant) -> bool {
                self.edge.lock().unwrap().poll(now)
            }
            pub fn media_ingress_useful(&self, ts: u32, now: std::time::Instant) -> bool {
                self.edge.lock().unwrap().ingress_useful(ts, now)
            }
            pub fn media_admits(&self, ts: u32, idr: bool, at: std::time::Instant) -> bool {
                self.edge.lock().unwrap().admit(ts, idr, at)
            }
            pub fn media_submitted(&self, ts: u32, idr: bool, at: std::time::Instant) {
                self.edge.lock().unwrap().submitted(ts, idr, at);
            }
            pub fn media_damage(&self) {
                self.edge.lock().unwrap().damage();
            }
            pub fn media_recovering(&self) -> bool {
                self.edge.lock().unwrap().recovering()
            }
            pub fn discard_queued(&self) {
                self.queued.store(0, std::sync::atomic::Ordering::Relaxed);
            }
            pub fn begin_resync(&self) {
                self.resyncs
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            pub fn take_recovery_request(&self) -> bool {
                false
            }
            pub fn queued_frames(&self) -> usize {
                self.queued.load(std::sync::atomic::Ordering::Relaxed)
            }
            pub fn submit_refresh_access_unit(
                &self,
                data: Vec<u8>,
                at: std::time::Instant,
                ts: u32,
            ) -> SubmitResult {
                self.cutovers
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.submit_access_unit(data, at, ts)
            }
            pub fn submit_access_unit(
                &self,
                data: Vec<u8>,
                at: std::time::Instant,
                ts: u32,
            ) -> SubmitResult {
                self.submitted.lock().unwrap().push(data);
                self.submitted_times.lock().unwrap().push((at, ts));
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
                audio_rtp_gaps: AtomicU64::new(0),
                audio_rtp_late: AtomicU64::new(0),
                audio_rtp_backlog_ms: AtomicU64::new(0),
                audio_rtp_lost: AtomicU64::new(0),
                rtp_assembly_sum_us: AtomicU64::new(0),
                rtp_assembly_count: AtomicU64::new(0),
                rtp_assembly_max_us: AtomicU64::new(0),
            };
        }
    }
}

#[path = "../../../src/streaming/audio_timing.rs"]
pub(crate) mod audio_timing;

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
                sequence_number: seq,
                timestamp: ts,
                marker,
                ..Default::default()
            },
            payload: Bytes::from_static(payload),
        }
    }

    #[test]
    fn sample_builder_release_preserves_audio_arrival_age_across_wrap_and_stall() {
        let mut audio = video_rtp::AudioRtp::new(48_000, 0);
        let old = Instant::now() - Duration::from_secs(6);
        let mut ready = Vec::new();
        audio.receive(
            packet(u16::MAX, u32::MAX - 959, false, &[0xf8, 0xff, 0xfe]),
            old,
            &mut ready,
        );
        audio.receive(
            packet(0, 0, false, &[0xf8, 0xff, 0xfe]),
            Instant::now(),
            &mut ready,
        );
        audio.receive(
            packet(1, 960, false, &[0xf8, 0xff, 0xfe]),
            Instant::now(),
            &mut ready,
        );
        assert!(!ready.is_empty());
        assert_eq!(ready[0].received_at, old);
        assert!(!ready[0].fits_playback(Instant::now(), Duration::ZERO, Duration::ZERO));
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
            self.drops += assembler
                .receive(&self.worker, packet, &mut self.keyframe)
                .dropped;
        }
        fn flush(&mut self, assembler: &mut video_rtp::VideoRtp, now: Instant) {
            while let Some(p) = self.order.pop(now) {
                self.consume(assembler, p);
            }
        }
        fn receive(&mut self, assembler: &mut video_rtp::VideoRtp, p: rtp::Packet, now: Instant) {
            if let Some(p) = self.order.push(p.header.sequence_number, p, now) {
                self.consume(assembler, p);
            }
            while let Some(p) = self.order.pop_ready(now) {
                self.consume(assembler, p);
            }
        }
    }

    #[test]
    fn padding_between_fu_fragments_is_not_lost_media_even_across_wrap() {
        for seq in [10_u16, 65534] {
            let mut receiver = OrderedReceiver::default();
            let mut assembler = video_rtp::VideoRtp::new(1280, 720);
            let now = Instant::now();
            for p in [
                packet(seq, 1000, false, &[0x7c, 0x85, 0x88]),
                packet(seq.wrapping_add(2), 1000, true, &[0x7c, 0x45, 0x99]),
                packet(seq.wrapping_add(1), 99999, true, &[]),
                packet(seq.wrapping_add(3), 2500, true, &[0x61, 0xaa, 0xbb]),
            ] {
                receiver.receive(&mut assembler, p, now);
            }
            receiver.flush(&mut assembler, now);
            assert_eq!(receiver.drops, 0);
            assert!(!receiver.keyframe);
            assert_eq!(
                *receiver.worker.submitted.lock().unwrap(),
                vec![
                    vec![0, 0, 0, 1, 0x65, 0x88, 0x99],
                    vec![0, 0, 0, 1, 0x61, 0xaa, 0xbb]
                ]
            );
        }
    }

    #[test]
    fn padding_cannot_bridge_a_missing_fragment_or_complete_an_unfinished_fu() {
        let mut receiver = OrderedReceiver::default();
        let mut assembler = video_rtp::VideoRtp::new(1280, 720);
        let now = Instant::now();
        receiver.receive(
            &mut assembler,
            packet(10, 1000, false, &[0x7c, 0x85, 0x88]),
            now,
        );
        receiver.receive(&mut assembler, packet(11, 1000, true, &[]), now);
        assert!(receiver.worker.submitted.lock().unwrap().is_empty());
        // Sequence 12 is genuinely absent, not padding we actually received.
        receiver.receive(
            &mut assembler,
            packet(13, 1000, true, &[0x7c, 0x45, 0xaa]),
            now,
        );
        receiver.flush(&mut assembler, now + Duration::from_millis(7));
        receiver.receive(
            &mut assembler,
            packet(14, 2500, true, &[0x61, 0xaa, 0xbb]),
            now + Duration::from_millis(8),
        );
        assert!(receiver.keyframe);
        assert!(receiver.worker.submitted.lock().unwrap().is_empty());
    }

    #[test]
    fn padding_during_unfinished_au_has_a_bounded_storage_limit() {
        let mut assembler = video_rtp::VideoRtp::new(1280, 720);
        let worker = VideoDecodeWorker::default();
        let mut keyframe = false;
        assembler.receive(
            &worker,
            packet(0, 1000, false, &[0x7c, 0x85, 0x88]),
            &mut keyframe,
        );
        for seq in 1..=2100 {
            assembler.receive(&worker, packet(seq, 1000, false, &[]), &mut keyframe);
        }
        assert!(keyframe);
        assert!(worker.submitted.lock().unwrap().is_empty());
        assembler.receive(
            &worker,
            packet(2101, 2500, true, &[0x65, 0x88, 0xaa]),
            &mut keyframe,
        );
        assert_eq!(worker.submitted.lock().unwrap().len(), 1);
    }

    #[test]
    fn thirty_virtual_minutes_preserve_fragment_bytes_through_bursts_and_wraps() {
        let mut receiver = OrderedReceiver::default();
        let mut assembler = video_rtp::VideoRtp::new(1280, 720);
        let start = Instant::now();
        let mut seq = u16::MAX - 17;
        let mut timestamp = u32::MAX - 90_000;
        for frame in 0..108_000_u64 {
            // Eight frames delivered in each 133ms burst. Every frame has an
            // out-of-order end fragment; use actual reorder + H264 assembly.
            // No hardware decode is claimed for these small assembly fixtures.
            let now = start + Duration::from_micros((frame / 8) * 133_333);
            for p in [
                packet(seq, timestamp, false, &[0x7c, 0x85, 0x88]),
                packet(seq.wrapping_add(3), timestamp, true, &[0x7c, 0x45, 0xaa]),
                packet(seq.wrapping_add(1), timestamp, true, &[]),
                packet(seq.wrapping_add(2), timestamp, false, &[0x7c, 0x05, 0x99]),
            ] {
                receiver.receive(&mut assembler, p, now);
            }
            receiver.flush(&mut assembler, now);
            let mut outputs = receiver.worker.submitted.lock().unwrap();
            assert_eq!(*outputs, vec![vec![0, 0, 0, 1, 0x65, 0x88, 0x99, 0xaa]]);
            outputs.clear();
            seq = seq.wrapping_add(4);
            timestamp = timestamp.wrapping_add(1500);
        }
        assert_eq!(receiver.drops, 0);
        assert_eq!(receiver.order.stats.missing, 0);
        assert!(!receiver.keyframe);
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
        let old_drops: u32 = packets
            .iter()
            .cloned()
            .map(|p| old.receive(&worker, p, &mut keyframe).dropped)
            .sum();
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
        assert_eq!(
            *fixed.worker.submitted.lock().unwrap(),
            vec![
                vec![0, 0, 0, 1, 0x65, 0x88, 0x99],
                vec![0, 0, 0, 1, 0x61, 0xaa, 0xbb],
            ]
        );
        assert_eq!(fixed.order.stats.filled, 1);
    }

    #[test]
    fn delayed_pump_processes_available_gap_filler_before_expiring_it() {
        let mut fixed = OrderedReceiver::default();
        let mut assembler = video_rtp::VideoRtp::new(1280, 720);
        let now = Instant::now();
        fixed.receive(
            &mut assembler,
            packet(10, 1000, false, &[0x7c, 0x85, 0x88]),
            now,
        );
        fixed.receive(
            &mut assembler,
            packet(12, 2500, true, &[0x61, 0xaa, 0xbb]),
            now,
        );
        // The next receive batch is processed after the 6ms deadline. Packet 13
        // arrives before the gap filler within this already-received batch.
        let delayed = now + Duration::from_millis(11);
        fixed.receive(
            &mut assembler,
            packet(13, 4000, true, &[0x61, 0xcc, 0xdd]),
            delayed,
        );
        fixed.receive(
            &mut assembler,
            packet(11, 1000, true, &[0x7c, 0x45, 0x99]),
            delayed,
        );
        fixed.flush(&mut assembler, delayed);
        assert_eq!(fixed.drops, 0);
        assert_eq!(fixed.order.stats.missing, 0);
        assert!(!fixed.keyframe);
        assert_eq!(
            *fixed.worker.submitted.lock().unwrap(),
            vec![
                vec![0, 0, 0, 1, 0x65, 0x88, 0x99],
                vec![0, 0, 0, 1, 0x61, 0xaa, 0xbb],
                vec![0, 0, 0, 1, 0x61, 0xcc, 0xdd],
            ]
        );
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
        ] {
            fixed.receive(&mut assembler, p, now);
        }
        assert!(fixed.worker.submitted.lock().unwrap().is_empty());
        fixed.flush(&mut assembler, now + Duration::from_millis(6));
        assert_eq!(fixed.drops, 2);
        assert!(fixed.keyframe);
        assert_eq!(fixed.order.stats.missing, 1);
        assert!(fixed.worker.submitted.lock().unwrap().is_empty());
        // A fragment arriving after the deadline cannot resurrect the old AU.
        fixed.receive(
            &mut assembler,
            packet(11, 1000, false, &[0x7c, 0x05, 0x77]),
            now + Duration::from_millis(7),
        );
        assert_eq!(fixed.worker.submitted.lock().unwrap().len(), 0);
        assert_eq!(fixed.order.stats.too_late, 1);
    }

    #[test]
    fn damage_blocks_dependent_frames_until_a_complete_idr_is_submitted() {
        let mut assembler = video_rtp::VideoRtp::new(1280, 720);
        let worker = VideoDecodeWorker::default();
        let mut request = false;
        assembler.receive(
            &worker,
            packet(10, 1000, false, &[0x7c, 0x81, 0x88]),
            &mut request,
        );
        let damage = assembler.receive(
            &worker,
            packet(12, 2500, true, &[0x61, 0xaa, 0xbb]),
            &mut request,
        );
        assert_eq!(damage.dropped, 2);
        assert!(assembler.waiting_for_keyframe());
        assert!(worker.submitted.lock().unwrap().is_empty());
        let idr = assembler.receive(
            &worker,
            packet(13, 4000, true, &[0x65, 0xaa, 0xbb]),
            &mut request,
        );
        assert_eq!(idr.submitted, 1);
        assert!(!assembler.waiting_for_keyframe());
        let next = assembler.receive(
            &worker,
            packet(14, 5500, true, &[0x61, 0xaa, 0xbb]),
            &mut request,
        );
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
        rtp::Packet {
            header: rtp::header::Header {
                sequence_number: seq,
                timestamp: ts,
                marker,
                ..Default::default()
            },
            payload: Bytes::from_static(bytes),
        }
    }
    #[test]
    fn recovered_tail_keeps_the_h264_reference_chain_instead_of_entering_idr_wait() {
        let t = Instant::now();
        let mut order = reorder::PacketOrder::default();
        order.enable_repair(true);
        let mut video = video_rtp::VideoRtp::new(1280, 720);
        let worker = streaming::video::VideoDecodeWorker::default();
        let mut keyframe = false;
        let first = packet(10, 1000, false, &[0x7c, 0x85, 0x88]);
        let next = packet(12, 2500, true, &[0x61, 0xaa, 0xbb]);
        video.receive(&worker, order.push(10, first, t).unwrap(), &mut keyframe);
        assert!(order.push(12, next, t).is_none());
        assert_eq!(
            order.missing_for_nack(t + Duration::from_millis(2)),
            vec![11]
        );
        assert!(order.pop(t + Duration::from_millis(30)).is_none());
        let tail = packet(11, 1000, true, &[0x7c, 0x45, 0x99]);
        let now = t + Duration::from_millis(46);
        video.receive(&worker, order.push(11, tail, now).unwrap(), &mut keyframe);
        video.receive(&worker, order.pop_ready(now).unwrap(), &mut keyframe);
        assert!(!keyframe);
        assert_eq!(
            *worker.submitted.lock().unwrap(),
            vec![
                vec![0, 0, 0, 1, 0x65, 0x88, 0x99],
                vec![0, 0, 0, 1, 0x61, 0xaa, 0xbb]
            ]
        );
    }

    #[test]
    fn two_second_ingress_age_is_preserved_through_reorder_assembly_and_decoder_submission() {
        let old = Instant::now() - Duration::from_secs(2);
        let mut video = video_rtp::VideoRtp::new(1280, 720);
        let worker = streaming::video::VideoDecodeWorker::default();
        let mut order = reorder::PacketOrder::default();
        let mut keyframe = false;
        // Arrive out of sequence across u16 wrap; the earliest socket timestamp
        // belongs to a later fragment. Completion must preserve the minimum.
        let now = Instant::now();
        let first = order
            .push(
                u16::MAX - 1,
                (
                    packet(u16::MAX - 1, 1234, false, &[0x7c, 0x85, 0x88]),
                    old + Duration::from_millis(10),
                ),
                now,
            )
            .unwrap();
        video.receive_at(&worker, first.0, first.1, &mut keyframe);
        assert!(
            order
                .push(0, (packet(0, 1234, true, &[0x7c, 0x45, 0xaa]), old), now)
                .is_none()
        );
        let middle = order
            .push(
                u16::MAX,
                (
                    packet(u16::MAX, 1234, false, &[0x7c, 0x05, 0x99]),
                    old + Duration::from_millis(20),
                ),
                now,
            )
            .unwrap();
        video.receive_at(&worker, middle.0, middle.1, &mut keyframe);
        let last = order.pop_ready(now).unwrap();
        video.receive_at(&worker, last.0, last.1, &mut keyframe);
        assert_eq!(*worker.submitted_times.lock().unwrap(), vec![(old, 1234)]);
        assert!(!keyframe);
        let summary = video.take_assembly_summary();
        assert!(summary.contains("samples:1"));
    }
    #[test]
    fn manual_refresh_keeps_partial_au_and_playback_until_self_contained_idr() {
        let mut video = video_rtp::VideoRtp::new(1280, 720);
        let worker = streaming::video::VideoDecodeWorker::default();
        let mut keyframe = false;
        video.receive(
            &worker,
            packet(10, 1000, false, &[0x7c, 0x85, 0x88]),
            &mut keyframe,
        );
        // A new decoder already waits for its first IDR (HA04). Refresh is not
        // packet loss: it adds no keyframe wait and keeps the partial AU.
        let waiting = video.waiting_for_keyframe();
        video.refresh();
        assert_eq!(video.waiting_for_keyframe(), waiting);
        video.receive(
            &worker,
            packet(11, 1000, true, &[0x7c, 0x45, 0x99]),
            &mut keyframe,
        );
        assert!(!video.waiting_for_keyframe());
        video.receive(
            &worker,
            packet(12, 2500, true, &[0x61, 0xaa, 0xbb]),
            &mut keyframe,
        );
        // An IDR without its own parameter sets cannot discard queued updates.
        video.receive(
            &worker,
            packet(13, 4000, true, &[0x65, 0xbb, 0xcc]),
            &mut keyframe,
        );
        assert_eq!(
            worker.cutovers.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert_eq!(
            *worker.submitted.lock().unwrap(),
            vec![
                vec![0, 0, 0, 1, 0x65, 0x88, 0x99],
                vec![0, 0, 0, 1, 0x61, 0xaa, 0xbb],
                vec![0, 0, 0, 1, 0x65, 0xbb, 0xcc]
            ]
        );
        // Complete 720p SPS/PPS + IDR at the same RTP timestamp.
        for (seq, marker, nal) in [(14, false, SPS), (15, false, PPS), (16, true, IDR)] {
            video.receive(&worker, packet(seq, 5500, marker, nal), &mut keyframe);
        }
        assert_eq!(
            worker.cutovers.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert!(
            video
                .recovery_summary(Instant::now())
                .contains("Refresh pending:0 IDRadmitted:1")
        );
        video.receive(
            &worker,
            packet(17, 7000, true, &[0x61, 0xcc, 0xdd]),
            &mut keyframe,
        );
        assert_eq!(worker.submitted.lock().unwrap().len(), 5);
        assert!(!keyframe);
        assert!(!video.waiting_for_keyframe());
    }

    #[test]
    fn live_edge_quarantine_requires_current_idr_and_reuses_only_matching_parameter_sets() {
        let worker = streaming::video::VideoDecodeWorker::default();
        let mut video = video_rtp::VideoRtp::new(1280, 720);
        let mut request = false;
        for (seq, marker, nal) in [(10, false, SPS), (11, false, PPS), (12, true, IDR)] {
            video.receive(&worker, packet(seq, 0, marker, nal), &mut request);
        }
        let submitted = worker.submitted.lock().unwrap().len();
        assert_eq!(submitted, 1);
        let now = Instant::now();
        {
            let mut edge = worker.edge.lock().unwrap();
            edge.establish(
                0,
                now - Duration::from_secs(2),
                now - Duration::from_secs(2),
            );
            edge.observe(
                0,
                10,
                now - Duration::from_secs(2),
                now - Duration::from_secs(2),
            );
            assert!(edge.observe(1500, 13, now, now));
        }
        video.quarantine(&worker);
        // A fully assembled old IDR with cached parameters is still stale.
        assert_eq!(
            video
                .receive(&worker, packet(13, 1500, true, IDR), &mut request)
                .submitted,
            0
        );
        worker.edge.lock().unwrap().observe(180_000, 14, now, now);
        assert_eq!(
            video
                .receive(
                    &worker,
                    packet(14, 180_000, true, &[0x61, 0xaa]),
                    &mut request
                )
                .submitted,
            0
        );
        let stats = video.receive(&worker, packet(15, 181_500, true, IDR), &mut request);
        assert_eq!(stats.submitted, 1);
        let bytes = worker.submitted.lock().unwrap().last().unwrap().clone();
        assert!(bytes.windows(SPS.len()).any(|w| w == SPS));
        assert!(bytes.windows(PPS.len()).any(|w| w == PPS));
        assert!(bytes.ends_with(IDR));
        assert_eq!(
            worker.edge.lock().unwrap().state(),
            live_edge::State::AwaitingPicture
        );
        // An unknown PPS ID must not borrow unrelated cached sets.
        video.quarantine(&worker);
        assert_eq!(
            video
                .receive(
                    &worker,
                    packet(16, 183_000, true, &[0x65, 0xb4]),
                    &mut request
                )
                .submitted,
            0
        );
        video.source_changed(&worker);
        video.quarantine(&worker);
        assert_eq!(
            video
                .receive(&worker, packet(17, 184_500, true, IDR), &mut request)
                .submitted,
            0
        );
    }

    // libx264 baseline, 1280x720/60; synthesized black frame parameter sets.
    const SPS: &[u8] = &[
        0x67, 0x42, 0xc0, 0x20, 0xda, 0x01, 0x40, 0x16, 0xec, 0x04, 0x40, 0, 0, 3, 0, 0x40, 0, 0,
        0x1e, 0x23, 0xc6, 0x0c, 0xa8,
    ];
    const PPS: &[u8] = &[0x68, 0xce, 0x0f, 0xc8];
    const IDR: &[u8] = &[0x65, 0xbb, 0xcc]; // slice bytes are opaque to the submission sink

    #[test]
    fn backlog_can_cut_over_at_natural_keyframe_but_not_at_parameter_sets_or_pictures() {
        let mut video = video_rtp::VideoRtp::new(1280, 720);
        let worker = streaming::video::VideoDecodeWorker::default();
        let mut keyframe = false;
        worker
            .queued
            .store(32, std::sync::atomic::Ordering::Relaxed);
        // First AU contains all parameter sets but no IDR. Second is self-contained.
        for (seq, marker, nal) in [
            (10, false, SPS),
            (11, false, PPS),
            (12, true, &[0x61, 0xaa][..]),
        ] {
            video.receive(&worker, packet(seq, 1000, marker, nal), &mut keyframe);
        }
        assert_eq!(
            worker.cutovers.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        for (seq, marker, nal) in [(13, false, SPS), (14, false, IDR), (15, true, PPS)] {
            video.receive(&worker, packet(seq, 2500, marker, nal), &mut keyframe);
        }
        // PPS last in the Annex-B reader must be inspected on completion too.
        assert_eq!(
            worker.cutovers.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert!(!video.waiting_for_keyframe());
    }

    #[test]
    fn damage_during_soft_refresh_still_blocks_dependent_pictures() {
        let mut video = video_rtp::VideoRtp::new(1280, 720);
        let worker = streaming::video::VideoDecodeWorker::default();
        let mut keyframe = false;
        video.refresh();
        video.receive(
            &worker,
            packet(10, 1000, false, &[0x7c, 0x85, 0x88]),
            &mut keyframe,
        );
        video.receive(
            &worker,
            packet(12, 1000, true, &[0x7c, 0x45, 0x99]),
            &mut keyframe,
        );
        video.receive(
            &worker,
            packet(13, 2500, true, &[0x61, 0xaa, 0xbb]),
            &mut keyframe,
        );
        assert!(video.waiting_for_keyframe());
        assert!(keyframe);
        assert!(worker.submitted.lock().unwrap().is_empty());
        for (seq, marker, nal) in [(14, false, SPS), (15, false, PPS), (16, true, IDR)] {
            video.receive(&worker, packet(seq, 4000, marker, nal), &mut keyframe);
        }
        assert!(!video.waiting_for_keyframe());
        assert_eq!(worker.submitted.lock().unwrap().len(), 1);
        assert_eq!(
            worker.cutovers.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }
}

// Convenience belongs to this harness; production supplies socket-dequeue time.
impl video_rtp::VideoRtp {
    fn receive(
        &mut self,
        worker: &streaming::video::VideoDecodeWorker,
        packet: rtp::Packet,
        keyframe: &mut bool,
    ) -> video_rtp::VideoSampleStats {
        self.receive_at(worker, packet, std::time::Instant::now(), keyframe)
    }
}
