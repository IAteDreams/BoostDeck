//! Peak meters.
//!
//! Peaks are tracked per buffer by `Chain::process`; the render thread stores
//! them into the `Shared` atomics and the UI reads and resets them. These are
//! the conversions both sides share. Must not allocate.

/// Linear gain or amplitude to decibels. Zero is negative infinity.
#[inline]
pub fn to_db(linear: f32) -> f32 {
    20.0 * linear.log10()
}

/// Decibels to linear gain or amplitude.
#[inline]
pub fn from_db(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

/// Largest magnitude in `samples`, or 0 for an empty slice.
#[inline]
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decibels_round_trip() {
        for db in [-60.0, -6.0, -1.0, 0.0, 6.0, 12.0, 24.0] {
            assert!((to_db(from_db(db)) - db).abs() < 1e-4, "{db}");
        }
        assert!((from_db(6.0) - 1.995).abs() < 1e-3);
        assert_eq!(to_db(0.0), f32::NEG_INFINITY);
    }

    #[test]
    fn peak_is_the_largest_magnitude() {
        assert_eq!(peak(&[0.1, -0.8, 0.5]), 0.8);
        assert_eq!(peak(&[]), 0.0);
    }
}
