#[path = "../../../src/streaming/voice_encoder.rs"]
mod voice_encoder;
#[cfg(test)]
mod tests {
    use super::voice_encoder::VoiceEncoder;
    use std::ffi::{c_int, c_void};
    #[link(name = "opus")]
    unsafe extern "C" {
        fn opus_decoder_create(fs: c_int, channels: c_int, error: *mut c_int) -> *mut c_void;
        fn opus_decode(decoder: *mut c_void, data: *const u8, len: c_int, pcm: *mut i16, frame_size: c_int, decode_fec: c_int) -> c_int;
        fn opus_decoder_destroy(decoder: *mut c_void);
        fn opus_packet_get_nb_samples(data: *const u8, len: c_int, fs: c_int) -> c_int;
    }
    #[test]
    fn native_opus_voice_roundtrip_has_20ms_duration_and_non_silent_audio() {
        let mut encoder=VoiceEncoder::new().unwrap();
        let mut error=0;
        let decoder=unsafe { opus_decoder_create(48000, 2, &mut error) };
        assert_eq!(error,0); assert!(!decoder.is_null());
        let mut decoded=[0i16;1920];
        let mut peak=0;
        for frame in 0..10 {
            let samples=std::array::from_fn(|i| (((frame*320+i) as f32 * 440.0 * std::f32::consts::TAU / 16000.0).sin()*8000.0) as i16);
            let packet=encoder.encode(&samples).unwrap();
            assert!(packet.len() <= 400);
            assert_eq!(unsafe { opus_packet_get_nb_samples(packet.as_ptr(),packet.len() as c_int,48000) },960);
            assert_eq!(unsafe { opus_decode(decoder,packet.as_ptr(),packet.len() as c_int,decoded.as_mut_ptr(),960,0) },960);
            peak=peak.max(decoded.iter().map(|s|i32::from(*s).abs()).max().unwrap());
        }
        unsafe { opus_decoder_destroy(decoder); }
        assert!(peak > 1000, "encoded voice was silent");
    }
}
