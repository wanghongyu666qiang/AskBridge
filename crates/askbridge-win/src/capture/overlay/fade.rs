//! Time-based alpha for the overlay's entrance and cancellation fades.
//!
//! Kept as pure arithmetic on a monotonic instant so the easing is unit
//! testable and the paint path stays free of timing logic.

use std::time::{Duration, Instant};

/// How long the dim backdrop takes to reach full strength on entry.
pub(super) const FADE_IN: Duration = Duration::from_millis(140);
/// How long the backdrop takes to clear when the user cancels.
pub(super) const FADE_OUT: Duration = Duration::from_millis(110);

/// The overlay's fade state machine.
///
/// `alpha` is the fraction of the target dim to paint, in `0.0..=1.0`. A
/// settled fade reports `is_settled`, which the message loop uses to stop its
/// timer instead of repainting forever.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Fade {
    phase: Phase,
    started: Option<Instant>,
    from: f32,
    to: f32,
    duration: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    FadingIn,
    Settled,
    FadingOut,
    /// Faded out and ready to hide the window.
    Hidden,
}

impl Default for Fade {
    fn default() -> Self {
        Self {
            phase: Phase::FadingIn,
            started: None,
            from: 0.0,
            to: 1.0,
            duration: FADE_IN,
        }
    }
}

impl Fade {
    /// Starts the fade-in. Called once the window is on screen.
    pub(super) fn begin(&mut self, now: Instant) {
        self.phase = Phase::FadingIn;
        self.started = Some(now);
        self.from = 0.0;
        self.to = 1.0;
        self.duration = FADE_IN;
    }

    /// Starts the fade-out from the current alpha. Calling this twice keeps
    /// the first call's start point so the exit does not jump.
    pub(super) fn cancel(&mut self, now: Instant) {
        if matches!(self.phase, Phase::FadingOut | Phase::Hidden) {
            return;
        }
        self.from = self.alpha_at(now);
        self.to = 0.0;
        self.duration = FADE_OUT;
        self.started = Some(now);
        self.phase = Phase::FadingOut;
    }

    /// Advances the state machine. Returns `true` when the paint should be
    /// repeated, i.e. while a fade is in progress.
    pub(super) fn tick(&mut self, now: Instant) -> bool {
        if !matches!(self.phase, Phase::FadingIn | Phase::FadingOut) {
            return false;
        }
        if self.progress_at(now) >= 1.0 {
            if self.phase == Phase::FadingIn {
                self.phase = Phase::Settled;
            } else {
                self.phase = Phase::Hidden;
            }
            return false;
        }
        true
    }

    /// Whether the window should be hidden because the exit fade finished.
    pub(super) const fn is_hidden(&self) -> bool {
        matches!(self.phase, Phase::Hidden)
    }

