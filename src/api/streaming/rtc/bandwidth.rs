//! Receiver video budget used for SDP and subsequent REMB estimates.
//!
//! HA07: 3 Mbps, up from 2 Mbps. The Xbox paces video at roughly its bandwidth
//! estimate, which this ceiling caps, while its encoder overshoots during
//! heavy scenes (2.1-3.7 Mbps in HA06-22 and 2.4-4.4 Mbps in HA04-20, both
//! under a 2 Mbps cap). Below the encoder's rate, a backlog builds until the
//! Xbox's 2 s queue limit forces faster sending, which leaves video about
//! 1.3 s behind. The startup capability hint (`maxBitrateKbps`) stays 2000.
//!
//! HA11: 4 Mbps, up from 3 Mbps. In HA10-26 every lag episode began with video
//! arriving at 3.2-3.45 Mbps (the 3 Mbps cap plus overhead), with video then
//! up to 1.1 s behind audio and no packet lost.
pub(crate) const VIDEO_CEILING_BPS: u32 = 4_000_000;
