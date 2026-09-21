use crate::streaming::video::{SubmitResult, VideoDecodeWorker};
use crate::streaming::video::policy::Recovery;
use bytes::Bytes;
use h264_reader::annexb::AnnexBReader;
use h264_reader::nal::sps::SeqParameterSet;
use h264_reader::nal::{Nal, RefNal, UnitType};
use h264_reader::push::NalInterest;
use rtc::rtp::Packet;
use rtc::rtp::codec::h264::H264Packet;
use rtc::rtp::codec::opus::OpusPacket;
use rtc::rtp::packetizer::Depacketizer;
use rtc_media::io::sample_builder::SampleBuilder;
use std::sync::atomic::Ordering;
use std::time::Instant;

const MAX_PENDING_AUDIO_PACKETS: usize = 32;
const MAX_H264_ACCESS_UNIT_BYTES: usize = 2 * 1024 * 1024;
const VIDEO_RTP_CLOCK_RATE: u32 = 90_000;
const AUDIO_MAX_LATE_PACKETS: u16 = 32;

#[derive(Default)]
pub(super) struct VideoSampleStats {
    pub dropped: u32,
    pub assembled: u32,
    pub submitted: u32,
    pub missing_sequence: u32,
    pub incomplete_fua: u32,
    pub malformed: u32,
    pub queue_full: u32,
    pub sps_rejected: u32,
    pub idr_wait: u32,
    pub other: u32,
    pub sequence_jumps: u32,
    pub estimated_missing_packets: u32,
    pub late_packets: u32,
    pub duplicate_packets: u32,
    pub empty_packets: u32,
    pub idr: u32,
    pub post_damage_submitted: u32,
    pub source_frame_duration_us: Option<u64>,
    pub encoded_resolution: Option<(u32, u32)>,
}

#[derive(Clone, Copy)]
enum DropReason {
    MissingSequence,
    IncompleteFuA,
    Malformed,
    QueueFull,
    SpsRejected,
    IdrWait,
    Other,
}

impl VideoSampleStats {
    fn record_drop(&mut self, reason: DropReason) {
        crate::streaming::video::trace::record("au_drop_reason", 0, reason as u64);
        self.dropped = self.dropped.saturating_add(1);
        let counter = match reason {
            DropReason::MissingSequence => &mut self.missing_sequence,
            DropReason::IncompleteFuA => &mut self.incomplete_fua,
            DropReason::Malformed => &mut self.malformed,
            DropReason::QueueFull => &mut self.queue_full,
            DropReason::SpsRejected => &mut self.sps_rejected,
            DropReason::IdrWait => &mut self.idr_wait,
            DropReason::Other => &mut self.other,
        };
        *counter = (*counter).saturating_add(1);
    }
}

pub(super) struct AudioRtp {
    samples: SampleBuilder<OpusPacket>,
    payload_type: u8,
    sample_rate: u32,
    last_sequence: Option<u16>,
    latest_timestamp: Option<u32>,
}

impl AudioRtp {
    pub(super) fn new(sample_rate: u32, payload_type: u8) -> Self {
        Self {
            samples: SampleBuilder::new(AUDIO_MAX_LATE_PACKETS, OpusPacket, sample_rate)
                .with_max_time_delay(std::time::Duration::from_millis(80)),
            payload_type,
            sample_rate,
            last_sequence: None,
            latest_timestamp: None,
        }
    }

