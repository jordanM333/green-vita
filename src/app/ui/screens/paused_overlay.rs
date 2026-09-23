use crate::app::ui::header::show_header_row;
use crate::app::ui::theme::Theme;
use crate::app::ui::widgets::menu_item_sized;
use crate::i18n::I18n;
use crate::{App, AppCommand, AppState, InputCommand};
use anyhow::Result;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Resume,
    RefreshStream,
    ToggleDiagnostics,
    ToggleMicrophone,
    Settings,
    PressGuideButton,
    ExitGame,
}

pub const MENU_ITEMS: [Command; 4] = [
    Command::Resume,
    Command::ToggleDiagnostics,
    Command::Settings,
    Command::ExitGame,
];

fn menu_items(app: &App) -> Vec<Command> {
    let mut items = MENU_ITEMS.to_vec();
    if app.state.streaming().is_some_and(|s| s.can_refresh()) { items.insert(1, Command::RefreshStream); }
    items.insert(1, Command::ToggleMicrophone);
    items
}

impl Command {
    fn icon(self) -> &'static str {
        match self {
            Self::Resume => "\u{25b6}",
            Self::RefreshStream => "\u{21bb}",
            Self::ToggleDiagnostics => "\u{2630}",
            Self::ToggleMicrophone => "M",
            Self::Settings => "\u{2699}",
            Self::PressGuideButton => "\u{2302}",
            Self::ExitGame => "\u{2715}",
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            Self::Resume => "paused-resume",
            Self::RefreshStream => "paused-refresh-stream",
            Self::ToggleDiagnostics => "paused-diagnostics",
            Self::ToggleMicrophone => "paused-microphone",
            Self::Settings => "menu-settings",
            Self::PressGuideButton => "paused-xbox-button",
            Self::ExitGame => "paused-exit-game",
        }
    }
}

pub(crate) fn show(ctx: &egui::Context, app: &App, commands: &mut Vec<AppCommand>) {
    let theme = Theme::dark();
    let i18n = I18n::new(app.settings.locale);
    let mut frame = egui::Frame::central_panel(&ctx.style());
    frame.fill = egui::Color32::from_rgba_unmultiplied(0x2a, 0x2a, 0x2e, 190);
    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        show_header_row(ui, app, theme, &i18n, Some(commands));
        ui.separator();
        ui.vertical_centered(|ui| {
            ui.add_space(4.0);
            match app.active_title_name() {
                Some(title_name) => {
                    ui.heading(egui::RichText::new(title_name).color(theme.text_bright));
                    ui.colored_label(theme.accent, i18n.text("screen-paused"));
                }
                None => {
                    ui.heading(egui::RichText::new(i18n.text("screen-paused")).color(theme.accent));
                }
            }
            ui.add_space(12.0);
            let home = app.state.streaming().is_some_and(|s| s.can_refresh());
            ui.label(egui::RichText::new(format!("RX Test {} · {} streaming",
                crate::build_info::NUMBER, if home { "Home" } else { "Cloud" }))
                .color(theme.text));
            ui.add_space(6.0);
            ui.set_max_width(240.0);
            for (index, item) in menu_items(app).iter().copied().enumerate() {
                if index > 0 {
                    ui.add_space(4.0);
                }
                let label = if item == Command::ToggleDiagnostics {
                    format!(
                        "{}: {}",
                        i18n.text(item.label_key()),
                        i18n.text(if app.settings.show_stream_debug_info {
                            "paused-diagnostics-on"
                        } else {
                            "paused-diagnostics-off"
                        }),
                    )
                } else if item == Command::ToggleMicrophone {
                    i18n.text(if app.state.streaming().is_some_and(|s| !s.microphone.available()) {
                        "paused-mic-not-ready"
                    } else if app.state.streaming().is_some_and(|s|s.microphone.is_on()) {
                        "paused-mic-on"
                    } else { "paused-mic-off" })
                } else {
                    i18n.text(item.label_key())
                };
                if menu_item_sized(
                    ui,
                    theme,
                    item.icon(),
                    &label,
                    matches!(&app.state, AppState::Streaming(streaming) if streaming.pause_selected == index),
                    34.0,
                ) {
                    commands.push(item.into());
                }
            }
            ui.add_space(8.0);
            if let Some(streaming) = app.state.streaming() {
                ui.label(egui::RichText::new(streaming.microphone.status()).color(theme.text).size(12.0));
                if streaming.microphone.is_on() {
                    ui.add(egui::ProgressBar::new(streaming.microphone.level()).desired_width(220.0).text(i18n.text("mic-input-level")));
                }
            }
            ui.label(egui::RichText::new(i18n.text(if home {
                "paused-home-refresh-help"
            } else { "paused-cloud-refresh-help" })).color(theme.text).size(12.0));
        });
    });
}

impl App {
    pub(crate) async fn handle_paused_overlay_input(
        &mut self,
        command: InputCommand,
    ) -> Result<()> {
        let Some(selected) = (match &self.state {
            AppState::Streaming(streaming) => Some(streaming.pause_selected),
            _ => None,
        }) else {
            return Ok(());
        };
        let items = menu_items(self);
        match command {
            InputCommand::MoveUp => {
                if let AppState::Streaming(streaming) = &mut self.state {
                    streaming.pause_selected =
                        crate::app::command::move_prev(selected, items.len());
                }
            }
            InputCommand::MoveDown => {
                if let AppState::Streaming(streaming) = &mut self.state {
                    streaming.pause_selected =
                        crate::app::command::move_next(selected, items.len());
                }
            }
            InputCommand::MoveLeft | InputCommand::MoveRight => {}
            InputCommand::Confirm => {
                let command = items
                    .get(selected)
                    .copied()
                    .unwrap_or(Command::ExitGame);
                self.handle_paused_overlay_command(command).await?;
            }
            // Only the Resume row / pause toggle resumes; Back is sent through to the game while
            // streaming and does not close this overlay.
            InputCommand::Back => {}
        }

        Ok(())
    }

    pub(crate) async fn handle_paused_overlay_command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Resume => {
                if let Some(streaming) = self.state.streaming_mut() {
                    streaming.set_paused(false);
                }
                self.menu.open = false;
            }
            Command::ToggleDiagnostics => {
                self.settings.show_stream_debug_info = !self.settings.show_stream_debug_info;
                self.settings.save();
                if let Some(streaming) = self.state.streaming_mut() {
                    streaming.set_paused(false);
                }
                self.menu.open = false;
            }
            Command::ToggleMicrophone => {
                if let Some(streaming) = self.state.streaming_mut() {
                    streaming.microphone.set_on(!streaming.microphone.is_on());
                }
            }
            Command::RefreshStream => self.refresh_home_stream(),
            Command::Settings => {
                self.open_settings();
            }
            Command::PressGuideButton => {
                if let Some(streaming) = self.state.streaming_mut() {
                    streaming.set_paused(false);
                    streaming.press_guide_button();
                }
                self.menu.open = false;
            }
            Command::ExitGame => {
                self.exit_stream().await;
            }
        }

        Ok(())
    }
}
