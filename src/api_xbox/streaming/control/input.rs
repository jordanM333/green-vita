//! Wire format for xCloud's "input" data channel: gamepad + pointer report packets.

use crate::streaming::input::{GamepadFrame, PointerEvent};
use crate::streaming::video::timing::PresentedFrame;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ReportType {
    None = 0,
    Metadata = 1,
    Gamepad = 2,
    Pointer = 4,
    ClientMetadata = 8,
    ServerMetadata = 16,
}

#[derive(Debug, Clone, Default)]
pub struct PointerFrame {
    pub events: Vec<PointerEvent>,
}

#[derive(Debug, Clone)]
pub struct InputPacket {
    report_type: u16,
    total_size: usize,
    sequence: u32,
    gamepad_frames: Vec<GamepadFrame>,
    pointer_frames: Vec<PointerFrame>,
    max_touchpoints: u8,
    timestamp_ms: f64,
}

impl InputPacket {
    pub fn new(sequence: u32, timestamp_ms: f64) -> Self {
        Self {
            report_type: ReportType::None as u16,
            total_size: 14,
            sequence,
            gamepad_frames: Vec::new(),
            pointer_frames: Vec::new(),
            max_touchpoints: 0,
            timestamp_ms,
        }
    }

    pub fn client_metadata(sequence: u32, max_touchpoints: u8, timestamp_ms: f64) -> Self {
        let mut packet = Self::new(sequence, timestamp_ms);
        packet.report_type = ReportType::ClientMetadata as u16;
        packet.total_size = 15;
        packet.max_touchpoints = max_touchpoints;
        packet
    }

    pub fn set_data(&mut self, gamepads: Vec<GamepadFrame>, pointers: Vec<PointerFrame>) {
        let mut size = 14;
        if !gamepads.is_empty() {
            self.report_type |= ReportType::Gamepad as u16;
            size += 1 + 23 * gamepads.len();
        }
        if !pointers.is_empty() {
            self.report_type |= ReportType::Pointer as u16;
            size += 1 + pointers
                .iter()
                .map(|frame| 1 + frame.events.len() * 20)
                .sum::<usize>();
        }

        self.total_size = size;
        self.gamepad_frames = gamepads;
        self.pointer_frames = pointers;
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.total_size);
        push_le(&mut bytes, self.report_type.to_le_bytes());
        push_le(&mut bytes, self.sequence.to_le_bytes());
        bytes.extend_from_slice(&self.timestamp_ms.to_le_bytes());

        if !self.gamepad_frames.is_empty() {
            self.write_gamepads(&mut bytes);
        }
        if !self.pointer_frames.is_empty() {
            self.write_pointers(&mut bytes);
        }
        if self.report_type == ReportType::ClientMetadata as u16 {
            bytes.push(self.max_touchpoints);
        }

        debug_assert_eq!(bytes.len(), self.total_size);
        bytes
    }

    fn write_gamepads(&self, bytes: &mut Vec<u8>) {
        bytes.push(self.gamepad_frames.len() as u8);
        for input in &self.gamepad_frames {
            bytes.push(input.gamepad_index);
            let mut mask = 0u16;
            let buttons = [
                (input.nexus, 2),
                (input.menu, 4),
                (input.view, 8),
                (input.a, 16),
                (input.b, 32),
                (input.x, 64),
                (input.y, 128),
                (input.dpad_up, 256),
                (input.dpad_down, 512),
                (input.dpad_left, 1024),
                (input.dpad_right, 2048),
                (input.left_shoulder, 4096),
                (input.right_shoulder, 8192),
                (input.left_thumb, 16384),
                (input.right_thumb, 32768),
            ];
            for (value, bit) in buttons {
                if value > 0.0 {
                    mask |= bit;
                }
            }
            push_le(bytes, mask.to_le_bytes());
            push_le(bytes, normalize_axis(input.left_thumb_x_axis).to_le_bytes());
            push_le(
                bytes,
                normalize_axis(-input.left_thumb_y_axis).to_le_bytes(),
            );
            push_le(
                bytes,
                normalize_axis(input.right_thumb_x_axis).to_le_bytes(),
            );
            push_le(
                bytes,
                normalize_axis(-input.right_thumb_y_axis).to_le_bytes(),
            );
            push_le(bytes, normalize_trigger(input.left_trigger).to_le_bytes());
            push_le(bytes, normalize_trigger(input.right_trigger).to_le_bytes());
            push_le(bytes, 1u32.to_le_bytes());
            bytes.extend_from_slice(&1u32.to_be_bytes());
        }
    }

    fn write_pointers(&self, bytes: &mut Vec<u8>) {
        bytes.push(1);
        if let Some(frame) = self.pointer_frames.first() {
            bytes.push(frame.events.len() as u8);
            for event in &frame.events {
                push_le(bytes, event.contact_major.to_le_bytes());
                push_le(bytes, event.contact_minor.to_le_bytes());
                bytes.push(event.pressure);
                push_le(bytes, event.twist.to_le_bytes());
                push_le(bytes, 0u32.to_le_bytes());
                push_le(bytes, event.x.to_le_bytes());
                push_le(bytes, event.y.to_le_bytes());
                bytes.push(event.event_type);
            }
        }
    }
}