    pub(super) fn receive(&mut self, packet: Packet, audio_packets: &mut Vec<Bytes>) {
        if packet.header.payload_type != self.payload_type {
            return;
        }

        let sequence = packet.header.sequence_number;
        if let Some(previous) = self.last_sequence {
            let forward = sequence.wrapping_sub(previous);
            if forward > 0 && forward < (1 << 15) {
                if forward > 1 {
                    crate::streaming::video::metrics::METRICS.audio_rtp_gaps.fetch_add(
                        u64::from(forward - 1), Ordering::Relaxed,
                    );
                }
                self.last_sequence = Some(sequence);
            } else if forward >= (1 << 15) {
                crate::streaming::video::metrics::METRICS.audio_rtp_late
                    .fetch_add(1, Ordering::Relaxed);
            }
        } else {
            self.last_sequence = Some(sequence);
        }

        let timestamp = packet.header.timestamp;
        if self.latest_timestamp.is_none_or(|latest| {
            let forward = timestamp.wrapping_sub(latest);
            forward > 0 && forward < (1 << 31)
        }) {
            self.latest_timestamp = Some(timestamp);
        }

        self.samples.push(packet);
        while audio_packets.len() < MAX_PENDING_AUDIO_PACKETS {
            let Some(sample) = self.samples.pop() else {
                break;
            };
            if let Some(latest) = self.latest_timestamp {
                let lead = latest.wrapping_sub(sample.packet_timestamp);
                if lead < (1 << 31) && self.sample_rate > 0 {
                    crate::streaming::video::metrics::METRICS.audio_rtp_backlog_ms.store(
                        u64::from(lead) * 1_000 / u64::from(self.sample_rate),
                        Ordering::Relaxed,
                    );
                }
            }
            let dropped = sample
                .prev_dropped_packets
                .saturating_sub(sample.prev_padding_packets);
            crate::streaming::video::metrics::METRICS.audio_rtp_lost.fetch_add(
                u64::from(dropped), Ordering::Relaxed,
            );
            audio_packets.push(sample.data);
        }
    }
}

pub(super) struct VideoRtp {
    depacketizer: H264Packet,
    pending: Option<PendingVideoFrame>,
    next_sequence: Option<u16>,
    last_frame_timestamp: Option<u32>,
    source_frame_duration_us: Option<u64>,
    stream_too_large: bool,
    recovery: Recovery,
    decoder_capacity: (u32, u32),
    last_sps_resolution: Option<(u32, u32)>,
    last_arrival_sequence: Option<u16>,
    last_idr_at: Option<Instant>,
    suspect_reference: bool,
}

struct PendingVideoFrame {
    timestamp: u32,
    first_packet_at: Instant,
    packets: Vec<Packet>,
    marker: Option<u16>,
    bytes: usize,
}

enum FrameAssembly {
    Pending,
    Complete { data: Bytes, marker_sequence: u16 },
    Invalid(DropReason),
}

impl PendingVideoFrame {
    fn new(packet: Packet) -> Self {
        let marker = packet.header.marker.then_some(packet.header.sequence_number);
        let bytes = packet.payload.len();
        Self {
            timestamp: packet.header.timestamp,
            first_packet_at: Instant::now(),
            packets: vec![packet],
            marker,
            bytes,
        }
    }

    fn insert(&mut self, packet: Packet) -> bool {
        // PacketOrder has already emitted strictly increasing sequence numbers.
        // Do not scan and sort an ever-growing AU on every arriving fragment.
        if self.packets.last().is_none_or(|previous| {
            let forward = packet.header.sequence_number.wrapping_sub(previous.header.sequence_number);
            forward > 0 && forward < (1 << 15)
        }) {
            self.bytes = self.bytes.saturating_add(packet.payload.len());
            if packet.header.marker { self.marker = Some(packet.header.sequence_number); }
            self.packets.push(packet);
            true
        } else {
            false
        }
    }

    fn marker_sequence(&self) -> Option<u16> {
        self.marker
    }

    fn abandonment_reason(&self, expected_sequence: Option<u16>) -> DropReason {
        if let Some(marker_sequence) = self.marker_sequence() {
            let mut sequences = self
                .packets
                .iter()
                .map(|packet| packet.header.sequence_number)
                .collect::<Vec<_>>();
            sequences.sort_unstable_by_key(|sequence| {
                std::cmp::Reverse(marker_sequence.wrapping_sub(*sequence))
            });
            if expected_sequence.is_some_and(|expected| sequences.first() != Some(&expected))
                || sequences
                    .windows(2)
                    .any(|pair| pair[1] != pair[0].wrapping_add(1))
            {
                return DropReason::MissingSequence;
            }
        }
        let fu_start = self.packets.iter().any(|packet| {
            packet.payload.first().is_some_and(|header| header & 0x1f == 28)
                && packet.payload.get(1).is_some_and(|flags| flags & 0x80 != 0)
        });
        let fu_end = self.packets.iter().any(|packet| {
            packet.payload.first().is_some_and(|header| header & 0x1f == 28)
                && packet.payload.get(1).is_some_and(|flags| flags & 0x40 != 0)
        });
        if (fu_start && !fu_end)
            || (!fu_start
                && self.packets.iter().any(|packet| {
                    packet.payload.first().is_some_and(|header| header & 0x1f == 28)
                }))
        {
            DropReason::IncompleteFuA
        } else {
            DropReason::MissingSequence
        }
    }

