use crate::{App, AppCommand};
use crate::app::ui::theme::Theme;
use crate::app::ui::widgets::draw_hold_progress_ring;
use crate::i18n::I18n;
use crate::streaming::video::metrics::METRICS;
use std::sync::atomic::Ordering;

/// Fullscreen video view with explicit Vita quick-menu instructions.
pub(crate) fn show(ctx: &egui::Context, app: &App, hold_progress: Option<f32>, commands: &mut Vec<AppCommand>) {
    const HINT_VISIBLE: std::time::Duration = std::time::Duration::from_secs(6);
    const HINT_FADE: std::time::Duration = std::time::Duration::from_secs(1);

    let theme = Theme::dark();
    let i18n = I18n::new(app.settings.locale);
    let streaming = match &app.state {
        crate::AppState::Streaming(streaming) => streaming,
        _ => return,
    };
    let mut frame = egui::Frame::central_panel(&ctx.style());
    frame.fill = egui::Color32::TRANSPARENT;
    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        let elapsed = streaming.hint_started_at.elapsed();
        let alpha = if elapsed < HINT_VISIBLE {
            1.0
        } else if elapsed < HINT_VISIBLE + HINT_FADE {
            1.0 - (elapsed - HINT_VISIBLE).as_secs_f32() / HINT_FADE.as_secs_f32()
        } else {
            0.0
        };

        if alpha > 0.0 {
            ui.vertical_centered(|ui| {
                ui.add_space(16.0);
                ui.colored_label(
                    theme.text.gamma_multiply(alpha),
                    i18n.text("streaming-open-menu"),
                );
            });
        }

        // Keep collecting metrics when hidden so the quick menu can restore the live overlay
        // without restarting the stream or resetting its counters.
        if app.settings.show_stream_debug_info {
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(crate::streaming::mic_button::HEIGHT + 16.0);
                egui::Frame::default()
                    .fill(egui::Color32::from_black_alpha(192))
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        // Full details go to the flight-recorder status file on exit.
                        // Use the once-per-second status; no duplicate
                        // live counters or twenty-line diagnostic paint every frame.
                        let compact = streaming.status.lines().filter(|line| {
                            ["SPS:", "H264:", "Q depth", "FPS hw", "Render draw:", "PTS matched/", "Frame age:", "Input local:", "Recovery wait:"]
                                .iter().any(|prefix| line.starts_with(prefix))
                        }).map(|line| line.chars().take(92).collect::<String>())
                            .collect::<Vec<_>>().join("\n");
                        let mode = if streaming.can_refresh() { "Home" } else { "Cloud" };
                        let measured = streaming.measured_delay_ms()
                            .map(|ms| format!("{ms}ms")).unwrap_or_else(|| "n/a".to_owned());
                        let recovery = format!("delay:{measured} · catch-up:auto");
                        ui.label(egui::RichText::new(format!("RX Test {} · {mode} · {recovery}\n{compact}", crate::build_info::NUMBER))
                            .color(theme.text).size(12.0));
                        ui.label(egui::RichText::new(format!(
                            "Audio queued:{}ms | A:{} | Hold SELECT 1.5s for Quick menu",
                            METRICS.audio_sdl_queue_ms.load(Ordering::Relaxed),
                            METRICS.local_button_mask.load(Ordering::Relaxed) & 1,
                        )).color(theme.text_bright).size(12.0));
                    });
                ui.add_space(12.0);
            });
        }
    });

    {
        use crate::streaming::mic_button;
        let screen = ctx.screen_rect().size();
        let screen = (screen.x, screen.y);
        let (mic_x, mic_y) = mic_button::position(mic_button::Button::Microphone, screen);
        let mic = &streaming.microphone;
        let on = mic.is_on();
        let active = mic.is_active();
        let available = mic.available();
        let (key, color) = if active {
            ("mic-live", egui::Color32::from_rgba_unmultiplied(166, 245, 181, mic_button::FOREGROUND_ALPHA))
        } else if on {
            ("mic-connecting", egui::Color32::from_rgba_unmultiplied(255, 219, 135, mic_button::FOREGROUND_ALPHA))
        } else if available {
            ("mic-off", egui::Color32::from_white_alpha(mic_button::FOREGROUND_ALPHA))
        } else {
            ("mic-unavailable", egui::Color32::from_white_alpha(120))
        };
        egui::Area::new(egui::Id::new("microphone_toggle"))
            .fixed_pos(egui::pos2(mic_x, mic_y))
            .order(egui::Order::Foreground)
            .movable(false)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(mic_button::WIDTH, mic_button::HEIGHT), egui::Sense::click());
                let painter = ui.painter();
                painter.rect_filled(rect, 10.0, egui::Color32::from_black_alpha(mic_button::BACKGROUND_ALPHA));
                let stroke = egui::Stroke::new(1.6, color);
                let center = rect.min + egui::vec2(20.0, 17.0);
                painter.rect_stroke(egui::Rect::from_center_size(center, egui::vec2(7.0, 13.0)),
                    4.0, stroke, egui::StrokeKind::Inside);
                painter.line_segment([center + egui::vec2(-7.0, 1.0), center + egui::vec2(-7.0, 8.0)], stroke);
                painter.line_segment([center + egui::vec2(-7.0, 8.0), center + egui::vec2(7.0, 8.0)], stroke);
                painter.line_segment([center + egui::vec2(7.0, 8.0), center + egui::vec2(7.0, 1.0)], stroke);
                painter.line_segment([center + egui::vec2(0.0, 8.0), center + egui::vec2(0.0, 12.0)], stroke);
                if !on { painter.line_segment([center + egui::vec2(-10.0, -9.0), center + egui::vec2(10.0, 12.0)], stroke); }
                painter.text(rect.min + egui::vec2(38.0, rect.height()/2.0),
                    egui::Align2::LEFT_CENTER, i18n.text(key), egui::FontId::proportional(10.5), color);
                if response.clicked() && (available || on) { mic.set_on(!on); }
            });
    }

    {
        use crate::streaming::mic_button;
        let screen = ctx.screen_rect().size();
        let (x, y) = mic_button::position(mic_button::Button::Xbox, (screen.x, screen.y));
        egui::Area::new(egui::Id::new("xbox_guide_button"))
            .fixed_pos(egui::pos2(x, y))
            .order(egui::Order::Foreground)
            .movable(false)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(mic_button::WIDTH, mic_button::HEIGHT), egui::Sense::click());
                let painter = ui.painter();
                let color = egui::Color32::from_white_alpha(mic_button::FOREGROUND_ALPHA);
                painter.rect_filled(rect, 10.0, egui::Color32::from_black_alpha(mic_button::BACKGROUND_ALPHA));
                let center = rect.min + egui::vec2(25.0, rect.height() / 2.0);
                let stroke = egui::Stroke::new(1.8, color);
                painter.circle_stroke(center, 11.0, stroke);
                painter.line_segment([center + egui::vec2(-7.0, -7.0), center + egui::vec2(7.0, 7.0)], stroke);
                painter.line_segment([center + egui::vec2(7.0, -7.0), center + egui::vec2(-7.0, 7.0)], stroke);
                painter.text(rect.min + egui::vec2(46.0, rect.height() / 2.0),
                    egui::Align2::LEFT_CENTER, "XBOX", egui::FontId::proportional(10.5), color);
                if response.clicked() {
                    commands.push(super::paused_overlay::Command::PressGuideButton.into());
                }
            });
    }

    {
        use crate::streaming::mic_button;
        let screen = ctx.screen_rect().size();
        let (x, y) = mic_button::position(mic_button::Button::QuickSettings, (screen.x, screen.y));
        egui::Area::new(egui::Id::new("quick_settings_button"))
            .fixed_pos(egui::pos2(x, y)).order(egui::Order::Foreground).movable(false)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(mic_button::WIDTH, mic_button::HEIGHT), egui::Sense::click());
                let painter = ui.painter();
                let color = egui::Color32::from_white_alpha(mic_button::FOREGROUND_ALPHA);
                painter.rect_filled(rect, 10.0, egui::Color32::from_black_alpha(mic_button::BACKGROUND_ALPHA));
                for offset in [-6.0, 0.0, 6.0] {
                    let center = rect.min + egui::vec2(25.0, rect.height() / 2.0 + offset);
                    painter.line_segment([center - egui::vec2(9.0, 0.0), center + egui::vec2(9.0, 0.0)],
                        egui::Stroke::new(1.8, color));
                }
                painter.text(rect.min + egui::vec2(46.0, rect.height() / 2.0),
                    egui::Align2::LEFT_CENTER, i18n.text("streaming-quick-settings"), egui::FontId::proportional(10.5), color);
                if response.clicked() {
                    commands.push(crate::app::command::NavigationCommand::OpenPauseOverlay.into());
                }
            });
    }

    if streaming.can_refresh() && streaming.video_lag.needs_help(std::time::Instant::now()) {
        egui::Area::new(egui::Id::new("video_lag_help"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 36.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(i18n.text("streaming-lag-home"))
                    .color(egui::Color32::WHITE)
                    .background_color(egui::Color32::from_black_alpha(160)).size(13.0));
            });
    }

    if streaming.video_startup.needs_help(std::time::Instant::now()) {
        egui::Area::new(egui::Id::new("video_startup_help"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::default().fill(egui::Color32::from_black_alpha(200))
                    .inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                        ui.set_max_width(360.0);
                        ui.label(egui::RichText::new(i18n.text("streaming-video-wait"))
                            .color(egui::Color32::WHITE).size(16.0));
                        ui.label(egui::RichText::new(i18n.text(if streaming.can_refresh() {
                            "streaming-video-retry-home"
                        } else { "streaming-video-retry-cloud" }))
                            .color(egui::Color32::WHITE).size(13.0));
                    });
            });
    }

    if let Some(progress) = hold_progress {
        egui::Area::new(egui::Id::new("pause_hold_indicator"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(16.0, 16.0))
            .show(ctx, |ui| {
                draw_hold_progress_ring(ui, progress);
            });
    }
}
