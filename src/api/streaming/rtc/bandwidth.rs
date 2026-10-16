//! Receiver video budget used for SDP and subsequent REMB estimates.
//!
//! HA07: 3 Mbps, up from 2 Mbps. The Xbox paces video at roughly its bandwidth
//! estimate, which this ceiling caps, while its encoder overshoots during
//! heavy scenes (2.1-3.7 Mbps in HA06-22 and 2.4-4.4 Mbps in HA04-20, both
//! under a 2 Mbps cap). Below the encoder's rate, a backlog builds until the
//! Xbox's 2 s queue limit forces faster sending, which leaves video about
//! 1.3 s behind. The startup capability hint (`maxBitrateKbps`) stays 2000.
//!
//! HA11 tried 4 Mbps; HA11-27 lost audio and video for 8-17 s at a time once
//! video arrived above ~3.7 Mbps (Order lost 4633, ICE RTT over 1 s). HA12
//! returns to 3 Mbps.
pub(crate) const VIDEO_CEILING_BPS: u32 = 3_000_000;
