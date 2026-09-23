//! Negotiate arrival feedback, using the same receiver as the RTC default stack.
//! This adds no media buffering. The sender chooses how to react to the reports.
use rtc::interceptor::{Registry, TwccReceiverBuilder, TwccReceiverInterceptor};
use rtc::peer_connection::configuration::media_engine::MediaEngine;
use rtc::rtp_transceiver::rtp_sender::{RTCPFeedback, RTCRtpHeaderExtensionCapability, RtpCodecKind};
use super::reports::ReceiveReports;

pub(crate) use super::reports::TRANSPORT_CC_URI;
pub(crate) type ReceiveFeedback = TwccReceiverInterceptor<ReceiveReports>;

pub(crate) fn configure(media: &mut MediaEngine, clocks: Vec<(u8, u32)>)
    -> rtc::shared::error::Result<Registry<ReceiveFeedback>>
{
    for kind in [RtpCodecKind::Video, RtpCodecKind::Audio] {
        media.register_feedback(RTCPFeedback {
            typ: "transport-cc".into(), ..Default::default()
        }, kind);
        media.register_header_extension(RTCRtpHeaderExtensionCapability {
            uri: TRANSPORT_CC_URI.into(),
        }, kind, None)?;
    }
    // TWCC must be outside ReceiveReports: later negotiated bindings must reach
    // TWCC even when the RR fallback already knows the SSRC's clock rate.
    Ok(Registry::new()
        .with(move |inner| ReceiveReports::new(inner, clocks))
        .with(TwccReceiverBuilder::new().build()))
}
