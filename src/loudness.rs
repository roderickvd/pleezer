//! Equal loudness compensation based on ISO 226:2013 standard using biquad filters.
//!
//! Implements precise equal-loudness contours using:
//! * Multi-band IIR filter bank with 6 bands (low shelf, 4 peaks, high shelf)
//! * ISO 226:2013 frequency response curves
//! * Reference level of 83 dB SPL
//! * Volume-dependent gain adjustments
//! * Phase-optimized filter design
//!
//! # Filter Design
//!
//! Six-band filter configuration:
//! * 30 Hz - Low shelf (Q=0.707)
//! * 100 Hz - Low-mid peak (Q=1.0)
//! * 500 Hz - Mid peak (Q=1.414)
//! * 2 kHz - Upper-mid peak (Q=1.2)
//! * 6 kHz - Presence peak (Q=1.5)
//! * 12 kHz - High shelf (Q=0.707)
//!
//! The filter gains are dynamically adjusted based on:
//! * Current listening level (volume)
//! * Target LUFS level
//! * Equal-loudness contour shapes
//! * Reference playback level (83 dB SPL)

use std::f32::consts::SQRT_2;

use biquad::{Biquad, Coefficients, DirectForm1, Q_BUTTERWORTH_F32, ToHertz, Type};
use rodio::SampleRate;

/// ISO 226:2013 standard frequencies in Hz
const FREQUENCIES: &[f32] = &[
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
    500.0, 630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0,
    8000.0, 10000.0, 12500.0,
];

/// Acoustic transfer function coefficients (`α_f`)
const ALPHA_F: &[f32] = &[
    0.532, 0.506, 0.480, 0.455, 0.432, 0.409, 0.387, 0.367, 0.349, 0.330, 0.315, 0.301, 0.288,
    0.276, 0.267, 0.259, 0.253, 0.250, 0.246, 0.244, 0.243, 0.243, 0.243, 0.242, 0.242, 0.245,
    0.254, 0.271, 0.301,
];

/// Hearing threshold coefficients (`L_U`)
const L_U: &[f32] = &[
    -31.6, -27.2, -23.0, -19.1, -15.9, -13.0, -10.3, -8.1, -6.2, -4.5, -3.1, -2.0, -1.1, -0.4, 0.0,
    0.3, 0.5, 0.0, -2.7, -4.1, -1.0, 1.7, 2.5, 1.2, -2.1, -7.1, -11.2, -10.7, -3.1,
];

/// Threshold of hearing coefficients (`T_f`)
const T_F: &[f32] = &[
    78.5, 68.7, 59.5, 51.1, 44.0, 37.5, 31.5, 26.5, 22.1, 17.9, 14.4, 11.4, 8.6, 6.2, 4.4, 3.0,
    2.2, 2.4, 3.5, 1.7, -1.3, -4.2, -6.0, -5.4, -1.5, 6.0, 12.6, 13.9, 12.3,
];

/// Reference sound pressure level (dB)
/// Used in ISO 226:2013 calculations
const REF_SPL: f32 = 94.0;

/// Loudness scaling factor from ISO 226:2013 standard
const LOUDNESS_SCALE: f32 = 4.47e-3;

/// Reference sound pressure level for playback calibration (dB SPL)
/// Currently fixed at 83 dB SPL, which corresponds to K-20 metering standard
pub const REFERENCE_SPL: f32 = 83.0;

/// Number of bands in the filter bank
const NUM_BANDS: usize = 6;

/// Center frequencies for each filter band in Hz
const BAND_FREQUENCIES: [f32; NUM_BANDS] = [
    30.0,    // Low shelf
    100.0,   // Low-mid peak
    500.0,   // Mid peak
    2000.0,  // Upper-mid peak
    6000.0,  // Presence peak
    12000.0, // High shelf
];

/// Q factors for each filter band
const BAND_Q: [f32; NUM_BANDS] = [
    Q_BUTTERWORTH_F32, // Low shelf
    1.0,               // Low-mid peak
    SQRT_2,            // Mid peak
    1.2,               // Upper-mid peak
    1.5,               // Presence peak
    Q_BUTTERWORTH_F32, // High shelf
];

/// Calculate required SPL for target loudness level at frequency
fn calculate_target_spl(frequency: f32, phon: f32) -> f32 {
    // Find nearest frequency indices
    let idx = FREQUENCIES
        .iter()
        .position(|&f| f >= frequency)
        .unwrap_or(FREQUENCIES.len() - 1);
    let idx_low = if idx == 0 { 0 } else { idx - 1 };

    // Interpolate parameters
    let f1 = FREQUENCIES[idx_low];
    let f2 = FREQUENCIES[idx];
    let t = if 2.0 * (f1 - f2).abs() <= f32::EPSILON * (f1.abs() + f2.abs()) {
        0.0
    } else {
        (frequency - f1) / (f2 - f1)
    };

    let alpha_f = ALPHA_F[idx_low] + t * (ALPHA_F[idx] - ALPHA_F[idx_low]);
    let lu_f = L_U[idx_low] + t * (L_U[idx] - L_U[idx_low]);
    let tf_f = T_F[idx_low] + t * (T_F[idx] - T_F[idx_low]);

    // Inverse of ISO 226:2013 equation
    let a_f = LOUDNESS_SCALE * (10.0_f32.powf(0.025 * phon) - 1.15)
        + (0.4 * 10.0_f32.powf((tf_f + lu_f) / 10.0 - 9.0)).powf(alpha_f);

    (10.0 / alpha_f) * f32::log10(a_f) - lu_f + REF_SPL
}

