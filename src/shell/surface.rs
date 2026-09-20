use crate::app::StreamingSession;
use crate::shell::egui_painter::SdlEguiPainter;
use crate::streaming::video::{CdramBlock, DirectVideoOutput, VideoTextureTarget};
use anyhow::{Context, Result};
use sdl2::pixels::PixelFormatEnum;
use sdl2::render::{Canvas, Texture};
use sdl2::video::Window;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

pub const WIDTH: u32 = 960;
pub const HEIGHT: u32 = 544;

pub struct VitaSurface {
    pub(crate) canvas: Canvas<Window>,
    video_textures: Option<Vec<Texture>>,
    video_output_buffers: Option<Vec<CdramBlock>>,
    displayed_video_texture: Option<usize>,
    direct_video_output: Option<Arc<DirectVideoOutput>>,
    video_width: u32,
    video_height: u32,
    egui_painter: SdlEguiPainter,
}

impl VitaSurface {
    pub fn new(video: &sdl2::VideoSubsystem) -> Result<Self> {
        let window = video
            .window("GreenVita", WIDTH, HEIGHT)
            .position_centered()
            .build()
            .context("failed to create SDL Vita window")?;
        let mut canvas = window
            .into_canvas()
            .accelerated()
            .build()
            .map_err(anyhow::Error::msg)
            .context("failed to create SDL Vita renderer")?;
        canvas
            .set_logical_size(WIDTH, HEIGHT)
            .map_err(anyhow::Error::msg)
            .context("failed to set Vita logical render size")?;

        Ok(Self {
            canvas,
            video_textures: None,
            video_output_buffers: None,
            displayed_video_texture: None,
            direct_video_output: None,
            video_width: 0,
            video_height: 0,
            egui_painter: SdlEguiPainter::default(),
        })
    }

    /// Where the video quad lands on screen - accounts for letterboxing, see `fit_rect`.
    pub fn video_rect(&self) -> sdl2::rect::Rect {
        Self::fit_rect(self.video_width, self.video_height, WIDTH, HEIGHT)
    }

    pub fn has_pending_video_frame(&self) -> bool {
        self.direct_video_output
            .as_ref()
            .is_some_and(|output| output.has_pending_frame())
    }

    pub fn has_displayed_video_frame(&self) -> bool {
        self.displayed_video_texture.is_some()
    }

    pub fn sync_video_frame(&mut self, streaming: Option<&StreamingSession>) -> Result<()> {
        let Some(streaming) = streaming else {
            self.detach_direct_video_output();
            return Ok(());
        };
        self.ensure_direct_video_output(streaming)?;

        // The decoder publishes a completed CDRAM frame directly to this shared output. The
        // frame handle that travels through RTC and app mailboxes can already be obsolete.
        let Some((index, target)) = self
            .direct_video_output
            .as_ref()
            .and_then(|output| output.take_latest_for_display())
        else {
            return Ok(());
        };
        if index >= self.video_textures.as_ref().map_or(0, Vec::len) {
            anyhow::bail!("decoder returned invalid direct texture index {index}");
        }
        // Copy a complete decoder output into the renderer's texture under its SDL lock.
        // The decoder never writes to the texture that a queued GPU scene can still sample.
        let texture = &mut self.video_textures.as_mut().expect("textures registered")[index];
        let upload_started = Instant::now();
        texture
            .with_lock(None, |pixels, pitch| {
                let width_bytes = (self.video_width as usize) * 2;
                let height = self.video_height as usize;
                let source_pitch = target.pitch as usize;
                let source_len = source_pitch
                    .checked_mul(height)
                    .context("decoder output length overflow")?;
                let destination_len = pitch
                    .checked_mul(height)
                    .context("SDL video texture length overflow")?;
                if source_pitch < width_bytes
                    || pitch < width_bytes
                    || (target.capacity as usize) < source_len
                    || pixels.len() < destination_len
                {
                    anyhow::bail!("decoder output does not fit SDL video texture");
                }
                if source_pitch == pitch {
                    // A DMA copy keeps the non-cached decoder output off the CPU copy path.
                    let result = unsafe {
                        vitasdk_sys::sceDmacMemcpy(
                            pixels.as_mut_ptr().cast(),
                            target.ptr as *const std::ffi::c_void,
                            source_len as u32,
                        )
                    };
                    if result >= 0 {
                        return Ok(());
                    }
                }
                let source = unsafe {
                    std::slice::from_raw_parts(target.ptr as *const u8, source_len)
                };
                for row in 0..height {
                    let dst = &mut pixels[row * pitch..(row + 1) * pitch];
                    dst[..width_bytes]
                        .copy_from_slice(&source[row * source_pitch..row * source_pitch + width_bytes]);
                    dst[width_bytes..].fill(0);
                }
                Ok(())
            })
            .map_err(anyhow::Error::msg)
            .context("failed to lock SDL video texture")??;
        let upload_us = upload_started.elapsed().as_micros() as u64;
        let metrics = &crate::streaming::video::metrics::METRICS;
        metrics.video_upload_sum_us.fetch_add(upload_us, Ordering::Relaxed);
        metrics.video_upload_count.fetch_add(1, Ordering::Relaxed);
        metrics.video_upload_max_us.fetch_max(upload_us, Ordering::Relaxed);
        self.displayed_video_texture = Some(index);
        crate::streaming::video::metrics::METRICS
            .presented
            .fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn ensure_direct_video_output(&mut self, streaming: &StreamingSession) -> Result<()> {
        let output = streaming.direct_video_output();
        let output_is_current = self
            .direct_video_output
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &output));
        if !output_is_current && self.direct_video_output.is_some() {
            self.detach_direct_video_output();
        }
        if !output.decoder_ready.load(Ordering::Acquire) {
            return Ok(());
        }
        if output_is_current && self.video_textures.is_some() {
            return Ok(());
        }

