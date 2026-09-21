//! Bound unsent WebRTC ingress, independently of payload already on the wire.
//! SCTP's real cwnd/rwnd govern in-flight payload; counting ACK-waiting bytes
//! against these small application limits starves fresh input and feedback.
pub const INPUT_INGRESS_LIMIT: usize = 256;
pub const FEEDBACK_INGRESS_LIMIT: usize = 128;
