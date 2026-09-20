use crate::App;
use crate::app::ui::theme::Theme;
use crate::app::ui::widgets::draw_hold_progress_ring;
use crate::i18n::I18n;
use crate::streaming::video::metrics::METRICS;
use std::sync::atomic::Ordering;

/// Fullscreen video view with a one-time "Hold Back to pause" hint, fading out over `HINT_FADE`.
pub(crate) fn show(ctx: &egui::Context, app: &App, hold_progress: Option<f32>) {
    const HINT_VISIBLE: std::time::Duration = std::time::Duration::from_secs(2);
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
                    i18n.text("streaming-hold-back"),
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
                            ["SPS:", "AU done:", "Q depth", "FPS hw", "Link ICE:", "Video payload:"]
                                .iter().any(|prefix| line.starts_with(prefix))
                        }).map(|line| line.chars().take(115).collect::<String>())
                            .collect::<Vec<_>>().join("\n");
                        ui.label(egui::RichText::new(format!("STREAM V2 · decode queue: 3 AU / 50ms\n{compact}"))
                            .color(theme.text).size(12.0));
                        ui.label(egui::RichText::new(format!(
                            "Audio queued:{}ms | A:{} | diagnostics: pause menu",
                            METRICS.audio_sdl_queue_ms.load(Ordering::Relaxed),
                            METRICS.local_button_mask.load(Ordering::Relaxed) & 1,
                        )).color(theme.text_bright).size(12.0));
                    });
                ui.add_space(12.0);
            });
        }
    });

    if let Some(progress) = hold_progress {
        egui::Area::new(egui::Id::new("pause_hold_indicator"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(16.0, 16.0))
            .show(ctx, |ui| {
                draw_hold_progress_ring(ui, progress);
            });
    }
}
