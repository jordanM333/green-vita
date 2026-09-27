//! Mono voice encoder, 16 kHz PCM -> one 20 ms Opus packet.
use anyhow::{Result, ensure};
use std::ffi::{c_int, c_void};
#[cfg_attr(target_os = "vita", link(name = "opus", kind = "static"))]
#[cfg_attr(not(target_os = "vita"), link(name = "opus"))]
unsafe extern "C" {
    fn opus_encoder_create(
        fs: c_int,
        channels: c_int,
        application: c_int,
        error: *mut c_int,
    ) -> *mut c_void;
    fn opus_encoder_ctl(encoder: *mut c_void, request: c_int, ...) -> c_int;
    fn opus_encode(
        encoder: *mut c_void,
        pcm: *const i16,
        frame_size: c_int,
        data: *mut u8,
        max_data_bytes: c_int,
    ) -> c_int;
    fn opus_encoder_destroy(encoder: *mut c_void);
}
pub(crate) struct VoiceEncoder(*mut c_void);
impl VoiceEncoder {
    pub(crate) fn new() -> Result<Self> {
        let mut error = 0;
        // SAFETY: fixed supported mono/16 kHz VOIP parameters and a live c_int error output.
        let raw = unsafe { opus_encoder_create(16000, 1, 2048, &mut error) }; // OPUS_APPLICATION_VOIP
        ensure!(!raw.is_null(), "create failed ({error})");
        let encoder = Self(raw);
        ensure!(error == 0, "create failed ({error})");
        for (request, value) in [(4002, 16000), (4010, 1)] {
            // bitrate, complexity
            // SAFETY: raw is checked non-null and owned by encoder. These fixed
            // OPUS_SET_BITRATE/OPUS_SET_COMPLEXITY requests take one c_int argument.
            let result = unsafe { opus_encoder_ctl(raw, request, value as c_int) };
            ensure!(result == 0, "configure failed ({result})");
        }
        Ok(encoder)
    }
    pub(crate) fn encode(&mut self, samples: &[i16; 320]) -> Result<Vec<u8>> {
        let mut packet = vec![0; 400];
        // SAFETY: exclusive live encoder, exactly 320 initialized mono samples,
        // and 400 initialized writable bytes; libopus retains neither slice.
        let size = unsafe {
            opus_encode(
                self.0,
                samples.as_ptr(),
                320,
                packet.as_mut_ptr(),
                packet.len() as c_int,
            )
        };
        ensure!(size > 0, "encode failed ({size})");
        packet.truncate(size as usize);
        Ok(packet)
    }
}
impl Drop for VoiceEncoder {
    fn drop(&mut self) {
        // SAFETY: sole successfully created encoder owner; no encode call is in flight.
        unsafe {
            opus_encoder_destroy(self.0);
        }
    }
}