fn normalize_trigger(value: f32) -> u16 {
    if value < 0.0 {
        0
    } else {
        (65535.0 * value).clamp(0.0, 65535.0) as u16
    }
}

fn normalize_axis(value: f32) -> i16 {
    let max = 32767.0;
    (value * max).clamp(-max, max) as i16
}

fn push_le<const N: usize>(bytes: &mut Vec<u8>, raw: [u8; N]) {
    bytes.extend_from_slice(&raw);
}

/// Batches queued frames into [`InputPacket`]s, one wire packet per send.
#[derive(Debug)]
pub struct InputQueue {
    started_at: Instant,
    sequence: u32,
    gamepads: Vec<GamepadFrame>,
    pointers: Vec<PointerFrame>,
}

impl Default for InputQueue {
    fn default() -> Self {
        Self {
            started_at: Instant::now(),
            sequence: 0,
            gamepads: Vec::new(),
            pointers: Vec::new(),
        }
    }
}

impl InputQueue {
    pub(crate) fn rendered_frame_packet(&mut self, frame: PresentedFrame, now: Instant) -> Option<Vec<u8>> {
        let timing = frame.timing;
        // A delayed RTC pump must not transmit old presentation reports. All
        // fields share the exact monotonic origin used by the input header.
        if timing.received_at < self.started_at || timing.submitted_at < timing.received_at
            || timing.decoded_at < timing.submitted_at || frame.rendered_at < timing.decoded_at
            || now < frame.rendered_at
            || now.duration_since(frame.rendered_at) > std::time::Duration::from_millis(250)
        { return None; }
        let millis = |at: Instant| at.duration_since(self.started_at).as_millis() as u32;
        let values = [timing.rtp_timestamp, millis(timing.received_at), millis(timing.submitted_at),
            millis(timing.decoded_at), millis(frame.rendered_at), millis(now), millis(now)];
        let timestamp_ms = now.duration_since(self.started_at).as_secs_f64() * 1_000.0;
        let mut bytes = Vec::with_capacity(43);
        bytes.extend_from_slice(&(ReportType::Metadata as u16).to_le_bytes());
        bytes.extend_from_slice(&self.next_sequence().to_le_bytes());
        bytes.extend_from_slice(&timestamp_ms.to_le_bytes());
        bytes.push(1);
        for value in values { bytes.extend_from_slice(&value.to_le_bytes()); }
        Some(bytes)
    }

    fn timestamp_ms(&self) -> f64 {
        // Xbox's f64 input timestamp is a monotonic uptime in milliseconds. Measuring
        // from each packet's creation made every report appear to arrive at time zero.
        self.started_at.elapsed().as_secs_f64() * 1_000.0
    }

    pub fn queue_gamepad_frames(
        &mut self,
        frames: impl IntoIterator<Item = GamepadFrame>,
        force_send: bool,
    ) -> Option<Vec<u8>> {
        self.gamepads.extend(frames);
        self.check_queue_and_packet(force_send)
    }

    pub fn queue_pointer_frame(&mut self, frame: PointerFrame) -> Option<Vec<u8>> {
        if let Some(existing) = self.pointers.first_mut() {
            existing.events.extend(frame.events);
        } else {
            self.pointers.push(frame);
        }
        self.check_queue_and_packet(true)
    }