    fn assemble(
        &self,
        depacketizer: &mut H264Packet,
        expected_sequence: Option<u16>,
    ) -> FrameAssembly {
        let Some(marker_sequence) = self.marker_sequence() else {
            return FrameAssembly::Pending;
        };
        let packets = self.packets.iter().collect::<Vec<_>>();
        let Some(first) = packets.first() else {
            return FrameAssembly::Pending;
        };
        if expected_sequence.is_some_and(|expected| first.header.sequence_number != expected)
            || !depacketizer.is_partition_head(&first.payload)
        {
            return FrameAssembly::Pending;
        }
        if packets.windows(2).any(|pair| {
            pair[1].header.sequence_number != pair[0].header.sequence_number.wrapping_add(1)
        }) {
            return FrameAssembly::Pending;
        }

        if let Err(reason) = validate_h264_rtp_fragments(&packets) {
            return FrameAssembly::Invalid(reason);
        }

        *depacketizer = H264Packet::default();
        let mut data = Vec::new();
        for packet in packets {
            let Ok(nalu) = depacketizer.depacketize(&packet.payload) else {
                *depacketizer = H264Packet::default();
                return FrameAssembly::Invalid(DropReason::Malformed);
            };
            data.extend_from_slice(&nalu);
            if data.len() > MAX_H264_ACCESS_UNIT_BYTES {
                *depacketizer = H264Packet::default();
                return FrameAssembly::Invalid(DropReason::Other);
            }
        }
        *depacketizer = H264Packet::default();
        FrameAssembly::Complete {
            data: Bytes::from(data),
            marker_sequence,
        }
    }
}

// The rtc-rs depacketizer buffers FU-A bytes until an end fragment but does not require a
// start fragment or reject a frame ending mid-FU. Check both before sending an AU to AVCDEC.
fn validate_h264_rtp_fragments(packets: &[&Packet]) -> Result<(), DropReason> {
    let mut fragmented: Option<(u8, u8)> = None;
    for packet in packets {
        let payload = &packet.payload;
        if payload.len() < 2 {
            return Err(DropReason::Malformed);
        }
        match payload[0] & 0x1f {
            1..=23 => {
                if fragmented.is_some() {
                    return Err(DropReason::IncompleteFuA);
                }
            }
            24 => {
                if fragmented.is_some() {
                    return Err(DropReason::IncompleteFuA);
                }
                let mut offset = 1;
                while offset < payload.len() {
                    if offset + 2 > payload.len() {
                        return Err(DropReason::Malformed);
                    }
                    let size = (usize::from(payload[offset]) << 8)
                        | usize::from(payload[offset + 1]);
                    offset += 2;
                    if size == 0
                        || offset + size > payload.len()
                        || !(1..=23).contains(&(payload[offset] & 0x1f))
                    {
                        return Err(DropReason::Malformed);
                    }
                    offset += size;
                }
            }
            28 => {
                if payload.len() < 3 {
                    return Err(DropReason::Malformed);
                }
                let flags = payload[1];
                let start = flags & 0x80 != 0;
                let end = flags & 0x40 != 0;
                let fragment_type = flags & 0x1f;
                if flags & 0x20 != 0 || (start && end) || !(1..=23).contains(&fragment_type) {
                    return Err(DropReason::Malformed);
                }
                let key = (payload[0] & 0xe0, fragment_type);
                if start {
                    if fragmented.replace(key).is_some() {
                        return Err(DropReason::IncompleteFuA);
                    }
                } else if fragmented != Some(key) {
                    return Err(DropReason::IncompleteFuA);
                }
                if end {
                    fragmented = None;
                }
            }
            _ => return Err(DropReason::Malformed),
        }
    }
    if fragmented.is_some() {
        Err(DropReason::IncompleteFuA)
    } else {
        Ok(())
    }
}

impl VideoRtp {
    pub(super) fn new(decode_width: u32, decode_height: u32) -> Self {
        Self {
            depacketizer: H264Packet::default(),
            pending: None,
            next_sequence: None,
            last_frame_timestamp: None,
            source_frame_duration_us: None,
            stream_too_large: false,
            recovery: Recovery::default(),
            decoder_capacity: (decode_width, decode_height),
            last_sps_resolution: None,
            last_arrival_sequence: None,
            last_idr_at: None,
            suspect_reference: false,
        }
    }

