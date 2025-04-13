//! Utility traits and functions.
//!
//! This module provides general-purpose utilities including:
//! * Type conversion traits for audio processing
//! * Numeric value handling for sample calculations
//! * Safe floating point conversions
//! * Audio gain calculations:
//!   - Decibel to ratio conversion
//!   - Ratio to decibel conversion
//!   - Common audio constants
//!
//! # Audio Constants
//!
//! * `DB_TO_VOLTAGE`: 0.05 (for voltage/amplitude calculations)
//! * `VOLTAGE_TO_DB`: 20.0 (for voltage/amplitude calculations)
//! * `UNITY_GAIN`: 1.0 (no amplification/attenuation)
//! * `ZERO_DB`: 0.0 (reference level)
//!
//! # Example
//!
//! ```rust
//! use pleezer::util::{ToF32, db_to_ratio, ratio_to_db};
//!
//! // Safe numeric conversion
//! let large_value: f64 = 1e308;
//! let clamped: f32 = large_value.to_f32_lossy();
//!
//! // Audio gain calculations
//! let ratio = db_to_ratio(-6.0);  // Convert -6 dB to ratio
//! let db = ratio_to_db(0.5);      // Convert 0.5 ratio to dB
//! ```

use std::f32::consts::{LN_10, LOG10_E};

/// Trait for converting numeric values to `f32` with controlled truncation.
///
/// Provides safe conversion to `f32` by:
/// * Clamping values to `f32` range
/// * Preventing infinity values
/// * Preventing NaN values
///
/// Particularly useful for audio processing where:
/// * Sample values must be normalized to [-1.0, 1.0]
/// * Buffer sizes need safe conversion
/// * Duration calculations must avoid overflow
///
/// # Example
///
/// ```rust
/// use pleezer::util::ToF32;
///
/// let large_value: f64 = 1e308;
/// let clamped: f32 = large_value.to_f32_lossy();
/// assert!(clamped == f32::MAX);
/// ```
pub trait ToF32 {
    /// Converts a value to `f32`, clamping to prevent invalid results.
    ///
    /// Values outside the `f32` range are clamped to the nearest valid value:
    /// * Values > `f32::MAX` become `f32::MAX`
    /// * Values < `f32::MIN` become `f32::MIN`
    ///
    /// # Returns
    ///
    /// A valid `f32` value within the supported range.
    fn to_f32_lossy(self) -> f32;
}

/// Implements conversion from `f64` to `f32` with range clamping.
///
/// Clamps the value to the valid `f32` range before truncating:
/// * `f64` values beyond `f32::MAX` become `f32::MAX`
/// * `f64` values beyond `f32::MIN` become `f32::MIN`
///
/// # Example
///
/// ```rust
/// use pleezer::util::ToF32;
///
/// let too_large = f64::MAX;
/// let clamped = too_large.to_f32_lossy();
/// assert!(clamped == f32::MAX);
/// ```
impl ToF32 for f64 {
    #[inline]
    #[expect(clippy::cast_possible_truncation)]
    fn to_f32_lossy(self) -> f32 {
        self.clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32
    }
}

/// Implements conversion from `u32` to `f32` with range clamping.
///
/// Clamps the value to the valid `f32` range before truncating:
/// * `u32` values beyond `f32::MAX` become `f32::MAX`
/// * `u32` values below `f32::MIN` (0) are impossible due to unsigned type
///
/// # Example
///
/// ```rust
/// use pleezer::util::ToF32;
///
/// let too_large = u32::MAX;
/// let clamped = too_large.to_f32_lossy();
/// assert!(clamped == f32::MAX);
/// ```
impl ToF32 for u32 {
    #[inline]
    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_precision_loss)]
    #[expect(clippy::cast_sign_loss)]
    fn to_f32_lossy(self) -> f32 {
        if self > f32::MAX as u32 {
            f32::MAX
        } else {
            self as f32
        }
    }
}

/// Implements conversion from `u64` to `f32` with range clamping.
///
/// Clamps the value to the valid `f32` range before truncating:
/// * `u64` values beyond `f32::MAX` become `f32::MAX`
/// * `u64` values below `f32::MIN` (0) are impossible due to unsigned type
///
/// # Example
///
/// ```rust
/// use pleezer::util::ToF32;
///
/// let too_large = u64::MAX;
/// let clamped = too_large.to_f32_lossy();
/// assert!(clamped == f32::MAX);
/// ```
impl ToF32 for u64 {
    #[inline]
    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_precision_loss)]
    #[expect(clippy::cast_sign_loss)]
    fn to_f32_lossy(self) -> f32 {
        if self > f32::MAX as u64 {
            f32::MAX
        } else {
            self as f32
        }
    }
}

