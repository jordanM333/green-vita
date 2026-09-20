// Compile the production interceptor against exactly the shipped RTC components,
// without linking the Vita-only application or mocking report generation.
extern crate self as rtc;
pub use {interceptor, rtp, rtcp, shared, sansio};
#[path = "../../../src/api/streaming/rtc/reports.rs"]
mod reports;
#[path = "../../../src/api/streaming/rtc/clock.rs"]
mod clock;
#[path = "../../../src/streaming/video/freshness.rs"]
pub(crate) mod freshness;
pub(crate) mod streaming { pub(crate) mod video { pub(crate) use crate::freshness; } }