    pub(super) fn waiting_for_keyframe(&self) -> bool {
        self.recovery.waiting()
    }

    pub(super) fn recover_latency(&mut self, worker: &VideoDecodeWorker) {
        self.pending = None;
        self.next_sequence = None;
        self.depacketizer = H264Packet::default();
        self.recovery.damage();
        self.suspect_reference = true;
        worker.flush_before_next_idr();
    }

    pub(super) fn recovery_summary(&self, now: Instant) -> String {
        self.recovery.summary(now)
    }

    pub(super) fn recover_decoder(&mut self, worker: &VideoDecodeWorker) -> bool {
        if worker.take_recovery_request() {
            self.record_damage(worker);
            return true;
        }
        false
    }

    pub(super) fn idr_age_secs(&self) -> Option<u64> {
        self.last_idr_at.map(|at| at.elapsed().as_secs())
    }

    pub(super) fn receive(
        &mut self,
        worker: &VideoDecodeWorker,
        packet: Packet,
        keyframe_requested: &mut bool,
    ) -> VideoSampleStats {
        self.receive_at(worker, packet, Instant::now(), keyframe_requested)
    }

    pub(super) fn receive_at(
        &mut self,
        worker: &VideoDecodeWorker,
        packet: Packet,
        received_at: Instant,
        keyframe_requested: &mut bool,
    ) -> VideoSampleStats {
        let mut stats = VideoSampleStats::default();
        if worker.take_recovery_request() {
            self.record_damage(worker);
            *keyframe_requested = true;
        }
        let sequence = packet.header.sequence_number;
        let mut out_of_order = false;
        if let Some(previous) = self.last_arrival_sequence {
            let distance = sequence.wrapping_sub(previous);
            if distance == 0 {
                stats.duplicate_packets = 1;
                return stats;
            }
            if distance < (1 << 15) {
                if distance > 1 {
                    // Arrival jumps can also be reordered packets, so these are provisional.
                    stats.sequence_jumps = 1;
                    stats.estimated_missing_packets = u32::from(distance - 1);
                }
                self.last_arrival_sequence = Some(sequence);
            } else {
                out_of_order = true;
                stats.late_packets = 1;
            }
        } else {
            self.last_arrival_sequence = Some(sequence);
        }
        if packet.payload.is_empty() {
            stats.empty_packets = 1;
            if self.next_sequence == Some(packet.header.sequence_number) {
                self.next_sequence = Some(packet.header.sequence_number.wrapping_add(1));
            }
            return stats;
        }

        let packet_timestamp = packet.header.timestamp;
        if let Some(pending) = &self.pending
            && pending.timestamp != packet_timestamp
        {
            if !timestamp_is_newer(packet_timestamp, pending.timestamp) {
                if !out_of_order {
                    stats.late_packets = 1;
                }
                return stats;
            }
            if let Some(incomplete) = self.pending.take() {
                let reason = incomplete.abandonment_reason(self.next_sequence);
                crate::streaming::video::trace::record("au_abandon", incomplete.timestamp, reason as u64);
                self.last_frame_timestamp = Some(incomplete.timestamp);
                self.next_sequence = incomplete
                    .marker_sequence()
                    .map(|sequence| sequence.wrapping_add(1));
                self.depacketizer = H264Packet::default();
                *keyframe_requested = true;
                self.record_damage(worker);
                self.suspect_reference = true;
                stats.record_drop(reason);
            }
        }
        if self.pending.is_none() {
            if self
                .last_frame_timestamp
                .is_some_and(|last| !timestamp_is_newer(packet_timestamp, last))
            {
                if !out_of_order {
                    stats.late_packets = 1;
                }
                return stats;
            }
            let mut pending = PendingVideoFrame::new(packet);
            pending.first_packet_at = received_at;
            crate::streaming::video::trace::record("au_first", pending.timestamp, 0);
            self.pending = Some(pending);
        } else if let Some(pending) = &mut self.pending {
            pending.first_packet_at = pending.first_packet_at.min(received_at);
            if !pending.insert(packet) {
                stats.duplicate_packets = 1;
                return stats;
            }
        }

        // Bound an AU even if its marker never arrives. The old byte limit ran
        // only after complete assembly, allowing an unlimited pending packet list.
        if self.pending.as_ref().is_some_and(|p| {
            p.bytes > MAX_H264_ACCESS_UNIT_BYTES || p.packets.len() > 2048
        }) {
            if let Some(pending) = self.pending.take() {
                self.last_frame_timestamp = Some(pending.timestamp);
            }
            self.next_sequence = None;
            self.record_damage(worker);
            *keyframe_requested = true;
            stats.record_drop(DropReason::Other);
            return stats;
        }

        let assembly = self
            .pending
            .as_ref()
            .map(|pending| pending.assemble(&mut self.depacketizer, self.next_sequence));
        let Some(assembly) = assembly else {
            return stats;
        };
        let (data, marker_sequence) = match assembly {
            FrameAssembly::Pending => return stats,
            FrameAssembly::Invalid(reason) => {
                if let Some(invalid) = self.pending.take() {
                    self.last_frame_timestamp = Some(invalid.timestamp);
                    self.next_sequence = invalid
                        .marker_sequence()
                        .map(|sequence| sequence.wrapping_add(1));
                }
                self.depacketizer = H264Packet::default();
                *keyframe_requested = true;
                self.record_damage(worker);
                self.suspect_reference = true;
                stats.record_drop(reason);
                return stats;
            }
            FrameAssembly::Complete {
                data,
                marker_sequence,
            } => (data, marker_sequence),
        };
        stats.assembled = 1;
        let completed = self.pending.take().expect("assembled pending video frame");
        crate::streaming::video::trace::record("au_complete", completed.timestamp,
            completed.first_packet_at.elapsed().as_micros() as u64);
        // Record both average and worst-case RTP assembly time for the stream HUD.
        let assembly_us = completed.first_packet_at.elapsed().as_micros() as u64;
        crate::streaming::video::metrics::METRICS
            .rtp_assembly_sum_us
            .fetch_add(assembly_us, Ordering::Relaxed);
        crate::streaming::video::metrics::METRICS
            .rtp_assembly_count
            .fetch_add(1, Ordering::Relaxed);
        crate::streaming::video::metrics::METRICS
            .rtp_assembly_max_us
            .fetch_max(assembly_us, Ordering::Relaxed);
        self.next_sequence = Some(marker_sequence.wrapping_add(1));
        stats.source_frame_duration_us = self.last_frame_timestamp.map(|previous| {
            u64::from(completed.timestamp.wrapping_sub(previous)) * 1_000_000
                / u64::from(VIDEO_RTP_CLOCK_RATE)
        });
        if let Some(duration) = stats.source_frame_duration_us {
            self.source_frame_duration_us = Some(
                self.source_frame_duration_us
                    .map(|average| (average * 7 + duration) / 8)
                    .unwrap_or(duration),
            );
        }
        self.last_frame_timestamp = Some(completed.timestamp);

        let unit = inspect_h264_access_unit(&data);
        if unit.has_idr {
            stats.idr = 1;
            self.last_idr_at = Some(Instant::now());
        }
        stats.encoded_resolution = unit.resolution;
        if let Some((width, height)) = unit.resolution
            && self.last_sps_resolution != unit.resolution
        {
            eprintln!(
                "H264 SPS detected: {width}x{height}; Vita decoder capacity: {}x{}",
                self.decoder_capacity.0, self.decoder_capacity.1
            );
            self.last_sps_resolution = unit.resolution;
        }
        // Bound incoming SPS by hardware capacity, independently of Xbox's requested size.
        let sample_too_large = unit
            .resolution
            .is_some_and(|(width, height)| {
                width > self.decoder_capacity.0 || height > self.decoder_capacity.1
            });
        if sample_too_large {
            if !self.stream_too_large {
                eprintln!(
                    "Dropping H264 access unit {:?} beyond Vita decoder capacity {}x{}",
                    unit.resolution, self.decoder_capacity.0, self.decoder_capacity.1
                );
            }
            self.stream_too_large = true;
            // Flush queued decoder work once, then wait for a compatible IDR instead of feeding
            // frames that the Vita hardware cannot decode.
            *keyframe_requested = true;
            self.record_damage(worker);
            self.suspect_reference = true;
            stats.record_drop(DropReason::SpsRejected);
            return stats;
        }
        if self.stream_too_large {
            if unit.resolution.is_none() || !unit.has_idr {
                *keyframe_requested = true;
                self.record_damage(worker);
                stats.record_drop(DropReason::IdrWait);
                return stats;
            }
            self.stream_too_large = false;
        }
        if !self.recovery.accepts(unit.has_idr) {
            *keyframe_requested = true;
            stats.record_drop(DropReason::IdrWait);
            return stats;
        }

        match worker.submit_access_unit(data.to_vec(), completed.first_packet_at, completed.timestamp) {
            SubmitResult::Submitted => {
                if unit.has_idr && self.recovery.waiting() {
                    crate::streaming::video::trace::record("recovery_end_ms", completed.timestamp,
                        self.recovery.wait_ms(Instant::now()));
                }
                self.recovery.submitted(unit.has_idr);
                stats.submitted = 1;
                if unit.has_idr {
                    self.suspect_reference = false;
                } else if self.suspect_reference {
                    stats.post_damage_submitted = 1;
                }
            }
            SubmitResult::QueueFull => {
                self.record_damage(worker);
                *keyframe_requested = true;
                self.suspect_reference = true;
                stats.record_drop(DropReason::QueueFull);
            }
            SubmitResult::Disconnected => {
                self.record_damage(worker);
                *keyframe_requested = true;
                self.suspect_reference = true;
                stats.record_drop(DropReason::Other);
            }
        }
        stats
    }