    pub fn client_metadata_packet(&mut self, max_touchpoints: u8) -> Vec<u8> {
        let timestamp_ms = self.timestamp_ms();
        InputPacket::client_metadata(self.next_sequence(), max_touchpoints, timestamp_ms).to_bytes()
    }

    fn check_queue_and_packet(&mut self, force_send: bool) -> Option<Vec<u8>> {
        let should_send = force_send || !self.gamepads.is_empty() || !self.pointers.is_empty();
        should_send.then(|| self.drain_packet())
    }

    fn drain_packet(&mut self) -> Vec<u8> {
        let timestamp_ms = self.timestamp_ms();
        let mut packet = InputPacket::new(self.next_sequence(), timestamp_ms);
        packet.set_data(
            std::mem::take(&mut self.gamepads),
            std::mem::take(&mut self.pointers),
        );
        packet.to_bytes()
    }

    fn next_sequence(&mut self) -> u32 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn timestamp(bytes: &[u8]) -> f64 {
        f64::from_le_bytes(bytes[6..14].try_into().unwrap())
    }

    #[test]
    fn input_timestamps_advance_from_a_shared_clock() {
        let mut queue = InputQueue::default();
        queue.started_at -= Duration::from_secs(3);
        let metadata = queue.client_metadata_packet(0);
        let gamepad = queue.queue_gamepad_frames([GamepadFrame::default()], true).unwrap();
        assert!((3_000.0..4_000.0).contains(&timestamp(&metadata)));
        assert!(timestamp(&gamepad) >= timestamp(&metadata));
        assert_eq!(u32::from_le_bytes(gamepad[2..6].try_into().unwrap()), 2);
    }

    #[test]
    fn rendered_frame_report_matches_xbox_wire_layout_and_input_clock() {
        use crate::streaming::video::timing::FrameTiming;
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut queue = InputQueue::default();
        queue.started_at = start;
        queue.client_metadata_packet(0);
        let frame = PresentedFrame { timing: FrameTiming {
            rtp_timestamp: 0xf1234567, received_at: at(1000), submitted_at: at(1002),
            decoded_at: at(1010), epoch: 0,
        }, rendered_at: at(1017) };
        let bytes = queue.rendered_frame_packet(frame, at(1020)).unwrap();
        assert_eq!(bytes.len(), 43);
        assert_eq!(&bytes[0..6], &[1, 0, 2, 0, 0, 0]);
        assert_eq!(timestamp(&bytes), 1020.0);
        assert_eq!(bytes[14], 1);
        let fields: Vec<_> = bytes[15..].chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
        assert_eq!(fields, [0xf1234567, 1000, 1002, 1010, 1017, 1020, 1020]);
        let gamepad = queue.queue_gamepad_frames([GamepadFrame::default()], true).unwrap();
        assert_eq!(u32::from_le_bytes(gamepad[2..6].try_into().unwrap()), 3);
    }

    #[test]
    fn stale_or_invalid_frame_reports_are_discarded_without_consuming_a_sequence() {
        use crate::streaming::video::timing::FrameTiming;
        let mut queue = InputQueue::default();
        let start = queue.started_at;
        let mut frame = PresentedFrame { timing: FrameTiming { rtp_timestamp: 0,
            received_at: start, submitted_at: start, decoded_at: start, epoch: 0,
        }, rendered_at: start };
        assert!(queue.rendered_frame_packet(frame, start + Duration::from_secs(1)).is_none());
        frame.timing.decoded_at += Duration::from_millis(1);
        assert!(queue.rendered_frame_packet(frame, start).is_none());
        assert_eq!(queue.sequence, 0);
    }

    #[test]
    fn metadata_milliseconds_wrap_without_losing_rtp_identity() {
        use crate::streaming::video::timing::FrameTiming;
        let mut queue = InputQueue::default();
        let now = queue.started_at + Duration::from_millis(u64::from(u32::MAX) + 2);
        let frame = PresentedFrame { timing: FrameTiming { rtp_timestamp: 0,
            received_at: now, submitted_at: now, decoded_at: now, epoch: 0,
        }, rendered_at: now };
        let bytes = queue.rendered_frame_packet(frame, now).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[15..19].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(bytes[19..23].try_into().unwrap()), 1);
    }
}
