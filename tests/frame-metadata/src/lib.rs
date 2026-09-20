#![allow(dead_code)]
#[path = "../../../src/streaming/input.rs"]
pub(crate) mod input_state;
#[path = "../../../src/streaming/video/timing.rs"]
pub(crate) mod timing;
mod streaming {
    pub(crate) use crate::input_state as input;
    pub(crate) mod video { pub(crate) use crate::timing; }
}
#[path = "../../../src/api_xbox/streaming/control/input.rs"]
mod input_packet;
