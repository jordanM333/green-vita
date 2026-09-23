//! Shared overlay geometry and gesture ownership. A mic tap must never become
//! a game pointer or an optional front-touch trigger/stick press.
use std::collections::HashSet;

pub(crate) const X: f32 = 12.0;
pub(crate) const Y: f32 = 8.0;
pub(crate) const WIDTH: f32 = 112.0;
pub(crate) const HEIGHT: f32 = 38.0;

pub(crate) fn contains(x: f32, y: f32) -> bool {
    (X..X + WIDTH).contains(&x) && (Y..Y + HEIGHT).contains(&y)
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
