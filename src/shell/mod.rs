#[link(name = "SDL2", kind = "static")]
#[link(name = "SceAudio_stub", kind = "static")]
unsafe extern "C" {}

mod egui_painter;
mod surface;

use crate::app::ui::build_ui;
use crate::input::{
    RearTouchButtons, back_button_held, held_menu_direction, map_controller_button_event,
    map_keyboard_event, map_pointer_event, map_stream_pointer_event, open_first_controller,
    read_gamepad_frame,
};
use crate::streaming::audio::AudioRenderer;
use crate::streaming::video::{STREAM_HEIGHT, STREAM_WIDTH};
use crate::{App, AppState, InputCommand, NavigationCommand};
use anyhow::{Context, Result};
use sdl2::event::Event;
use std::time::{Duration, Instant};
use surface::{HEIGHT, VitaSurface, WIDTH};
use tokio::time::sleep;

const PAUSE_HOLD_DURATION: Duration = Duration::from_millis(1500);
/// Scales `pixels_per_point` up so the UI reads legibly on the Vita's small screen.
const UI_SCALE: f32 = 1.3;
// D-Pad/left-stick auto-repeat: immediate on press, then repeating once held past the delay.
const DIRECTION_REPEAT_INITIAL_DELAY: Duration = Duration::from_millis(350);
const DIRECTION_REPEAT_INTERVAL: Duration = Duration::from_millis(90);
const STREAM_INPUT_POLL_INTERVAL: Duration = Duration::from_millis(4);
const STREAM_STATUS_REFRESH_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) const TARGET_FRAME_TIME: Duration = Duration::from_millis(16);

