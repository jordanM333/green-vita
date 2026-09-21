//! PS Vita hardware H.264 decoder (`sceVideodec`/`sceAvcdec`).
use super::timing::{FrameTiming, PictureTracker, UNKNOWN_PTS};
use std::time::Instant;
use super::memory::{CdramBlock, release_reserved_decoder_cdram};
use super::{DecoderConfig, VideoTextureTarget, metrics};
use anyhow::{Result, bail};
use std::os::raw::c_void;
use vitasdk_sys::*;

// The idea of reducing the reference frames came from MattKC on his Vanilla project
// Make sure to check it out, good content :)
const AVCDEC_NUM_REF_FRAMES: u32 = 1;
// AVCDEC and SDL's Vita GXM renderer both support RGB565 natively. At 960x544 this halves the
// decoder-to-texture traffic from roughly 2 MiB to 1 MiB per frame, which matters at 60 FPS.
const OUTPUT_BYTES_PER_PIXEL: u32 = 2;
const OUTPUT_PIXEL_FORMAT: u32 = SCE_AVCDEC_PIXELFORMAT_RGBA565 as u32;

struct AvcdecLibrary {
    module_loaded: bool,
}

impl AvcdecLibrary {
    fn initialize(width: u32, height: u32) -> Result<Self> {
        let module_loaded = unsafe {
            let loaded_before = sceSysmoduleIsLoaded(SCE_SYSMODULE_AVCDEC);
            let ret = sceSysmoduleLoadModule(SCE_SYSMODULE_AVCDEC);
            if ret >= 0 {
                true
            } else if ret as u32 == SCE_SYSMODULE_ERROR_INVALID_VALUE {
                eprintln!(
                    "sceSysmoduleLoadModule(SCE_SYSMODULE_AVCDEC=0x{SCE_SYSMODULE_AVCDEC:x}) returned {ret:#x}; continuing with SceVideodec imports; is_loaded_before={loaded_before:#x}",
                );
                false
            } else {
                bail!(
                    "sceSysmoduleLoadModule(SCE_SYSMODULE_AVCDEC=0x{SCE_SYSMODULE_AVCDEC:x}) failed: {ret:#x}; is_loaded_before={loaded_before:#x}",
                );
            }
        };

        let init_info = SceVideodecQueryInitInfoHwAvcdec {
            size: size_of::<SceVideodecQueryInitInfoHwAvcdec>() as u32,
            horizontal: width,
            vertical: height,
            numOfRefFrames: AVCDEC_NUM_REF_FRAMES,
            numOfStreams: 1,
        };
        let ret = unsafe { sceVideodecInitLibrary(SCE_VIDEODEC_TYPE_HW_AVCDEC, &init_info) };
        if ret < 0 {
            if module_loaded {
                unsafe {
                    sceSysmoduleUnloadModule(SCE_SYSMODULE_AVCDEC);
                }
            }
            bail!("sceVideodecInitLibrary({width}x{height}) failed: {ret:#x}");
        }

        Ok(Self { module_loaded })
    }
}

impl Drop for AvcdecLibrary {
    fn drop(&mut self) {
        unsafe {
            sceVideodecTermLibrary(SCE_VIDEODEC_TYPE_HW_AVCDEC);
            if self.module_loaded {
                sceSysmoduleUnloadModule(SCE_SYSMODULE_AVCDEC);
            }
        }
    }
}

struct AvcdecDecoder(SceAvcdecCtrl);

impl Drop for AvcdecDecoder {
    fn drop(&mut self) {
        unsafe {
            sceAvcdecDeleteDecoder(&mut self.0);
        }
    }
}

pub(super) struct DecodedPicture {
    pub timing: Option<FrameTiming>,
}

pub struct HwVideoDecoder {
    pictures: PictureTracker,
    decoder: AvcdecDecoder,
    _frame_memory: CdramBlock,
    _library: AvcdecLibrary,
    width: u32,
    height: u32,
    reported_first_picture: bool,
}

