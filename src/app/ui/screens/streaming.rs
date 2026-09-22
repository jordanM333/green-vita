use crate::App;
use crate::app::ui::theme::Theme;
use crate::app::ui::widgets::draw_hold_progress_ring;
use crate::i18n::I18n;
use crate::streaming::video::metrics::METRICS;
use std::sync::atomic::Ordering;

/// Fullscreen video view with explicit Vita quick-menu instructions.
pub(crate) fn show(ctx: &egui::Context, app: &App, hold_progress: Option<f32>) {
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
                ui.add_space(12.0);
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
                        let recovery = if streaming.can_refresh() {
                            format!("delay:{measured} · refresh:manual")
                        } else { format!("delay:{measured}") };
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

    // Keep the route to recovery visible even after the startup hint fades.
    // This is a label, so no touch/game controls are intercepted.
    if !app.settings.show_stream_debug_info && streaming.hint_started_at.elapsed() >= HINT_VISIBLE + HINT_FADE {
        egui::Area::new(egui::Id::new("stream_quick_menu_hint"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 8.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(i18n.text("streaming-menu-hint"))
                    .color(egui::Color32::WHITE).background_color(egui::Color32::from_black_alpha(128)).size(12.0));
            });
    }

    if streaming.microphone.is_on() {
        egui::Area::new(egui::Id::new("microphone_indicator"))
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(12.0, 8.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(i18n.text("mic-live"))
                    .color(egui::Color32::WHITE).background_color(egui::Color32::from_black_alpha(192)).size(12.0));
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