pub async fn run(mut app: App) -> Result<()> {
    crate::streaming::video::reserve_decoder_cdram();

    let sdl = sdl2::init().map_err(anyhow::Error::msg)?;
    let video = sdl.video().map_err(anyhow::Error::msg)?;
    let audio = sdl.audio().map_err(anyhow::Error::msg)?;
    crate::input::register_vita_controller_mapping(&sdl).map_err(anyhow::Error::msg)?;
    let game_controller_subsystem = sdl.game_controller().map_err(anyhow::Error::msg)?;
    let mut controller = open_first_controller(&game_controller_subsystem);
    let joystick_subsystem = sdl.joystick().map_err(anyhow::Error::msg)?;
    let raw_joystick = joystick_subsystem.open(0).ok();
    let mut event_pump = sdl.event_pump().map_err(anyhow::Error::msg)?;
    let mut surface = VitaSurface::new(&video)?;
    let mut audio_renderer =
        AudioRenderer::new(&audio).context("failed to set up audio renderer")?;
    let egui_ctx = egui::Context::default();
    crate::app::ui::fonts::configure(&egui_ctx);
    let start_time = Instant::now();
    let mut pointer_pos = egui::Pos2::ZERO;
    let mut back_hold_since: Option<Instant> = None;
    let mut rear_touch_buttons = RearTouchButtons::default();
    let mut held_direction: Option<InputCommand> = None;
    let mut held_direction_since = Instant::now();
    let mut last_direction_repeat_at = Instant::now();
    let mut vita_ime_active = false;
    let mut vita_ime_pending = None;
    let mut last_ui_presented_at = Instant::now();

    loop {
        let loop_started_at = Instant::now();
        let mut egui_events = Vec::new();
        let mut direct_commands = Vec::new();
        if app.title_search_requested && !vita_ime_active {
            app.title_search_requested = false;
            vita_ime_pending = None;
            video.text_input().start();
            vita_ime_active = true;
        }

        for event in event_pump.poll_iter() {
            rear_touch_buttons.handle_event(&event);
            let ime_owned_event = vita_ime_active;
            if ime_owned_event {
                match &event {
                    Event::TextInput { text, .. } => vita_ime_pending = Some(text.clone()),
                    Event::KeyDown {
                        keycode: Some(sdl2::keyboard::Keycode::Return),
                        ..
                    } => {
                        if let Some(text) = vita_ime_pending.take() {
                            app.set_title_search_query(text);
                        }
                        video.text_input().stop();
                        vita_ime_active = false;
                    }
                    _ => {}
                }
            }
            // SDL may emit both events for one press.
            if !ime_owned_event
                && let Some(command) = map_keyboard_event(&event)
                && !direct_commands.contains(&command)
            {
                direct_commands.push(command);
            }
            if !ime_owned_event
                && let Some(command) = map_controller_button_event(&event)
                && !direct_commands.contains(&command)
            {
                direct_commands.push(command);
            }
            if let Some(egui_event) = map_pointer_event(
                &event,
                (WIDTH as f32 / UI_SCALE, HEIGHT as f32 / UI_SCALE),
                UI_SCALE,
                &mut pointer_pos,
            ) {
                egui_events.push(egui_event);
            }
            if let AppState::Streaming(streaming) = &app.state
                && !streaming.paused
                && !streaming.front_touch_auxiliary_buttons(&app.settings)
                && let Some(pointer_event) = map_stream_pointer_event(
                    &event,
                    (WIDTH as f32, HEIGHT as f32),
                    surface.video_rect(),
                    // Scale input into the active stream resolution.
                    streaming
                        .video_size()
                        .unwrap_or((STREAM_WIDTH, STREAM_HEIGHT)),
                )
            {
                streaming.send_pointer_event(pointer_event);
            }
            if let Event::ControllerDeviceAdded { .. } = event
                && controller.is_none()
            {
                controller = open_first_controller(&game_controller_subsystem);
            }
            if let Event::ControllerDeviceRemoved { .. } = event {
                controller = None;
            }
        }

        if vita_ime_active
            && !video
                .text_input()
                .is_screen_keyboard_shown(surface.canvas.window())
        {
            video.text_input().stop();
            vita_ime_pending = None;
            vita_ime_active = false;
        }

        match (!vita_ime_active)
            .then(|| held_menu_direction(controller.as_ref()))
            .flatten()
        {
            Some(direction) if held_direction == Some(direction) => {
                if held_direction_since.elapsed() >= DIRECTION_REPEAT_INITIAL_DELAY
                    && last_direction_repeat_at.elapsed() >= DIRECTION_REPEAT_INTERVAL
                {
                    direct_commands.push(direction.into());
                    last_direction_repeat_at = Instant::now();
                }
            }
            Some(direction) => {
                direct_commands.push(direction.into());
                held_direction = Some(direction);
                held_direction_since = Instant::now();
                last_direction_repeat_at = Instant::now();
            }
            None => held_direction = None,
        }

        // Drives the top-left hold-progress ring in `build_ui`.
        let mut hold_progress: Option<f32> = None;
        // Back doubles as game input (View) and as the hold-to-pause gesture: withheld while
        // held, replayed as a single View tap only if released before the pause fired.
        let mut relay_back_as_view = false;
        if matches!(&app.state, AppState::Streaming(streaming) if !streaming.paused) {
            if back_button_held(controller.as_ref()) {
                let held_since = *back_hold_since.get_or_insert_with(Instant::now);
                let elapsed = held_since.elapsed();
                hold_progress = Some(
                    (elapsed.as_secs_f32() / PAUSE_HOLD_DURATION.as_secs_f32()).clamp(0.0, 1.0),
                );
                if elapsed >= PAUSE_HOLD_DURATION {
                    direct_commands.push(NavigationCommand::OpenPauseOverlay.into());
                    back_hold_since = None;
                }
            } else if let Some(held_since) = back_hold_since.take() {
                relay_back_as_view = held_since.elapsed() < PAUSE_HOLD_DURATION;
            }
        } else {
            back_hold_since = None;
        }

        for command in direct_commands {
            app.handle_command(command).await?;
        }

        send_stream_gamepad_state(
            &mut app,
            controller.as_ref(),
            raw_joystick.as_ref(),
            &rear_touch_buttons,
            relay_back_as_view,
        );

        app.tick().await?;
        if let Some(streaming) = app.state.streaming_mut() {
            audio_renderer.submit_packets(streaming.take_audio_packets());
        }
        // Painting an unchanged picture costs about 14 ms in the on-device trace.
        // Keep polling input, audio, and the stream, but save that GPU work
        // until the decoder has a newer frame. Refresh the UI periodically even if video stalls.
        let skip_unchanged_stream = matches!(&app.state, AppState::Streaming(streaming) if !streaming.paused)
            && surface.has_displayed_video_frame()
            && !surface.has_pending_video_frame()
            && egui_events.is_empty()
            && hold_progress.is_none()
            && last_ui_presented_at.elapsed() < STREAM_STATUS_REFRESH_INTERVAL;

        if !skip_unchanged_stream {
            let raw_input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(WIDTH as f32 / UI_SCALE, HEIGHT as f32 / UI_SCALE),
                )),
                viewport_id: egui::ViewportId::ROOT,
                viewports: std::iter::once((
                    egui::ViewportId::ROOT,
                    egui::ViewportInfo {
                        native_pixels_per_point: Some(UI_SCALE),
                        ..Default::default()
                    },
                ))
                .collect(),
                time: Some(start_time.elapsed().as_secs_f64()),
                predicted_dt: TARGET_FRAME_TIME.as_secs_f32(),
                events: egui_events,
                ..Default::default()
            };

            let mut ui_commands = Vec::new();
            let full_output = egui_ctx.run(raw_input, |ctx| {
                ui_commands = build_ui(ctx, &app, hold_progress);
            });

            for command in ui_commands {
                app.handle_command(command).await?;
            }

            // Acquire the newest decoded frame immediately before drawing. It may have arrived
            // while egui was building the overlay, and old decoded pictures stay replaceable.
            surface.sync_video_frame(app.state.streaming())?;
            surface.draw_scene(matches!(&app.state, AppState::Streaming(_)))?;
            let clipped_primitives =
                egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
            surface.paint_egui(
                full_output.pixels_per_point,
                &clipped_primitives,
                &full_output.textures_delta,
            )?;
            last_ui_presented_at = Instant::now();
        } else {
            crate::streaming::video::metrics::METRICS
                .unchanged_frame_skipped
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        let metrics = &crate::streaming::video::metrics::METRICS;
        if !skip_unchanged_stream {
            let loop_us = loop_started_at.elapsed().as_micros() as u64;
            metrics.ui_loop_sum_us.fetch_add(loop_us, std::sync::atomic::Ordering::Relaxed);
            metrics.ui_loop_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            metrics.ui_loop_max_us.fetch_max(loop_us, std::sync::atomic::Ordering::Relaxed);
        }
        let frame_deadline = loop_started_at
            + if skip_unchanged_stream {
                STREAM_INPUT_POLL_INTERVAL
            } else {
                TARGET_FRAME_TIME
            };
        let display_output = app.state.streaming()
            .filter(|streaming| !streaming.paused)
            .map(|streaming| streaming.direct_video_output());
        if Instant::now() < frame_deadline {
            while Instant::now() < frame_deadline {
                let remaining = frame_deadline.saturating_duration_since(Instant::now());
                // New video interrupts frame pacing; input still refreshes every 4 ms
                // when the decoder is idle. Menus retain their normal frame limit.
                if let Some(output) = display_output.as_ref() {
                    tokio::select! {
                        _ = output.wait_for_frame() => break,
                        _ = sleep(remaining.min(STREAM_INPUT_POLL_INTERVAL)) => {}
                    }
                } else {
                    sleep(remaining.min(STREAM_INPUT_POLL_INTERVAL)).await;
                }
                if Instant::now() >= frame_deadline {
                    break;
                }

                // SDL controller state is refreshed independently of rendering. Events remain
                // queued for the normal UI pass at the start of the next frame.
                event_pump.pump_events();
                send_stream_gamepad_state(
                    &mut app,
                    controller.as_ref(),
                    raw_joystick.as_ref(),
                    &rear_touch_buttons,
                    false,
                );
            }
        } else {
            tokio::task::yield_now().await;
        }
    }
}

