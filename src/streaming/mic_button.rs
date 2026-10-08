//! Shared overlay geometry and gesture ownership. An overlay tap must never become
//! a game pointer or an optional front-touch trigger/stick press.
use std::collections::HashSet;
use std::time::Duration;

pub(crate) const WIDTH: f32 = 112.0;
pub(crate) const HEIGHT: f32 = 38.0;
pub(crate) const BOTTOM: f32 = 8.0;
pub(crate) const BACKGROUND_ALPHA: u8 = 60;
pub(crate) const FOREGROUND_ALPHA: u8 = 180;
/// The bottom buttons stay fully visible this long after a stream starts, the
/// quick menu closes or the screen is touched, then fade out over `FADE`.
pub(crate) const VISIBLE: Duration = Duration::from_secs(10);
pub(crate) const FADE: Duration = Duration::from_secs(1);

/// Opacity of the bottom buttons `elapsed` after they were last revealed.
pub(crate) fn opacity(elapsed: Duration) -> f32 {
    if elapsed < VISIBLE {
        1.0
    } else if elapsed < VISIBLE + FADE {
        1.0 - (elapsed - VISIBLE).as_secs_f32() / FADE.as_secs_f32()
    } else {
        0.0
    }
}

/// Buttons accept taps while any part of them is drawn.
pub(crate) fn shown(elapsed: Duration) -> bool {
    elapsed < VISIBLE + FADE
}

#[derive(Clone, Copy)]
pub(crate) enum Button {
    Microphone,
    Xbox,
    QuickSettings,
}

pub(crate) fn position(button: Button, screen: (f32, f32)) -> (f32, f32) {
    let x = match button {
        Button::Microphone => 12.0,
        Button::Xbox => (screen.0 - WIDTH) / 2.0,
        Button::QuickSettings => screen.0 - WIDTH - 12.0,
    };
    (x, (screen.1 - HEIGHT - BOTTOM).max(0.0))
}