    fn record_damage(&mut self, worker: &VideoDecodeWorker) {
        if self.recovery.damage() {
            crate::streaming::video::trace::record("recovery_begin", 0, 0);
            worker.begin_resync();
        }
    }
}

fn timestamp_is_newer(candidate: u32, reference: u32) -> bool {
    let distance = candidate.wrapping_sub(reference);
    distance != 0 && distance < (1 << 31)
}

struct AccessUnitInfo {
    has_idr: bool,
    resolution: Option<(u32, u32)>,
}

fn inspect_h264_access_unit(data: &[u8]) -> AccessUnitInfo {
    let mut info = AccessUnitInfo {
        has_idr: false,
        resolution: None,
    };
    let mut reader = AnnexBReader::accumulate(|nal: RefNal<'_>| {
        let Ok(header) = nal.header() else {
            return NalInterest::Ignore;
        };
        match header.nal_unit_type() {
            UnitType::SliceLayerWithoutPartitioningIdr => {
                info.has_idr = true;
                NalInterest::Ignore
            }
            UnitType::SeqParameterSet => {
                if nal.is_complete() {
                    info.resolution = SeqParameterSet::from_bits(nal.rbsp_bits())
                        .and_then(|sps| sps.pixel_dimensions())
                        .ok();
                }
                NalInterest::Buffer
            }
            _ => NalInterest::Ignore,
        }
    });
    reader.push(data);
    reader.reset();
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(sequence: u16, marker: bool, payload: &'static [u8]) -> Packet {
        let mut packet = Packet::default();
        packet.header.sequence_number = sequence;
        packet.header.marker = marker;
        packet.header.timestamp = 12_345;
        packet.payload = Bytes::from_static(payload);
        packet
    }

    #[test]
    fn complete_fu_a_reconstructs_one_nal() {
        let mut pending = PendingVideoFrame::new(packet(10, false, &[0x7c, 0x85, 0x88, 0x99]));
        pending.insert(packet(11, true, &[0x7c, 0x45, 0xaa]));
        let mut depacketizer = H264Packet::default();
        match pending.assemble(&mut depacketizer, Some(10)) {
            FrameAssembly::Complete { data, marker_sequence } => {
                assert_eq!(marker_sequence, 11);
                assert_eq!(&data[..], &[0, 0, 0, 1, 0x65, 0x88, 0x99, 0xaa]);
            }
            _ => panic!("a complete fragmented NAL should assemble"),
        }
    }

    #[test]
    fn completed_frame_with_unfinished_fu_a_is_rejected() {
        let mut pending = PendingVideoFrame::new(packet(10, false, &[0x7c, 0x85, 0x88]));
        pending.insert(packet(11, true, &[0x61, 0x99, 0xaa]));
        assert!(matches!(
            pending.assemble(&mut H264Packet::default(), Some(10)),
            FrameAssembly::Invalid(DropReason::IncompleteFuA)
        ));
    }

    #[test]
    fn truncated_stap_a_length_is_rejected_before_depacketizing() {
        let pending = PendingVideoFrame::new(packet(
            10,
            true,
            &[0x78, 0, 3, 0x67, 0x42, 0xe0, 0],
        ));
        assert!(matches!(
            pending.assemble(&mut H264Packet::default(), None),
            FrameAssembly::Invalid(DropReason::Malformed)
        ));
    }
}