/// Multi-band equal loudness filter implementing ISO 226:2013
///
/// Implements equal-loudness compensation through:
/// * Six optimally placed filter bands targeting critical frequencies
/// * Dynamic filter gain adjustment based on listening level
/// * ISO 226:2013 equal-loudness contours
/// * Phase-optimized IIR filters
#[derive(Debug, Clone)]
pub struct EqualLoudnessFilter {
    /// Fixed bank of 6 biquad filters for frequency bands:
    /// [30Hz LS, 100Hz, 500Hz, 2kHz, 6kHz, 12kHz HS]
    filters: [DirectForm1<f32>; NUM_BANDS],
    /// Current volume level (0.0 to 1.0)
    volume: f32,
    /// Sample rate in Hz
    sample_rate: SampleRate,
    /// Target loudness level in LUFS
    lufs_target: f32,
}

impl EqualLoudnessFilter {
    /// Creates a new equal loudness filter for the given sample rate
    ///
    /// # Arguments
    ///
    /// * `sample_rate` - The audio sample rate in Hz
    /// * `lufs_target` - Target loudness level in LUFS (typically -15.0)
    /// * `volume` - Initial volume setting (0.0 to 1.0)
    ///
    /// # Panics
    ///
    /// Panics if unable to create filter coefficients for the given sample rate.
    /// This should only happen if the sample rate is 0 Hz.
    #[must_use]
    pub fn new(sample_rate: SampleRate, lufs_target: f32, volume: f32) -> Self {
        let phon = Self::calculate_phon(volume, lufs_target);

        let mut filter = Self {
            filters: [(); NUM_BANDS].map(|()| {
                DirectForm1::<f32>::new(
                    Coefficients::<f32>::from_params(
                        Type::PeakingEQ(0.0),
                        sample_rate.hz(),
                        1000.0.hz(),
                        1.0,
                    )
                    .expect("failed to create filter coefficients"),
                )
            }),
            sample_rate,
            lufs_target,
            volume,
        };

        filter.filters = std::array::from_fn(|band| filter.create_filters_for_phon(band, phon));
        filter
    }

    /// Maps volume and LUFS target to corresponding phon level
    ///
    /// Converts the current listening level to phons for equal-loudness curve selection.
    /// Results are clamped to the valid range (0-100 phons) defined in ISO 226:2013.
    fn calculate_phon(volume: f32, lufs_target: f32) -> f32 {
        // Map volume to phon level for equal-loudness curve selection
        let listening_level = REFERENCE_SPL + lufs_target;
        (listening_level * volume).clamp(0.0, 100.0)
    }

    /// Updates filter coefficients when volume changes
    ///
    /// Recalculates all filter gains to maintain proper equal-loudness compensation
    /// at the new listening level. Only updates if volume has changed significantly.
    pub fn update_volume(&mut self, volume: f32) {
        if 2.0 * (volume - self.volume).abs() > f32::EPSILON * (volume.abs() + self.volume.abs()) {
            let phon = Self::calculate_phon(volume, self.lufs_target);
            self.filters = std::array::from_fn(|band| self.create_filters_for_phon(band, phon));
            self.volume = volume;
        }
    }

    /// Processes one audio sample through the filter bank
    ///
    /// Applies equal-loudness compensation through all filter bands in sequence.
    /// Does not apply volume scaling - that happens separately in the dithering stage.
    #[inline]
    pub fn process(&mut self, input: f32) -> f32 {
        let mut output = input;
        for filter in &mut self.filters {
            output = filter.run(output);
        }
        output
    }

    /// Creates filters for a specific frequency band at given phon level
    ///
    /// Calculates filter gains by comparing equal-loudness contours at:
    /// * Current listening level (phon)
    /// * Reference level (`REFERENCE_SPL` + `lufs_target`)
    ///
    /// Uses only the relative shape difference to maintain proper volume scaling.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// * Given band index is out of range (must be < `NUM_BANDS`)
    /// * Unable to create filter coefficients for the current sample rate
    fn create_filters_for_phon(&self, band: usize, phon: f32) -> DirectForm1<f32> {
        let freq = BAND_FREQUENCIES[band];
        let q = BAND_Q[band];

        // Get the response curves at our current and reference listening levels
        let target_response = calculate_target_spl(freq, phon);
        let reference_response = calculate_target_spl(freq, REFERENCE_SPL + self.lufs_target);

        // Calculate relative gain needed to match the equal-loudness contour shape,
        // not the absolute level
        let shape_difference =
            (target_response - reference_response) - (phon - (REFERENCE_SPL + self.lufs_target));

        let filter_type = if band == 0 {
            Type::LowShelf(shape_difference)
        } else if band == NUM_BANDS - 1 {
            Type::HighShelf(shape_difference)
        } else {
            Type::PeakingEQ(shape_difference)
        };

        let coeffs =
            Coefficients::<f32>::from_params(filter_type, self.sample_rate.hz(), freq.hz(), q)
                .expect("failed to create filter coefficients");

        DirectForm1::<f32>::new(coeffs)
    }

    /// Resets internal filter states without changing coefficients
    ///
    /// When seeking in audio, the internal states of the biquad filters need to be cleared
    /// to prevent artifacts from previous audio data. The filter coefficients are maintained
    /// since the listening level hasn't changed.
    pub fn reset(&mut self) {
        for filter in &mut self.filters {
            filter.reset_state();
        }
    }
}