pub(crate) fn contains(x: f32, y: f32, screen: (f32, f32)) -> bool {
    [Button::Microphone, Button::Xbox, Button::QuickSettings]
        .into_iter()
        .any(|button| {
            let (left, top) = position(button, screen);
            (left..left + WIDTH).contains(&x) && (top..top + HEIGHT).contains(&y)
        })
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Pointer {
    Finger(i64),
    Mouse,
}
#[derive(Clone, Copy)]
pub(crate) enum Phase {
    Down,
    Move,
    Up,
}
/// Where a touch landed relative to the bottom buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Hit {
    Outside,
    /// On a shown button: the button owns the gesture.
    Button,
    /// Where a hidden button sits: the tap only reveals the buttons, so it
    /// neither presses the button nor reaches the game.
    HiddenButton,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Route {
    Ui,
    Game,
    Consumed,
}

#[derive(Default)]
pub(crate) struct TouchGate {
    owned: HashSet<Pointer>,
    revealing: HashSet<Pointer>,
    primary: Option<Pointer>,
}
impl TouchGate {
    pub(crate) fn route(&mut self, pointer: Pointer, phase: Phase, hit: Hit) -> Route {
        if matches!(phase, Phase::Down) {
            match hit {
                Hit::Button => {
                    self.owned.insert(pointer);
                    if self.primary.is_none() {
                        self.primary = Some(pointer);
                    }
                }
                Hit::HiddenButton => {
                    self.revealing.insert(pointer);
                }
                Hit::Outside => {}
            }
        }
        if self.revealing.contains(&pointer) {
            if matches!(phase, Phase::Up) {
                self.revealing.remove(&pointer);
            }
            return Route::Consumed;
        }
        if !self.owned.contains(&pointer) {
            return Route::Game;
        }
        let route = if self.primary == Some(pointer) {
            Route::Ui
        } else {
            Route::Consumed
        };
        if matches!(phase, Phase::Up) {
            self.owned.remove(&pointer);
            if self.primary == Some(pointer) {
                self.primary = None;
            }
        }
        route
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_bottom_buttons_capture_touches_at_vita_ui_scale() {
        let screen = (960.0 / 1.3, 544.0 / 1.3);
        for button in [Button::Microphone, Button::Xbox, Button::QuickSettings] {
            let (x, y) = position(button, screen);
            assert!(contains(x + WIDTH / 2.0, y + HEIGHT / 2.0, screen));
            assert!(!contains(x + WIDTH / 2.0, y - 1.0, screen));
            assert!(!contains(x + WIDTH / 2.0, y + HEIGHT, screen));
            assert!((y + HEIGHT + BOTTOM - screen.1).abs() < 0.001);
        }
        assert!(!contains(30.0, 20.0, screen));
        assert!(contains(screen.0 - 20.0, screen.1 - 20.0, screen));
        assert!(!contains(screen.0 - 2.0, screen.1 - 20.0, screen));
    }
    #[test]
    fn mic_drag_never_leaks_to_game_even_outside_button() {
        let mut gate = TouchGate::default();
        let finger = Pointer::Finger(1);
        assert_eq!(gate.route(finger, Phase::Down, Hit::Button), Route::Ui);
        assert_eq!(gate.route(finger, Phase::Move, Hit::Outside), Route::Ui);
        assert_eq!(gate.route(finger, Phase::Up, Hit::Outside), Route::Ui);
        assert_eq!(gate.route(finger, Phase::Down, Hit::Outside), Route::Game);
        assert_eq!(gate.route(finger, Phase::Move, Hit::Button), Route::Game);
        assert_eq!(gate.route(finger, Phase::Up, Hit::Button), Route::Game);
    }
    #[test]
    fn second_finger_cannot_toggle_or_release_primary_mic_gesture() {
        let mut gate = TouchGate::default();
        let a = Pointer::Finger(1);
        let b = Pointer::Finger(2);
        assert_eq!(gate.route(a, Phase::Down, Hit::Button), Route::Ui);
        assert_eq!(gate.route(b, Phase::Down, Hit::Button), Route::Consumed);
        assert_eq!(gate.route(b, Phase::Up, Hit::Button), Route::Consumed);
        assert_eq!(gate.route(a, Phase::Up, Hit::Button), Route::Ui);
    }
    #[test]
    fn a_tap_on_a_hidden_button_only_reveals_the_buttons() {
        let mut gate = TouchGate::default();
        let finger = Pointer::Finger(1);
        // The whole reveal gesture is consumed, even if it slides off the
        // button, and it never becomes the egui primary pointer.
        assert_eq!(
            gate.route(finger, Phase::Down, Hit::HiddenButton),
            Route::Consumed
        );
        assert_eq!(
            gate.route(finger, Phase::Move, Hit::Outside),
            Route::Consumed
        );
        assert_eq!(gate.route(finger, Phase::Up, Hit::Outside), Route::Consumed);
        // The next tap, with the buttons now shown, presses the button.
        assert_eq!(gate.route(finger, Phase::Down, Hit::Button), Route::Ui);
        assert_eq!(gate.route(finger, Phase::Up, Hit::Button), Route::Ui);
        // A tap elsewhere still reaches the game.
        assert_eq!(gate.route(finger, Phase::Down, Hit::Outside), Route::Game);
        assert_eq!(gate.route(finger, Phase::Up, Hit::Outside), Route::Game);
    }
    #[test]
    fn buttons_stay_for_ten_seconds_then_fade_out() {
        assert_eq!(opacity(Duration::ZERO), 1.0);
        assert_eq!(opacity(Duration::from_millis(9_999)), 1.0);
        assert!((opacity(Duration::from_millis(10_500)) - 0.5).abs() < 0.001);
        assert_eq!(opacity(Duration::from_secs(11)), 0.0);
        assert!(shown(Duration::from_millis(10_999)));
        assert!(!shown(Duration::from_secs(11)));
    }
}