fn send_stream_gamepad_state(
    app: &mut App,
    controller: Option<&sdl2::controller::GameController>,
    raw_joystick: Option<&sdl2::joystick::Joystick>,
    rear_touch_buttons: &RearTouchButtons,
    relay_back_as_view: bool,
) {
    let settings = &app.settings;
    let (rear_touch_enabled, front_touch_auxiliary_buttons) = match &app.state {
        AppState::Streaming(streaming) if !streaming.paused => (
            streaming.rear_touch_enabled(settings),
            streaming.front_touch_auxiliary_buttons(settings),
        ),
        _ => return,
    };
    let AppState::Streaming(streaming) = &mut app.state else {
        return;
    };
    let Some(mut frame) = read_gamepad_frame(
        controller,
        raw_joystick,
        rear_touch_buttons,
        rear_touch_enabled,
        front_touch_auxiliary_buttons,
        settings.swap_rear_touch_trigger_stick,
    ) else {
        return;
    };

    // Back is relayed separately after the hold gesture resolves.
    frame.view = f32::from(relay_back_as_view);
    // Keep a local, per-render-frame marker so a phone recording can compare the physical
    // input with the first corresponding movement in Xbox's video. No stream settings change.
    let mask = u64::from(frame.a > 0.0)
        | (u64::from(frame.b > 0.0) << 1)
        | (u64::from(frame.right_trigger > 0.2) << 2);
    let lx = (frame.left_thumb_x_axis * 100.0).round().clamp(-100.0, 100.0) as i16;
    let ly = (frame.left_thumb_y_axis * 100.0).round().clamp(-100.0, 100.0) as i16;
    let metrics = &crate::streaming::video::metrics::METRICS;
    metrics.local_button_mask.store(mask, std::sync::atomic::Ordering::Relaxed);
    metrics.local_left_stick.store(
        (lx as u16 as u64) | ((ly as u16 as u64) << 16),
        std::sync::atomic::Ordering::Relaxed,
    );
    streaming.send_gamepad_frame(frame, settings);
}