    /// Whether the overlay has finished animating.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) const fn is_settled(&self) -> bool {
        matches!(self.phase, Phase::Settled)
    }

    /// Whether the overlay is on its way out.
    pub(super) const fn is_fading_out(&self) -> bool {
        matches!(self.phase, Phase::FadingOut | Phase::Hidden)
    }

    /// The current alpha in `0.0..=1.0`.
    pub(super) fn alpha(&self, now: Instant) -> f32 {
        self.alpha_at(now)
    }

    fn alpha_at(&self, now: Instant) -> f32 {
        let progress = self.progress_at(now);
        let eased = ease_out_cubic(progress);
        self.from + (self.to - self.from) * eased
    }

    fn progress_at(&self, now: Instant) -> f32 {
        let Some(started) = self.started else {
            return if self.phase == Phase::FadingIn {
                0.0
            } else {
                1.0
            };
        };
        if self.duration.is_zero() {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(started).as_secs_f32();
        (elapsed / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }
}

/// `1 - (1 - t)^3`: fast start, gentle settle. Matches the `--ease-out` curve
/// used by the settings page and the capture toolbar.
fn ease_out_cubic(t: f32) -> f32 {
    let clamped = t.clamp(0.0, 1.0);
    1.0 - (1.0 - clamped).powi(3)
}

/// The selection glow breathes once per this period, so a locked selection
/// keeps reading as "live" without anything moving.
pub(super) const PULSE_PERIOD: Duration = Duration::from_millis(2200);

/// A gentle sine breath in `0.82..=1.18`, used to scale the glow alphas.
pub(super) fn glow_pulse(elapsed: Duration) -> f32 {
    let period = PULSE_PERIOD.as_secs_f32();
    if period <= 0.0 {
        return 1.0;
    }
    let phase = elapsed.as_secs_f32() / period * std::f32::consts::TAU;
    1.0 + 0.18 * phase.sin()
}

/// Maps a fade fraction onto the overlay's dim alpha byte.
pub(super) fn scaled_alpha(base: u8, fraction: f32) -> u8 {
    let clamped = fraction.clamp(0.0, 1.0);
    ((f32::from(base) * clamped).round() as i32).clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> Instant {
        Instant::now()
    }

    #[test]
    fn fade_in_reaches_full_alpha_then_settles() {
        let now = start();
        let mut fade = Fade::default();
        fade.begin(now);

        assert_eq!(fade.alpha(now), 0.0);
        let middle = fade.alpha(now + FADE_IN / 2);
        assert!(middle > 0.0 && middle < 1.0, "midpoint alpha was {middle}");

        assert_eq!(fade.alpha(now + FADE_IN), 1.0);
        assert!(!fade.tick(now + FADE_IN));
        assert!(fade.is_settled());
    }

    #[test]
    fn fade_in_is_monotonic_and_never_overshoots() {
        let now = start();
        let mut fade = Fade::default();
        fade.begin(now);

        let mut previous = -1.0_f32;
        for step in 0..=20 {
            let alpha = fade.alpha(now + FADE_IN.mul_f32(step as f32 / 20.0));
            assert!(alpha >= previous, "alpha went backwards at step {step}");
            assert!((0.0..=1.0).contains(&alpha), "alpha out of range: {alpha}");
            previous = alpha;
        }
    }

    #[test]
    fn tick_reports_paint_while_animating_only() {
        let now = start();
        let mut fade = Fade::default();
        fade.begin(now);

        assert!(fade.tick(now));
        assert!(fade.tick(now + FADE_IN / 2));
        assert!(!fade.tick(now + FADE_IN));
        assert!(fade.is_settled());
    }

    #[test]
    fn cancel_fades_from_the_current_alpha_without_jumping() {
        let now = start();
        let mut fade = Fade::default();
        fade.begin(now);
        let halfway = now + FADE_IN / 2;
        let alpha_before = fade.alpha(halfway);

        fade.cancel(halfway);
        let alpha_after = fade.alpha(halfway);
        assert!(
            (alpha_before - alpha_after).abs() < 0.001,
            "cancel jumped from {alpha_before} to {alpha_after}"
        );
        assert!(fade.is_fading_out());

        assert_eq!(fade.alpha(halfway + FADE_OUT), 0.0);
        assert!(!fade.tick(halfway + FADE_OUT));
        assert!(fade.is_hidden());
    }

    #[test]
    fn cancel_is_idempotent() {
        let now = start();
        let mut fade = Fade::default();
        fade.begin(now);
        fade.cancel(now + Duration::from_millis(50));
        let first = fade.alpha(now + Duration::from_millis(80));
        fade.cancel(now + Duration::from_millis(60));
        let second = fade.alpha(now + Duration::from_millis(80));

        assert!((first - second).abs() < 0.001);
    }

    #[test]
    fn cancel_before_the_window_was_shown_still_exits() {
        let now = start();
        let mut fade = Fade::default();
        // No `begin`, mirroring a cancel that races the first paint.
        fade.cancel(now);
        assert_eq!(fade.alpha(now + FADE_OUT), 0.0);
        assert!(!fade.tick(now + FADE_OUT));
        assert!(fade.is_hidden());
    }

    #[test]
    fn settled_fade_ignores_further_ticks() {
        let now = start();
        let mut fade = Fade::default();
        fade.begin(now);
        fade.tick(now + FADE_IN);

        assert!(!fade.tick(now + FADE_IN + Duration::from_secs(1)));
        assert_eq!(fade.alpha(now + FADE_IN + Duration::from_secs(1)), 1.0);
    }

    #[test]
    fn easing_matches_the_documented_curve() {
        assert!((ease_out_cubic(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < f32::EPSILON);
        // Ease-out is ahead of linear in the first half.
        assert!(ease_out_cubic(0.5) > 0.5);
        assert!((ease_out_cubic(0.5) - 0.875).abs() < 0.001);
        // Out-of-range input is clamped, not extrapolated.
        assert!((ease_out_cubic(-1.0) - 0.0).abs() < f32::EPSILON);
        assert!((ease_out_cubic(2.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn scaled_alpha_rounds_and_clamps() {
        assert_eq!(scaled_alpha(145, 0.0), 0);
        assert_eq!(scaled_alpha(145, 1.0), 145);
        assert_eq!(scaled_alpha(145, 0.5), 73);
        assert_eq!(scaled_alpha(145, 2.0), 145);
        assert_eq!(scaled_alpha(145, -1.0), 0);
        assert_eq!(scaled_alpha(255, 1.0), 255);
    }
}