impl HwVideoDecoder {
    pub fn new(config: DecoderConfig) -> Result<Self> {
        unsafe {
            // Decoder capacity is the stock 1280x720 setting; Xbox can send a smaller 960x540
            // stream. Keep the library initialization and memory query at the same capacity.
            let library = AvcdecLibrary::initialize(config.decode_width, config.decode_height)?;

            let query = SceAvcdecQueryDecoderInfo {
                horizontal: config.decode_width,
                vertical: config.decode_height,
                numOfRefFrames: AVCDEC_NUM_REF_FRAMES,
            };
            let mut decoder_info = SceAvcdecDecoderInfo { frameMemSize: 0 };
            let ret = sceAvcdecQueryDecoderMemSize(
                SCE_VIDEODEC_TYPE_HW_AVCDEC,
                &query,
                &mut decoder_info,
            );
            if ret < 0 {
                bail!("sceAvcdecQueryDecoderMemSize failed: {ret:#x}");
            }
            release_reserved_decoder_cdram();
            let frame_memory =
                CdramBlock::allocate("xcloud_hw_video_frame", decoder_info.frameMemSize)?;
            let mut decoder_control = SceAvcdecCtrl {
                handle: 0,
                frameBuf: SceAvcdecBuf {
                    pBuf: frame_memory.ptr.cast(),
                    size: decoder_info.frameMemSize,
                },
            };
            let ret =
                sceAvcdecCreateDecoder(SCE_VIDEODEC_TYPE_HW_AVCDEC, &mut decoder_control, &query);
            if ret < 0 {
                bail!("sceAvcdecCreateDecoder failed: {ret:#x}");
            }
            let decoder = AvcdecDecoder(decoder_control);
            eprintln!(
                "Vita AVC decoder initialized: capacity {}x{}, output {}x{}, frame memory {} bytes",
                config.decode_width,
                config.decode_height,
                config.output_width,
                config.output_height,
                decoder_info.frameMemSize,
            );

            Ok(Self {
                pictures: PictureTracker::default(),
                decoder,
                _frame_memory: frame_memory,
                _library: library,
                width: config.output_width,
                height: config.output_height,
                reported_first_picture: false,
            })
        }
    }

    /// An output is matched only by the PTS returned in the picture, never by call order.
    pub(super) fn decode(
        &mut self,
        access_unit: &[u8],
        direct_target: VideoTextureTarget,
        rtp_timestamp: u32,
        received_at: Instant,
        submitted_at: Instant,
        epoch: u64,
    ) -> Result<Option<DecodedPicture>> {
        let pts = self.pictures.submit(rtp_timestamp, received_at, submitted_at, epoch);
        self.decode_output(access_unit, pts, Some(rtp_timestamp), direct_target)
    }

    /// Service buffered output without admitting another AU or ending the stream.
    /// Vita FFmpeg uses null/zero ES input with sceAvcdecDecode when its input
    /// buffer is full. This is NOT DecodeStop, a flush, or a decoder recreation.
    pub(super) fn poll(&mut self, target: VideoTextureTarget) -> Result<Option<DecodedPicture>> {
        self.decode_output(&[], UNKNOWN_PTS, None, target)
    }