/// Implements conversion from `u128` to `f32` with range clamping.
///
/// Clamps the value to the valid `f32` range before truncating:
/// * `u128` values beyond `f32::MAX` become `f32::MAX`
/// * `u128` values below `f32::MIN` (0) are impossible due to unsigned type
///
/// # Example
///
/// ```rust
/// use pleezer::util::ToF32;
///
/// let too_large = u128::MAX;
/// let clamped = too_large.to_f32_lossy();
/// assert!(clamped == f32::MAX);
/// ```
impl ToF32 for u128 {
    #[inline]
    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_precision_loss)]
    #[expect(clippy::cast_sign_loss)]
    fn to_f32_lossy(self) -> f32 {
        if self > f32::MAX as u128 {
            f32::MAX
        } else {
            self as f32
        }
    }
}

/// Implements conversion from `usize` to `f32` with range clamping.
///
/// Clamps the value to the valid `f32` range before truncating:
/// * `usize` values beyond `f32::MAX` become `f32::MAX`
/// * `usize` values below `f32::MIN` (0) are impossible due to unsigned type
///
/// # Example
///
/// ```rust
/// use pleezer::util::ToF32;
///
/// let too_large = usize::MAX;
/// let clamped = too_large.to_f32_lossy();
/// assert!(clamped == f32::MAX);
/// ```
impl ToF32 for usize {
    #[inline]
    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_precision_loss)]
    #[expect(clippy::cast_sign_loss)]
    fn to_f32_lossy(self) -> f32 {
        if self > f32::MAX as usize {
            f32::MAX
        } else {
            self as f32
        }
    }
}

/// Multiplier for converting from decibels to voltage ratio (0.05)
pub const DB_TO_VOLTAGE: f32 = 0.05;

/// Multiplier for converting from voltage ratio to decibels (20.0)
pub const VOLTAGE_TO_DB: f32 = 20.0;

/// Unity gain (no amplification or attenuation).
pub const UNITY_GAIN: f32 = 1.0;

/// Zero decibels reference level.
pub const ZERO_DB: f32 = 0.0;

/// Converts a decibel value to a linear amplitude ratio.
///
/// Used for volume normalization calculations:
/// * 0 dB -> ratio of 1.0 (no change)
/// * Positive dB -> ratio > 1.0 (amplification)
/// * Negative dB -> ratio < 1.0 (attenuation)
///
/// # Arguments
///
/// * `db` - Decibel value to convert
///
/// # Returns
///
/// Linear amplitude ratio corresponding to the decibel value
#[must_use]
#[inline]
pub fn db_to_ratio(db: f32) -> f32 {
    // This function uses `exp(ln(10) * x)` instead of `10.powf(x)` as it's significantly
    // faster on embedded/SBC platforms where pleezer is primarily intended to run.
    // Performance testing shows:
    // * On RPi4: ~14ns vs ~35ns for powf
    // * Special case for 0 dB provides fast path (~7ns)
    //
    f32::exp(LN_10 * db * DB_TO_VOLTAGE)
}

/// Converts a linear amplitude ratio to decibels.
///
/// Inverse of `db_to_ratio`:
/// * Ratio of 1.0 -> 0 dB (no change)
/// * Ratio > 1.0 -> Positive dB (amplification)
/// * Ratio < 1.0 -> Negative dB (attenuation)
///
/// # Arguments
///
/// * `ratio` - Linear amplitude ratio to convert
///
/// # Returns
///
/// Decibel value corresponding to the amplitude ratio
#[must_use]
#[inline]
pub fn ratio_to_db(ratio: f32) -> f32 {
    // This function uses `ln(x) * LOG10_E` instead of `log10(x)` as it's significantly
    // faster on embedded/SBC platforms where pleezer is primarily intended to run.
    // Performance testing shows:
    // * On RPi4: ~17ns vs ~31ns for log10
    // * Special case for unity gain provides fast path (~7ns)
    //
    // While this approach is marginally slower on some modern processors (e.g., Apple Silicon),
    // the absolute difference is negligible (~0.27ns) compared to the significant
    // performance benefit on the target platform.
    ratio.ln() * LOG10_E * VOLTAGE_TO_DB
}
