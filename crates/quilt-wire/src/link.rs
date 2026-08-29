//! Link-quality EWMA (Rung 5b).
//!
//! One exponential moving average over link-quality samples — radio RSSI
//! (dBm) on radio roads, delivery ratio (0..1) on wired roads. The OpenCode
//! design pins the smoothing constant to a **half-life in frames**: the
//! influence of a sample decays to one half after `h` further samples, so
//!
//! ```text
//! alpha = 1 - 2^(-1/h)        (h = half-life in frames)
//! q_k   = q_{k-1} + alpha * (sample_k - q_{k-1})
//! ```
//!
//! The estimator itself is pure core (no alloc, no_std): embedded targets
//! construct it with [`LinkQualityEwma::from_alpha`]. Deriving alpha from a
//! half-life needs `powf`, so [`alpha_from_half_life_frames`] is std-only —
//! firmware computes alpha at build time and ships the constant.

/// EWMA over link-quality samples.
///
/// `value()` is `None` before the first observation — the honest "no data"
/// state, distinct from any real quality value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkQualityEwma {
    alpha: f64,
    value: Option<f64>,
}

impl LinkQualityEwma {
    /// Construct from a smoothing constant directly. Panics unless
    /// `0 < alpha <= 1` (alpha = 1 = no smoothing, quality is the last sample).
    pub fn from_alpha(alpha: f64) -> Self {
        assert!(
            alpha > 0.0 && alpha <= 1.0,
            "EWMA alpha must be in (0, 1], got {alpha}"
        );
        LinkQualityEwma { alpha, value: None }
    }

    /// Construct from a half-life in frames: a sample's influence halves
    /// after `h` further samples. Panics unless `h > 0`.
    #[cfg(feature = "std")]
    pub fn from_half_life_frames(half_life: f64) -> Self {
        Self::from_alpha(alpha_from_half_life_frames(half_life))
    }

    pub fn alpha(&self) -> f64 {
        self.alpha
    }

    /// Fold in one sample; returns the updated estimate.
    pub fn update(&mut self, sample: f64) -> f64 {
        let q = match self.value {
            Some(q) => q + self.alpha * (sample - q),
            None => sample, // first observation seeds the estimate
        };
        self.value = Some(q);
        q
    }

    /// Current estimate; `None` before any observation.
    pub fn value(&self) -> Option<f64> {
        self.value
    }

    /// Forget all observations (the estimate, not the alpha).
    pub fn reset(&mut self) {
        self.value = None;
    }
}

/// alpha for a half-life of `h` frames: `1 - 2^(-1/h)`.
///
/// Consequence worth remembering: after `h` samples the weight of any one
/// sample has halved; after `k * h` samples it is `2^-k`. Panics unless
/// `h > 0`.
#[cfg(feature = "std")]
pub fn alpha_from_half_life_frames(half_life: f64) -> f64 {
    assert!(half_life > 0.0, "half-life must be > 0, got {half_life}");
    1.0 - 0.5f64.powf(1.0 / half_life)
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn alpha_matches_half_life_definition() {
        let a = alpha_from_half_life_frames(8.0);
        assert!((a - (1.0 - 0.5f64.powf(1.0 / 8.0))).abs() < 1e-15);
        assert!((alpha_from_half_life_frames(1.0) - 0.5).abs() < 1e-15);
        // Longer half-life => smaller alpha (slower tracking).
        assert!(alpha_from_half_life_frames(64.0) < alpha_from_half_life_frames(8.0));
    }

    #[test]
    fn value_halves_error_after_half_life_samples() {
        // Seed at 0, then feed a constant 1.0. After exactly `h` updates the
        // residual error must be exactly half the initial error; after `2h`,
        // a quarter. This is the defining property of the half-life alpha.
        let h = 16.0;
        let mut e = LinkQualityEwma::from_half_life_frames(h);
        e.update(0.0);
        for _ in 0..h as usize {
            e.update(1.0);
        }
        assert!((e.value().unwrap() - 0.5).abs() < 1e-9);
        for _ in 0..h as usize {
            e.update(1.0);
        }
        assert!((e.value().unwrap() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn converges_to_constant_input() {
        let mut e = LinkQualityEwma::from_half_life_frames(16.0);
        for _ in 0..200 {
            e.update(-60.0);
        }
        assert!((e.value().unwrap() - -60.0).abs() < 1e-9);
    }

    #[test]
    fn no_observation_means_none_and_reset_forgets() {
        let mut e = LinkQualityEwma::from_alpha(0.25);
        assert_eq!(e.value(), None);
        assert_eq!(e.update(-50.0), -50.0); // first sample seeds
        assert_eq!(e.value(), Some(-50.0));
        e.reset();
        assert_eq!(e.value(), None);
    }

    #[test]
    #[should_panic]
    fn rejects_nonpositive_half_life() {
        alpha_from_half_life_frames(0.0);
    }
}