    fn decode_output(
        &mut self,
        access_unit: &[u8],
        pts: u64,
        submission: Option<u32>,
        direct_target: VideoTextureTarget,
    ) -> Result<Option<DecodedPicture>> {
        unsafe {
            let au = SceAvcdecAu {
                // Vita FFmpeg uses the same 90 kHz PTS units and reads info.pts
                // from the resulting picture. DTS stays unknown.
                pts: SceVideodecTimeStamp {
                    upper: (pts >> 32) as u32,
                    lower: pts as u32,
                },
                dts: SceVideodecTimeStamp {
                    upper: 0xFFFFFFFF,
                    lower: 0xFFFFFFFF,
                },
                es: SceAvcdecBuf {
                    pBuf: if access_unit.is_empty() { std::ptr::null_mut() }
                        else { access_unit.as_ptr() as *mut c_void },
                    size: access_unit.len() as u32,
                },
            };

            let output_ptr = direct_target.ptr as *mut u8;
            let output_pitch = direct_target.pitch / OUTPUT_BYTES_PER_PIXEL;
            let output_capacity = direct_target.capacity;
            if output_pitch < self.width {
                bail!(
                    "direct video texture pitch {output_pitch} is smaller than {}",
                    self.width
                );
            }

            let mut picture = SceAvcdecPicture {
                size: size_of::<SceAvcdecPicture>() as u32,
                frame: SceAvcdecFrame {
                    pixelType: OUTPUT_PIXEL_FORMAT,
                    framePitch: output_pitch,
                    frameWidth: self.width,
                    frameHeight: self.height,
                    horizontalSize: self.width,
                    verticalSize: self.height,
                    frameCropLeftOffset: 0,
                    frameCropRightOffset: 0,
                    frameCropTopOffset: 0,
                    frameCropBottomOffset: 0,
                    opt: SceAvcdecFrameOption {
                        rgba: SceAvcdecFrameOptionRGBA {
                            alpha: 0xff,
                            cscCoefficient: 0,
                            reserved: [0; 14],
                        },
                    },
                    pPicture: [output_ptr.cast(), std::ptr::null_mut()],
                },
                info: std::mem::zeroed(),
            };
            // Do not mistake an untouched zero-initialized field for RTP timestamp zero.
            picture.info.pts.upper = u32::MAX;
            picture.info.pts.lower = u32::MAX;
            let mut picture_ptr: *mut SceAvcdecPicture = &mut picture;
            let mut array_picture = SceAvcdecArrayPicture {
                numOfOutput: 0,
                numOfElm: 1,
                pPicture: &mut picture_ptr,
            };

            let ret = sceAvcdecDecode(&self.decoder.0, &au, &mut array_picture);
            if ret < 0 {
                bail!("sceAvcdecDecode failed: {ret:#x}");
            }
            if array_picture.numOfOutput == 0 {
                return Ok(None);
            }
            if !self.reported_first_picture {
                eprintln!(
                    "Vita AVC first picture: frame {}x{}, visible {}x{}, pitch {} pixels, output surface {}x{}",
                    picture.frame.frameWidth,
                    picture.frame.frameHeight,
                    picture.frame.horizontalSize,
                    picture.frame.verticalSize,
                    picture.frame.framePitch,
                    self.width,
                    self.height,
                );
                self.reported_first_picture = true;
            }

            // `framePitch` is in pixels. Validate the complete byte size before trusting the
            // decoder-provided pitch to write into the SDL texture.
            let row_bytes = picture
                .frame
                .framePitch
                .checked_mul(OUTPUT_BYTES_PER_PIXEL)
                .ok_or_else(|| {
                    anyhow::anyhow!("video frame pitch overflow: {}", picture.frame.framePitch)
                })?;
            let output_len = row_bytes.checked_mul(self.height).ok_or_else(|| {
                anyhow::anyhow!("video output size overflow: {row_bytes} * {}", self.height)
            })?;
            if output_len > output_capacity {
                bail!(
                    "sceAvcdecDecode produced pitch requiring {output_len} bytes, but output buffer has {} bytes",
                    output_capacity
                );
            }
            metrics::METRICS.picture_dimensions.store(
                (u64::from(picture.frame.frameWidth) << 32)
                    | u64::from(picture.frame.frameHeight),
                std::sync::atomic::Ordering::Relaxed,
            );
            let output_pts = (u64::from(picture.info.pts.upper) << 32)
                | u64::from(picture.info.pts.lower);
            let timing = self.pictures.output(output_pts, Instant::now());
            super::trace::record(if submission.is_some() { "decoder_output_pts" }
                else { "decoder_poll_output_pts" }, submission.unwrap_or(0), output_pts);
            if timing.is_none() {
                super::trace::record("decoder_pts_unmatched", submission.unwrap_or(0),
                    u64::from(output_pts == UNKNOWN_PTS));
            }
            Ok(Some(DecodedPicture { timing }))
        }
    }
}

// SAFETY: the CDRAM blocks and decoder handle have no thread affinity in the underlying SCE API -
// this is only ever moved once (into `VideoDecodeWorker`'s thread), never accessed concurrently.
unsafe impl Send for HwVideoDecoder {}
