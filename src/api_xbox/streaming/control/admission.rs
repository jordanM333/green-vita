//! Bound ephemeral input before it enters WebRTC/SCTP's unbounded pending queue.
//! Includes unacknowledged bytes, so even a stalled SCTP congestion window cannot
//! accumulate seconds of reports. Presentation feedback leaves room for input.
pub const INPUT_OUTSTANDING_LIMIT: usize = 256;
pub const FEEDBACK_OUTSTANDING_LIMIT: usize = 128;
