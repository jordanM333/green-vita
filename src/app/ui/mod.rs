pub(crate) mod fonts;
pub mod header;
pub mod screens;
mod theme;
mod widgets;

use crate::{App, AppCommand, AppState};

pub fn build_ui(ctx: &egui::Context, app: &App, hold_progress: Option<f32>) -> Vec<AppCommand> {
    let mut commands = Vec::new();

    match &app.state {
        AppState::LanguageSelect { .. } => {
            screens::language_select::show(ctx, app, &mut commands);
        }
        AppState::Streaming(streaming) if streaming.paused => {
            screens::paused_overlay::show(ctx, app, &mut commands);
        }
        AppState::Streaming(_) => {
            screens::streaming::show(ctx, app, hold_progress, &mut commands);
        }
        AppState::TitleList { .. } | AppState::LoadingTitles(_) => {
            screens::title_list::show(ctx, app, &mut commands);
        }
        AppState::ConsoleList { .. } | AppState::LoadingConsoles(_) => {
            screens::console_list::show(ctx, app, &mut commands);
        }
        AppState::InitializeAuthentication
        | AppState::RequestingDeviceCode(_)
        | AppState::LoadingCredentials(_) => {
            screens::signing_in::show(ctx, app);
        }
        AppState::StartingStream { .. } | AppState::Connecting { .. } => {
            screens::connecting::show(ctx, app);
        }
        AppState::Error { .. } => {
            screens::error::show(ctx, app, &mut commands);
        }
        AppState::WaitingForDeviceAuthorization { .. } => {
            screens::token_setup::show(ctx, app);
        }
        AppState::ModeSelect { .. } => {
            screens::mode_select::show(ctx, app, &mut commands);
        }
        AppState::Settings { .. } => {
            screens::settings::show(ctx, app, &mut commands);
        }
    }

    if app.settings.persistence_error {
        egui::Area::new(egui::Id::new("settings_persistence_error"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 8.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::default()
                    .fill(egui::Color32::from_black_alpha(230))
                    .show(ui, |ui| {
                        ui.set_max_width(400.0);
                        ui.label(
                            crate::i18n::I18n::new(app.settings.locale)
                                .text("settings-storage-error"),
                        );
                    });
            });
    }
    commands
}