        self.detach_direct_video_output();
        let (width, height) = (output.width, output.height);
        let create_texture = || {
            self.canvas
                .create_texture_streaming(PixelFormatEnum::BGR565, width, height)
                .map_err(anyhow::Error::msg)
                .context("failed to create direct SDL BGR565 video texture")
        };
        let mut textures = vec![create_texture()?, create_texture()?];
        match create_texture() {
            Ok(spare) => textures.push(spare),
            Err(error) => eprintln!("No spare video texture ({error:#}); using two textures"),
        }
        let mut buffers = Vec::with_capacity(textures.len());
        let mut targets = Vec::with_capacity(textures.len());
        for index in 0..textures.len() {
            let texture = &mut textures[index];
            // Keep the CDRAM decode pitch equal to the SDL texture pitch so display uses one
            // DMA transfer. Do not retain a pointer returned by SDL after this lock ends.
            let pitch = texture
                .with_lock(None, |pixels, pitch| {
                    pixels.fill(0);
                    pitch
                })
                .map_err(anyhow::Error::msg)
                .context("failed to measure SDL video texture pitch")?;
            let capacity = pitch
                .checked_mul(height as usize)
                .and_then(|bytes| u32::try_from(bytes).ok())
                .context("video decoder output size overflow")?;
            let buffer = match CdramBlock::allocate(&format!("xcloud_video_out_{index}"), capacity)
            {
                Ok(buffer) => buffer,
                Err(error) if index >= 2 => {
                    eprintln!("No spare decoder output buffer ({error:#}); using two textures");
                    textures.truncate(2);
                    break;
                }
                Err(error) => return Err(error),
            };
            // AVCDEC writes only the visible region; clear any padding before SDL uploads it.
            unsafe { std::ptr::write_bytes(buffer.ptr, 0, capacity as usize) };
            targets.push(VideoTextureTarget {
                ptr: buffer.ptr as usize,
                pitch: pitch as u32,
                capacity,
            });
            buffers.push(buffer);
        }
        output.set_targets(targets);
        self.video_output_buffers = Some(buffers);
        self.video_textures = Some(textures);
        self.displayed_video_texture = None;
        self.direct_video_output = Some(output);
        self.video_width = width;
        self.video_height = height;
        Ok(())
    }

    fn detach_direct_video_output(&mut self) {
        if let Some(output) = self.direct_video_output.take() {
            output.clear_targets();
        }
        self.video_output_buffers = None;
        self.video_textures = None;
        self.displayed_video_texture = None;
        self.video_width = 0;
        self.video_height = 0;
    }

    pub fn draw_scene(&mut self, show_video: bool) -> Result<()> {
        self.canvas.set_draw_color(sdl2::pixels::Color::BLACK);
        self.canvas.clear();

        if show_video
            && let Some(index) = self.displayed_video_texture
            && let Some(texture) = self
                .video_textures
                .as_ref()
                .map(|textures| &textures[index])
        {
            let destination = self.video_rect();
            self.canvas
                .copy(texture, None, destination)
                .map_err(anyhow::Error::msg)
                .context("failed to draw SDL YUV video frame")?;
        }

        Ok(())
    }

    pub fn paint_egui(
        &mut self,
        pixels_per_point: f32,
        primitives: &[egui::ClippedPrimitive],
        textures_delta: &egui::TexturesDelta,
    ) -> Result<()> {
        let paint_started = Instant::now();
        self.egui_painter.paint(
            &mut self.canvas,
            [WIDTH, HEIGHT],
            pixels_per_point,
            primitives,
            textures_delta,
        )?;
        let egui_us = paint_started.elapsed().as_micros() as u64;
        self.canvas.present();
        crate::streaming::video::trace::record("present_return", 0,
            paint_started.elapsed().as_micros() as u64);
        let paint_us = paint_started.elapsed().as_micros() as u64;
        let metrics = &crate::streaming::video::metrics::METRICS;
        metrics.egui_draw_sum_us.fetch_add(egui_us, Ordering::Relaxed);
        metrics.egui_draw_max_us.fetch_max(egui_us, Ordering::Relaxed);
        let present_us = paint_us.saturating_sub(egui_us);
        metrics.render_present_sum_us.fetch_add(present_us, Ordering::Relaxed);
        metrics.render_present_max_us.fetch_max(present_us, Ordering::Relaxed);
        metrics.paint_sum_us.fetch_add(paint_us, Ordering::Relaxed);
        metrics.paint_count.fetch_add(1, Ordering::Relaxed);
        metrics.paint_max_us.fetch_max(paint_us, Ordering::Relaxed);
        Ok(())
    }

    fn fit_rect(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> sdl2::rect::Rect {
        if src_w == 0 || src_h == 0 {
            return sdl2::rect::Rect::new(0, 0, dst_w, dst_h);
        }
        let src_aspect = src_w as f32 / src_h as f32;
        let dst_aspect = dst_w as f32 / dst_h as f32;
        if src_aspect > dst_aspect {
            let height = (dst_w as f32 / src_aspect).round() as u32;
            let y = ((dst_h - height) / 2) as i32;
            sdl2::rect::Rect::new(0, y, dst_w, height)
        } else {
            let width = (dst_h as f32 * src_aspect).round() as u32;
            let x = ((dst_w - width) / 2) as i32;
            sdl2::rect::Rect::new(x, 0, width, dst_h)
        }
    }
}
