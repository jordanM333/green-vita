//! Runs the production streaming overlay and painter through SDL software rendering.
//! Session/protocol state is supplied by a fixture; Vita GXM/AVCDEC are not emulated.
#![allow(dead_code)]
#[path = "../../../src/app/ui/fonts.rs"]
mod fonts;
#[path = "../../../src/streaming/mic_button.rs"]
pub mod mic_button;
#[path = "../../../src/shell/egui_painter.rs"]
mod painter;
#[path = "../../../src/shell/texture.rs"]
mod texture;
mod shell {
    pub(crate) use crate::texture;
}
#[path = "../../../src/app/ui/screens/streaming.rs"]
mod streaming_screen;
#[path = "../../../src/app/ui/theme.rs"]
pub mod theme;

mod app {
    pub mod ui {
        pub use crate::theme;
        pub mod widgets {
            pub fn draw_hold_progress_ring(_: &mut egui::Ui, _: f32) {}
        }
    }
    pub mod command {
        pub enum NavigationCommand {
            OpenPauseOverlay,
        }
    }
}
mod paused_overlay {
    pub enum Command {
        PressGuideButton,
    }
}
struct AppCommand;
impl From<paused_overlay::Command> for AppCommand {
    fn from(_: paused_overlay::Command) -> Self {
        Self
    }
}
impl From<app::command::NavigationCommand> for AppCommand {
    fn from(_: app::command::NavigationCommand) -> Self {
        Self
    }
}
mod i18n {
    pub struct I18n;
    impl I18n {
        pub fn new(_: bool) -> Self {
            Self
        }
        pub fn text(&self, key: &str) -> String {
            key.replace('-', " ")
        }
    }
}
mod build_info {
    pub const NUMBER: &str = "HA02-17";
}
mod streaming {
    pub use crate::mic_button;
    pub mod video {
        pub mod live_edge {
            #[derive(Clone, Copy)]
            pub enum State {
                Unmeasured,
                Live,
                AwaitingKeyframe,
                AwaitingPicture,
                ClockUncertain,
            }
        }
        pub mod metrics {
            use std::sync::atomic::AtomicU64;
            pub struct Metrics {
                pub audio_sdl_queue_ms: AtomicU64,
                pub local_button_mask: AtomicU64,
            }
            pub static METRICS: Metrics = Metrics {
                audio_sdl_queue_ms: AtomicU64::new(0),
                local_button_mask: AtomicU64::new(0),
            };
        }
    }
}
struct App {
    settings: Settings,
    state: AppState,
}
struct Settings {
    locale: bool,
    show_stream_debug_info: bool,
}
enum AppState {
    Streaming(Session),
    Other,
}
struct Session {
    hint_started_at: std::time::Instant,
    status: String,
    media_reconnecting: bool,
    media_refresh_failed: bool,
    video_startup: Help,
    microphone: Microphone,
    edge: streaming::video::live_edge::State,
    held: bool,
}
struct Help;
impl Help {
    fn needs_help(&self, _: std::time::Instant) -> bool {
        false
    }
}
struct Microphone;
impl Microphone {
    fn is_on(&self) -> bool {
        false
    }
    fn is_active(&self) -> bool {
        false
    }
    fn available(&self) -> bool {
        true
    }
    fn set_on(&self, _: bool) {}
}
impl Session {
    fn direct_video_output(&self) -> &Self {
        self
    }
    fn live_edge_state(&self) -> streaming::video::live_edge::State {
        self.edge
    }
    fn video_held(&self, _: std::time::Instant) -> bool {
        self.held
    }
    fn can_refresh(&self) -> bool {
        false
    }
    fn measured_delay_ms(&self) -> Option<u64> {
        Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_held_frame_draws_no_indicator() {
        // HA09: no banner or indicator over the game while video is held or
        // late; the last good picture simply stays on screen.
        let sdl = sdl2::init().unwrap();
        let window = sdl
            .video()
            .unwrap()
            .window("presentation test", 960, 544)
            .hidden()
            .build()
            .unwrap();
        let mut canvas = window.into_canvas().software().build().unwrap();
        let mut painter = painter::SdlEguiPainter::default();
        // Production sets only the native scale (shell/mod.rs UI_SCALE).
        let ctx = egui::Context::default();
        fonts::configure(&ctx);
        let mut render = |held: bool| {
            let app = App {
                settings: Settings {
                    locale: false,
                    show_stream_debug_info: false,
                },
                state: AppState::Streaming(Session {
                    hint_started_at: std::time::Instant::now() - std::time::Duration::from_secs(60),
                    status: String::new(),
                    media_reconnecting: false,
                    media_refresh_failed: false,
                    video_startup: Help,
                    microphone: Microphone,
                    edge: streaming::video::live_edge::State::AwaitingKeyframe,
                    held,
                }),
            };
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960.0 / 1.3, 544.0 / 1.3),
                )),
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(1.3);
            let out = ctx.run(input, |ctx| {
                streaming_screen::show(ctx, &app, None, &mut Vec::new())
            });
            let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
            canvas.set_draw_color(sdl2::pixels::Color::RGB(80, 160, 240));
            canvas.clear();
            painter
                .paint(
                    &mut canvas,
                    [960, 544],
                    out.pixels_per_point,
                    &primitives,
                    &out.textures_delta,
                )
                .unwrap();
            canvas
                .read_pixels(None, sdl2::pixels::PixelFormatEnum::RGB24)
                .unwrap()
        };
        // Settle font uploads and the buttons' fade-in so the two frames
        // differ only by the label.
        for _ in 0..30 {
            render(false);
        }
        let live = render(false);
        let held = render(true);
        let changed: Vec<usize> = live
            .as_chunks::<3>()
            .0
            .iter()
            .zip(held.as_chunks::<3>().0)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(index, _)| index)
            .collect();
        if let Ok(dir) = std::env::var("GREENVITA_PRESENTATION_OUTPUT") {
            std::fs::create_dir_all(&dir).unwrap();
            image::save_buffer(format!("{dir}/held.png"), &held, 960, 544, image::ColorType::Rgb8)
                .unwrap();
        }
        assert!(changed.is_empty(), "{} pixels drawn over a held picture", changed.len());
    }

    #[test]
    fn live_play_leaves_the_top_of_the_picture_clear() {
        // HA08: no build label or capture status over the game.
        use streaming::video::live_edge::State;
        let sdl = sdl2::init().unwrap();
        let window = sdl
            .video()
            .unwrap()
            .window("presentation test", 960, 544)
            .hidden()
            .build()
            .unwrap();
        let mut canvas = window.into_canvas().software().build().unwrap();
        let mut painter = painter::SdlEguiPainter::default();
        let ctx = egui::Context::default();
        fonts::configure(&ctx);
        let mut render = |edge: State| {
            let app = App {
                settings: Settings {
                    locale: false,
                    show_stream_debug_info: false,
                },
                state: AppState::Streaming(Session {
                    hint_started_at: std::time::Instant::now() - std::time::Duration::from_secs(60),
                    status: include_str!("../status.txt").to_owned(),
                    media_reconnecting: false,
                    media_refresh_failed: false,
                    video_startup: Help,
                    microphone: Microphone,
                    edge,
                    held: false,
                }),
            };
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(960.0 / 1.3, 544.0 / 1.3),
                )),
                ..Default::default()
            };
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(1.3);
            let out = ctx.run(input, |ctx| {
                streaming_screen::show(ctx, &app, None, &mut Vec::new())
            });
            let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
            canvas.set_draw_color(sdl2::pixels::Color::RGB(80, 160, 240));
            canvas.clear();
            painter
                .paint(
                    &mut canvas,
                    [960, 544],
                    out.pixels_per_point,
                    &primitives,
                    &out.textures_delta,
                )
                .unwrap();
            canvas
                .read_pixels(None, sdl2::pixels::PixelFormatEnum::RGB24)
                .unwrap()
        };
        // Pixels changed in the top half of the screen.
        let covered = |pixels: &[u8]| {
            pixels.as_chunks::<3>().0[..960 * 272]
                .iter()
                .filter(|p| **p != [80, 160, 240])
                .count()
        };
        for _ in 0..30 {
            render(State::Live);
        }
        let live = render(State::Live);
        assert_eq!(covered(&live), 0, "text over the top of live video");
        // HA09: no banner for late, interrupted or recovering video either.
        for state in [
            State::Unmeasured,
            State::AwaitingKeyframe,
            State::AwaitingPicture,
            State::ClockUncertain,
        ] {
            assert_eq!(covered(&render(state)), 0, "banner over the picture");
        }
        if let Ok(dir) = std::env::var("GREENVITA_PRESENTATION_OUTPUT") {
            std::fs::create_dir_all(&dir).unwrap();
            image::save_buffer(format!("{dir}/live-top-clear.png"), &live, 960, 544, image::ColorType::Rgb8)
                .unwrap();
        }
    }

    #[test]
    fn actual_streaming_overlay_keeps_video_visible_after_status_update() {
        let sdl = sdl2::init().unwrap();
        let window = sdl
            .video()
            .unwrap()
            .window("presentation test", 960, 544)
            .hidden()
            .build()
            .unwrap();
        let mut canvas = window.into_canvas().software().build().unwrap();
        let mut painter = painter::SdlEguiPainter::default();
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(1.3);
        fonts::configure(&ctx);
        let mut app = App {
            settings: Settings {
                locale: false,
                show_stream_debug_info: false,
            },
            state: AppState::Streaming(Session {
                hint_started_at: std::time::Instant::now(),
                status: String::new(),
                media_reconnecting: false,
                media_refresh_failed: false,
                video_startup: Help,
                microphone: Microphone,
                edge: streaming::video::live_edge::State::Live,
                held: false,
            }),
        };
        let status = include_str!("../status.txt");
        for debug in [false, true] {
            app.settings.show_stream_debug_info = debug;
            for frame in 0..180 {
                if let AppState::Streaming(session) = &mut app.state {
                    session.status = if frame < 60 {
                        String::new()
                    } else {
                        status.to_owned()
                    };
                }
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(960.0 / 1.3, 544.0 / 1.3),
                    )),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(1.3);
                let out = ctx.run(input, |ctx| {
                    streaming_screen::show(ctx, &app, None, &mut Vec::new())
                });
                let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
                canvas.set_draw_color(sdl2::pixels::Color::RGB(80, 160, 240));
                canvas.clear();
                painter
                    .paint(
                        &mut canvas,
                        [960, 544],
                        out.pixels_per_point,
                        &primitives,
                        &out.textures_delta,
                    )
                    .unwrap();
                let pixels = canvas
                    .read_pixels(None, sdl2::pixels::PixelFormatEnum::RGB24)
                    .unwrap();
                let visible = pixels
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .filter(|p| **p == [80, 160, 240])
                    .count();
                if frame == 179 {
                    if let Ok(dir) = std::env::var("GREENVITA_PRESENTATION_OUTPUT") {
                        std::fs::create_dir_all(&dir).unwrap();
                        image::save_buffer(
                            format!("{dir}/overlay-debug-{debug}.png"),
                            &pixels,
                            960,
                            544,
                            image::ColorType::Rgb8,
                        )
                        .unwrap();
                    }
                    println!("debug={debug}: unchanged video pixels={visible}/522240");
                }
                assert!(
                    visible > 522240 / 4,
                    "video obscured at frame {frame}, debug={debug}, visible={visible}"
                );
                canvas.present();
            }
        }
    }
}
