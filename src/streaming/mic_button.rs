//! Shared overlay geometry and gesture ownership. An overlay tap must never become
//! a game pointer or an optional front-touch trigger/stick press.
use std::collections::HashSet;

pub(crate) const WIDTH: f32 = 112.0;
pub(crate) const HEIGHT: f32 = 38.0;
pub(crate) const BOTTOM: f32 = 8.0;
pub(crate) const BACKGROUND_ALPHA: u8 = 60;
pub(crate) const FOREGROUND_ALPHA: u8 = 180;

#[derive(Clone, Copy)]
pub(crate) enum Button { Microphone, Xbox }

pub(crate) fn position(button: Button, screen: (f32, f32)) -> (f32, f32) {
    let x = match button { Button::Microphone => 12.0, Button::Xbox => (screen.0 - WIDTH) / 2.0 };
    (x, (screen.1 - HEIGHT - BOTTOM).max(0.0))
}

pub(crate) fn contains(x: f32, y: f32, screen: (f32, f32)) -> bool {
    [Button::Microphone, Button::Xbox].into_iter().any(|button| {
        let (left, top) = position(button, screen);
        (left..left + WIDTH).contains(&x) && (top..top + HEIGHT).contains(&y)
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Pointer { Finger(i64), Mouse }
#[derive(Clone, Copy)]
pub(crate) enum Phase { Down, Move, Up }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Route { Ui, Game, Consumed }

#[derive(Default)]
pub(crate) struct TouchGate {
    owned: HashSet<Pointer>,
    primary: Option<Pointer>,
}
impl TouchGate {
    pub(crate) fn route(&mut self, pointer: Pointer, phase: Phase, inside: bool) -> Route {
        if matches!(phase, Phase::Down) && inside {
            self.owned.insert(pointer);
            if self.primary.is_none() { self.primary = Some(pointer); }
        }
        if !self.owned.contains(&pointer) { return Route::Game; }
        let route = if self.primary == Some(pointer) { Route::Ui } else { Route::Consumed };
        if matches!(phase, Phase::Up) {
            self.owned.remove(&pointer);
            if self.primary == Some(pointer) { self.primary = None; }
        }
        route
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_bottom_buttons_capture_touches_at_vita_ui_scale() {
        let screen = (960.0 / 1.3, 544.0 / 1.3);
        for button in [Button::Microphone, Button::Xbox] {
            let (x, y) = position(button, screen);
            assert!(contains(x + WIDTH / 2.0, y + HEIGHT / 2.0, screen));
            assert!(!contains(x + WIDTH / 2.0, y - 1.0, screen));
            assert!(!contains(x + WIDTH / 2.0, y + HEIGHT, screen));
            assert!((y + HEIGHT + BOTTOM - screen.1).abs() < 0.001);
        }
        assert!(!contains(30.0, 20.0, screen));
        assert!(!contains(screen.0 - 20.0, screen.1 - 20.0, screen));
    }
    #[test]
    fn mic_drag_never_leaks_to_game_even_outside_button() {
        let mut gate=TouchGate::default(); let finger=Pointer::Finger(1);
        assert_eq!(gate.route(finger,Phase::Down,true),Route::Ui);
        assert_eq!(gate.route(finger,Phase::Move,false),Route::Ui);
        assert_eq!(gate.route(finger,Phase::Up,false),Route::Ui);
        assert_eq!(gate.route(finger,Phase::Down,false),Route::Game);
        assert_eq!(gate.route(finger,Phase::Move,true),Route::Game);
        assert_eq!(gate.route(finger,Phase::Up,true),Route::Game);
    }
    #[test]
    fn second_finger_cannot_toggle_or_release_primary_mic_gesture() {
        let mut gate=TouchGate::default(); let a=Pointer::Finger(1); let b=Pointer::Finger(2);
        assert_eq!(gate.route(a,Phase::Down,true),Route::Ui);
        assert_eq!(gate.route(b,Phase::Down,true),Route::Consumed);
        assert_eq!(gate.route(b,Phase::Up,true),Route::Consumed);
        assert_eq!(gate.route(a,Phase::Up,true),Route::Ui);
    }
}
