//! High-quality audio dithering and noise shaping implementation.
//!
//! This module provides professional-grade audio processing through:
//! * Triangular Probability Density Function (TPDF) dithering for optimal noise characteristics
//! * Shibata noise shaping filters for psychoacoustic optimization
//! * Volume-aware dither scaling for dynamic range preservation
//!
//! # Dithering
//!
//! The dithering process:
//! * Applies when requantizing audio for DAC output
//! * Uses TPDF (triangular) dither for superior noise characteristics
//! * Scales dither amplitude based on volume for optimal dynamic range
//! * Includes DC offset compensation to convert truncation to rounding
//!
//! # Noise Shaping
//!
//! The module uses Shibata noise shaping filters optimized for different sample rates
//! and aggressiveness levels. These filters push quantization noise into less audible
//! frequencies based on human hearing characteristics.
//!
//! Available noise shaping profiles:
//! * Level 0: No shaping (plain TPDF dither) - safest, recommended for podcasts
//! * Level 1: Very mild shaping (~5 dB ultrasonic rise)
//! * Level 2: Mild shaping (~8 dB rise) - recommended default for most music
//! * Level 3: Moderate shaping (~12 dB rise) - can benefit classical/jazz/ambient
//! * Level 4-7: Aggressive shaping - not recommended due to high ultrasonic energy that may:
//!   - Stress tweeters and headphone drivers
//!   - Cause intermodulation distortion
//!   - Create fatiguing sound
//!
//! Supported sample rates:
//! * 44.1 kHz - Deezer's default streaming rate
//! * 48 kHz - Common for podcasts and live radio
//! * 22.05 kHz - May occur in spoken word content
//! * 88.2/96/192 kHz - Support for potential future high-resolution audio
//! * 8/11.025 kHz - Provided for completeness
//!
//! The noise shaping filters are optimized for each sample rate, with more aggressive
//! profiles (3-7) available for 44.1 and 48 kHz where they are most beneficial.
//! For other rates, fewer profiles are provided focusing on conservative shaping.
//!
//! # Implementation Details
//!
//! The processing pipeline:
//! 1. Applies volume scaling with headroom management
//! 2. Generates TPDF dither noise scaled to quantization step
//! 3. For noise shaping profiles 1-7:
//!    * Filters previous quantization errors using Shibata coefficients
//!    * Applies filtered error as pre-compensation
//! 4. Quantizes the signal while tracking new error
//! 5. Applies DC offset compensation
//!
//! The Shibata filter coefficients come from SSRC (Sample rate converter) by Naoki Shibata,
//! licensed under LGPL-2.1. They are carefully designed for optimal perceptual noise
//! distribution based on psychoacoustic research.

use std::{f32, sync::Arc, time::Duration};

use coeffs::{
    SHIBATA_8_ATH_A_0, SHIBATA_8_ATH_A_1, SHIBATA_11_ATH_A_0, SHIBATA_11_ATH_A_1,
    SHIBATA_22_ATH_A_0, SHIBATA_22_ATH_A_1, SHIBATA_96_ATH_A_0, SHIBATA_96_ATH_A_1,
    SHIBATA_96_ATH_A_2, SHIBATA_192_ATH_A_0, SHIBATA_192_ATH_A_1, SHIBATA_192_ATH_A_2,
    SHIBATA_882_ATH_A_0, SHIBATA_882_ATH_A_1, SHIBATA_882_ATH_A_2,
};
use rodio::{
    ChannelCount, Source,
    source::{SeekError, noise::WhiteTriangular},
};

use crate::{loudness::EqualLoudnessFilter, ringbuf::RingBuffer, volume::Volume};

/// Creates a new audio source with dithered volume control and optional noise shaping.
///
/// This function integrates professional-grade audio processing:
/// * TPDF (triangular) dithering for optimal noise characteristics
/// * Volume-aware dither scaling to preserve dynamic range
/// * Shibata noise shaping for psychoacoustic optimization
/// * Equal loudness compensation based on ISO 226:2013
/// * DC offset compensation to convert truncation to rounding
///
/// # Arguments
///
/// * `input` - The source audio stream
/// * `volume` - Volume control with optional dithering parameters
/// * `lufs_target` - Optional LUFS target for equal loudness compensation
/// * `noise_shaping_profile` - Noise shaping aggressiveness level:
///   - 0: No shaping (plain TPDF dither) - safest, recommended for podcasts
///   - 1: Very mild shaping (~5 dB ultrasonic rise)
///   - 2: Mild shaping (~8 dB rise) - recommended default for most music
///   - 3: Moderate shaping (~12 dB rise) - can benefit classical/jazz/ambient
///   - 4-7: Aggressive shaping - not recommended due to high ultrasonic energy that may:
///     - Stress tweeters and headphone drivers
///     - Cause intermodulation distortion
///     - Create fatiguing sound
///
/// # Sample Rate Support
///
/// Optimized noise shaping for common audio rates:
/// * 44.1 kHz - Deezer's default streaming rate (profiles 0-7)
/// * 48 kHz - Common for podcasts and live radio (profiles 0-7)
/// * 22.05 kHz - May occur in spoken word content (profiles 0-2)
/// * 88.2/96/192 kHz - Support for potential future high-resolution audio (profiles 0-2)
/// * 8/11.025 kHz - Provided for completeness (profiles 0-2)
///
/// For unsupported sample rates, noise shaping is automatically disabled (profile 0).
///
/// # Implementation Details
///
/// The processing pipeline:
/// 1. Applies volume scaling with headroom management
/// 2. Generates TPDF dither noise scaled to quantization step
/// 3. For noise shaping profiles 1-7:
///    * Filters previous quantization errors using Shibata coefficients
///    * Applies filtered error as pre-compensation
/// 4. Quantizes the signal while tracking new error
/// 5. Applies DC offset compensation
///
/// The actual filter used depends on both the sample rate and chosen profile,
/// with coefficients optimized for each combination.
#[expect(clippy::too_many_lines)]
pub fn dithered_volume<I>(
    input: I,
    volume: Arc<Volume>,
    lufs_target: Option<f32>,
    noise_shaping_profile: u8,
) -> Box<dyn Source<Item = I::Item> + Send>
where
    I: Source + Send + 'static,
{
    use coeffs::{
        SHIBATA_48_ATH_A_0, SHIBATA_48_ATH_A_1, SHIBATA_48_ATH_A_2, SHIBATA_48_ATH_A_3,
        SHIBATA_48_ATH_A_4, SHIBATA_48_ATH_A_5, SHIBATA_48_ATH_A_6, SHIBATA_441_ATH_A_0,
        SHIBATA_441_ATH_A_1, SHIBATA_441_ATH_A_2, SHIBATA_441_ATH_A_3, SHIBATA_441_ATH_A_4,
        SHIBATA_441_ATH_A_5, SHIBATA_441_ATH_A_6,
    };

    let sample_rate = input.sample_rate();
    if noise_shaping_profile == 0 {
        debug!("noise shaping profile: disabled");
    } else {
        debug!("noise shaping profile: {}", noise_shaping_profile.min(7));

        if ![
            8_000, 11_025, 22_050, 44_100, 48_000, 88_200, 96_000, 192_000,
        ]
        .contains(&sample_rate)
        {
            warn!("noise shaping not available for {sample_rate} Hz");
        } else if noise_shaping_profile > 2 && ![44_100, 48_000].contains(&sample_rate) {
            warn!("limiting noise shaping profile to 2 (highest available for {sample_rate} Hz)");
        }
    }

    let equal_loudness =
        lufs_target.map(|target| EqualLoudnessFilter::new(sample_rate, target, volume.volume()));

    match (sample_rate, noise_shaping_profile) {
        (_, 0) => Box::new(DitheredVolume::<I, 0> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &[],
        }),
        (44_100, 1) => Box::new(DitheredVolume::<I, 12> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_0,
        }),
        (44_100, 2) => Box::new(DitheredVolume::<I, 12> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_1,
        }),
        (44_100, 3) => Box::new(DitheredVolume::<I, 24> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_2,
        }),
        (44_100, 4) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_3,
        }),
        (44_100, 5) => Box::new(DitheredVolume::<I, 20> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_4,
        }),
        (44_100, 6) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_5,
        }),
        (44_100, _) => Box::new(DitheredVolume::<I, 20> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_6,
        }),
        (48_000, 1) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_0,
        }),
        (48_000, 2) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_1,
        }),
        (48_000, 3) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_2,
        }),
        (48_000, 4) => Box::new(DitheredVolume::<I, 19> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_3,
        }),
        (48_000, 5) => Box::new(DitheredVolume::<I, 28> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_4,
        }),
        (48_000, 6) => Box::new(DitheredVolume::<I, 20> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_5,
        }),
        (48_000, _) => Box::new(DitheredVolume::<I, 28> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_6,
        }),
        (88_200, 1) => Box::new(DitheredVolume::<I, 24> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_882_ATH_A_0,
        }),
        (88_200, 2) => Box::new(DitheredVolume::<I, 32> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_882_ATH_A_1,
        }),
        (88_200, _) => Box::new(DitheredVolume::<I, 20> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_882_ATH_A_2,
        }),
        (96_000, 1) => Box::new(DitheredVolume::<I, 32> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_96_ATH_A_0,
        }),
        (96_000, 2) => Box::new(DitheredVolume::<I, 24> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_96_ATH_A_1,
        }),
        (96_000, _) => Box::new(DitheredVolume::<I, 31> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_96_ATH_A_2,
        }),
        (192_000, 1) => Box::new(DitheredVolume::<I, 20> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_192_ATH_A_0,
        }),
        (192_000, 2) => Box::new(DitheredVolume::<I, 43> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_192_ATH_A_1,
        }),
        (192_000, _) => Box::new(DitheredVolume::<I, 54> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_192_ATH_A_2,
        }),
        (8_000, 1) => Box::new(DitheredVolume::<I, 8> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_8_ATH_A_0,
        }),
        (8_000, _) => Box::new(DitheredVolume::<I, 7> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_8_ATH_A_1,
        }),
        (11_025, 1) => Box::new(DitheredVolume::<I, 8> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_11_ATH_A_0,
        }),
        (11_025, _) => Box::new(DitheredVolume::<I, 6> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_11_ATH_A_1,
        }),
        (22_050, 1) => Box::new(DitheredVolume::<I, 7> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_22_ATH_A_0,
        }),
        (22_050, _) => Box::new(DitheredVolume::<I, 12> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_22_ATH_A_1,
        }),
        _ => Box::new(DitheredVolume::<I, 0> {
            input,
            volume,
            equal_loudness,
            noise: WhiteTriangular::new(sample_rate),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &[],
        }),
    }
}

/// Audio source with integrated dithering, noise shaping and volume control.
///
/// Processes audio samples in this order:
/// 1. Optional equal-loudness compensation (ISO 226:2013)
/// 2. When quantization is needed:
///    * Generates TPDF dither noise at quantization step size
///    * For noise shaping (N>0):
///      - Applies filtered error feedback from previous samples
///      - Reduced dither amplitude due to noise shaping linearization
///    * Quantizes signal and tracks error if noise shaping enabled
///    * Adds DC offset compensation
/// 3. Applies volume scaling
///
/// The type parameter N determines the noise shaping filter length,
/// which varies by sample rate and chosen profile level. N=0 disables
/// noise shaping for optimal performance when not needed.
#[derive(Debug, Clone)]
pub struct DitheredVolume<I, const N: usize> {
    /// The underlying audio source
    input: I,

    /// Volume control with dithering parameters
    volume: Arc<Volume>,

    /// Noise generator for dither
    noise: WhiteTriangular,

    /// Ring buffer storing previous quantization errors for noise shaping
    quantization_error_history: RingBuffer<N>,

    /// Shibata filter coefficients for the current sample rate and profile
    filter_coefficients: &'static [f32; N],

    /// Optional equal loudness compensation filter
    equal_loudness: Option<EqualLoudnessFilter>,
}

impl<I, const N: usize> DitheredVolume<I, N>
where
    I: Source,
{
    /// Returns a reference to the underlying audio source.
    #[inline]
    pub fn inner(&self) -> &I {
        &self.input
    }

    /// Returns a mutable reference to the underlying audio source.
    #[inline]
    pub fn inner_mut(&mut self) -> &mut I {
        &mut self.input
    }

    /// Consumes self and returns the underlying audio source.
    #[inline]
    pub fn into_inner(self) -> I {
        self.input
    }
}

impl<I, const N: usize> Iterator for DitheredVolume<I, N>
where
    I: Source,
{
    type Item = I::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        /// Dither amplitude scaling factor when noise shaping is enabled.
        /// Reduced by 6 dB compared to plain dithering since noise shaping
        /// provides additional linearization.
        const NOISE_SHAPING_DITHER_AMPLITUDE: f32 = 0.5;

        self.input.next().map(|mut sample| {
            let volume = self.volume.volume();

            // Apply equal loudness compensation if enabled, without volume scaling
            if let Some(equal_loudness) = self.equal_loudness.as_mut() {
                equal_loudness.update_volume(volume);
                sample = equal_loudness.process(sample);
            }

            if let Some(quantization_step) = self.volume.quantization_step() {
                // Calculate dither at the right bit depth
                let dither = self.noise.next().unwrap_or_default() * quantization_step;

                // Fast path for no noise shaping (N=0)
                if N == 0 {
                    sample = quantize(sample + dither, quantization_step);
                } else {
                    // Noise shaping path
                    let mut filtered_error = 0.0;
                    for i in 0..N {
                        filtered_error +=
                            self.filter_coefficients[i] * self.quantization_error_history.get(i);
                    }

                    let shaped = sample + filtered_error + NOISE_SHAPING_DITHER_AMPLITUDE * dither;

                    // Quantize and track error for noise shaping
                    let dithered = quantize(shaped, quantization_step);
                    self.quantization_error_history.push(dithered - shaped);
                    sample = dithered;
                }

                sample += DC_COMPENSATION * quantization_step;
            }

            sample * volume
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.input.size_hint()
    }
}

impl<I, const N: usize> Source for DitheredVolume<I, N>
where
    I: Source,
{
    /// Number of samples remaining in the current processing block.
    #[inline]
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    /// Channel count of the audio source.
    #[inline]
    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }

    /// Current sample rate in Hz.
    #[inline]
    fn sample_rate(&self) -> u32 {
        self.input.sample_rate()
    }

    /// Total duration of the audio source, if known.
    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }

    /// Attempts to seek to the specified position.
    /// Also resets the noise shaping error history when successful.
    #[inline]
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        let result = self.input.try_seek(pos);
        if result.is_ok() {
            self.quantization_error_history.reset();
            if let Some(equal_loudness) = &mut self.equal_loudness {
                equal_loudness.reset();
            }
        }
        result
    }
}

/// DC offset compensation value (0.5) used to shift truncation points.
/// This helps convert truncation behavior to be more like rounding,
/// though for negative values an additional correction is still needed.
pub(crate) const DC_COMPENSATION: f32 = 0.5;

/// Quantizes a signal to the nearest step value, using truncation with compensation for negative values.
///
/// The quantization process:
/// 1. Applies DC offset compensation (0.5) to shift the truncation points
/// 2. Truncates to nearest lower quantization step
/// 3. For negative signals, subtracts one quantization step to correct truncation bias
///
/// # Arguments
///
/// * `signal` - The input signal value
/// * `quantization_step` - The size of each quantization level
///
/// # Returns
///
/// The quantized signal value, adjusted for truncation bias on negative values
#[inline]
#[must_use]
fn quantize(signal: f32, quantization_step: f32) -> f32 {
    // Quantize with DC offset compensation
    let quantized = (signal / quantization_step + DC_COMPENSATION).trunc() * quantization_step;
    if signal < 0.0 {
        quantized - quantization_step
    } else {
        quantized
    }
}

/// Shibata noise shaping filter coefficients optimized for different sample rates.
///
/// These coefficients are from SSRC (Sample rate converter) by Naoki Shibata,
/// licensed under LGPL-2.1. They are designed for optimal perceptual noise
/// distribution based on psychoacoustic research.
mod coeffs {
    /// Minimal noise shaping filter for 44.1 kHz (12 coefficients)
    pub const SHIBATA_441_ATH_A_0: [f32; 12] = [
        -0.595_437_8,
        0.002_507_873,
        0.185_180_59,
        0.001_037_429_3,
        0.103_663_43,
        0.053_248_63,
        8.403_005e-5,
        3.856_993_3e-8,
        0.026_413_01,
        0.000_684_383_97,
        -3.158_050_5e-6,
        -0.031_739_63,
    ];

    /// Conservative noise shaping filter for 44.1 kHz (12 coefficients)
    pub const SHIBATA_441_ATH_A_1: [f32; 12] = [
        -0.998_202,
        0.599_515_4,
        -0.081_278_324,
        -9.297_739_6e-5,
        0.202_520_61,
        0.024_194_805,
        -0.000_902_274_33,
        0.045_577_545,
        0.044_477_824,
        0.003_068_177_7,
        0.000_169_364_5,
        6.856_103_6e-7,
    ];

    /// Balanced noise shaping filter for 44.1 kHz (24 coefficients) - recommended default
    pub const SHIBATA_441_ATH_A_2: [f32; 24] = [
        -1.356_863_9,
        1.225_293_5,
        -0.623_555_06,
        0.225_620_94,
        0.235_579_76,
        -0.135_363_62,
        0.091_538_146,
        0.056_445_64,
        -3.961_442_4e-5,
        0.023_561_92,
        0.010_756_319,
        0.000_319_491_32,
        -0.001_433_762,
        0.008_455_124,
        0.000_213_181_8,
        -7.617_592e-5,
        -0.001_010_233_1,
        -4.503_027_6e-5,
        -0.001_343_382_2,
        -0.001_393_724_2,
        -0.000_433_067,
        -0.000_469_497_87,
        -0.000_147_758_42,
        4.106_017_5e-5,
    ];

    /// Aggressive noise shaping filter for 44.1 kHz (16 coefficients)
    pub const SHIBATA_441_ATH_A_3: [f32; 16] = [
        -1.771_483_5,
        2.160_381_3,
        -1.851_221_2,
        1.345_941_7,
        -0.523_564_6,
        0.159_801_14,
        0.079_563_4,
        -0.017_584_056,
        0.039_745_52,
        0.021_822_928,
        0.007_233_896_3,
        0.000_838_793_1,
        0.009_479_233,
        0.006_856_449_4,
        -0.000_395_254_84,
        0.004_087_016_5,
    ];

    /// Very aggressive noise shaping filter for 44.1 kHz (20 coefficients)
    pub const SHIBATA_441_ATH_A_4: [f32; 20] = [
        -2.155_173,
        3.148_202_7,
        -3.420_880_6,
        3.134_365_6,
        -2.155_232_4,
        1.269_854,
        -0.503_365_93,
        0.164_644_7,
        0.013_838_038,
        0.006_250_574_3,
        -0.004_169_150_7,
        0.013_679_159,
        0.002_451_622,
        -0.000_244_074_65,
        0.005_245_24,
        0.000_420_199_27,
        -0.000_413_520_15,
        -0.000_163_229_36,
        0.000_473_211_02,
        -0.000_932_779_86,
    ];

    /// Extremely aggressive noise shaping filter for 44.1 kHz (16 coefficients)
    pub const SHIBATA_441_ATH_A_5: [f32; 16] = [
        -2.509_607_6,
        4.251_982,
        -5.479_231_4,
        5.972_496,
        -5.294_708_3,
        4.066_418,
        -2.524_713_3,
        1.303_939_9,
        -0.446_136_62,
        0.097_044_02,
        0.016_150_594,
        0.006_091_615,
        -0.013_266_252,
        0.017_414_96,
        -0.000_799_034_66,
        -7.114_16e-7,
    ];

    /// Maximum aggression noise shaping filter for 44.1 kHz (20 coefficients)
    pub const SHIBATA_441_ATH_A_6: [f32; 20] = [
        -2.826_326_6,
        5.353_436,
        -7.804_206,
        9.679_369,
        -10.157_135,
        9.439_996,
        -7.614_612_6,
        5.424_517_6,
        -3.247_828_2,
        1.630_185_2,
        -0.585_380_2,
        0.117_100_02,
        0.033_543_67,
        -0.008_884_147,
        -0.017_314_358,
        0.033_262_73,
        -0.018_168_22,
        0.006_801_503,
        0.000_969_119_5,
        -0.000_964_893_44,
    ];

    /// Minimal noise shaping filter for 48 kHz (16 coefficients)
    pub const SHIBATA_48_ATH_A_0: [f32; 16] = [
        -0.648_154_4,
        0.000_132_923_29,
        0.152_844_4,
        0.024_795_081,
        0.028_879_294,
        0.097_741_306,
        -3.723_334_5e-5,
        -3.036_181_6e-6,
        2.685_151_8e-5,
        0.015_118_856,
        0.000_119_081_56,
        -4.020_391_8e-6,
        -0.032_142_308,
        -1.210_869_2e-6,
        0.0,
        -2.413_082e-9,
    ];

    /// Conservative noise shaping filter for 48 kHz (16 coefficients)
    pub const SHIBATA_48_ATH_A_1: [f32; 16] = [
        -1.037_501_5,
        0.555_852_53,
        6.200_925_4e-5,
        -0.054_276_78,
        0.140_640_74,
        0.107_340_67,
        -6.741_447e-6,
        -0.000_905_077_85,
        0.071_966_76,
        0.018_717_75,
        0.003_851_746_4,
        0.005_743_284_7,
        0.001_160_279_5,
        0.000_235_626_47,
        6.177_044e-5,
        0.001_676_786_5,
    ];

    /// Balanced noise shaping filter for 48 kHz (16 coefficients) - recommended default
    pub const SHIBATA_48_ATH_A_2: [f32; 16] = [
        -1.491_957_8,
        1.308_917_9,
        -0.540_516_3,
        0.000_361_137_5,
        0.363_031_95,
        -0.109_111_28,
        -0.007_310_638,
        0.115_459_144,
        -0.003_772_285_5,
        0.012_545_259,
        0.029_272_487,
        0.005_002_2,
        0.000_202_188_52,
        0.004_905_734_7,
        0.005_127_976,
        0.002_505_671,
    ];

    /// Aggressive noise shaping filter for 48 kHz (19 coefficients)
    pub const SHIBATA_48_ATH_A_3: [f32; 19] = [
        -1.960_159_2,
        2.406_054_7,
        -1.948_885,
        1.162_663_9,
        -0.252_979_22,
        -0.031_299_483,
        0.112_349_72,
        0.028_672_902,
        -0.008_408_587,
        0.040_343_3,
        0.014_730_193,
        0.008_152_652,
        0.000_781_101_6,
        0.010_703_167,
        0.007_504_583,
        -6.789_937_5e-5,
        0.004_595_272_7,
        0.001_568_543_5,
        8.033_391e-5,
    ];

    /// Very aggressive noise shaping filter for 48 kHz (28 coefficients)
    pub const SHIBATA_48_ATH_A_4: [f32; 28] = [
        -2.421_972_8,
        3.637_804_5,
        -3.875_656_8,
        3.201_990_8,
        -1.846_927_2,
        0.761_118,
        -0.083_762_63,
        -0.064_117_65,
        0.066_511_706,
        0.011_620_322,
        0.000_896_770_16,
        -0.003_890_886_4,
        0.011_067_332,
        0.001_639_363_5,
        -0.002_100_992_2,
        0.003_973_741,
        0.000_649_898_43,
        -0.000_642_979_8,
        -0.001_001_957_3,
        0.000_249_402_95,
        -0.000_204_617_04,
        -0.001_489_691_3,
        3.696_430_6e-5,
        -5.559_246e-5,
        -0.000_221_960_55,
        -0.000_119_191_81,
        0.000_217_847_82,
        5.855_179e-5,
    ];

    /// Extremely aggressive noise shaping filter for 48 kHz (20 coefficients)
    pub const SHIBATA_48_ATH_A_5: [f32; 20] = [
        -2.846_033_3,
        5.035_543,
        -6.492_711,
        6.668_969,
        -5.342_242_7,
        3.433_106,
        -1.591_353_8,
        0.482_101_38,
        0.008_463_773,
        -0.035_323_333,
        0.005_527_129,
        0.021_560_267,
        0.006_101_152,
        -0.009_066_052,
        0.010_759_642,
        0.004_644_123,
        -0.002_885_128_1,
        0.002_711_836,
        0.000_833_278_1,
        -6.372_233e-5,
    ];

    /// Maximum aggression noise shaping filter for 48 kHz (28 coefficients)
    pub const SHIBATA_48_ATH_A_6: [f32; 28] = [
        -3.260_151_6,
        6.557_569_5,
        -9.748_665,
        11.713_089,
        -11.504_628,
        9.485_963,
        -6.404_273,
        3.477_282,
        -1.332_738_3,
        0.264_645_76,
        0.081_823_304,
        -0.044_643_41,
        -0.021_642_473,
        0.042_832_12,
        -0.003_383_262,
        -0.016_050_559,
        0.019_443_769,
        -0.002_014_045_6,
        -0.005_101_846_5,
        0.004_944_144_3,
        0.001_399_693_9,
        -0.003_581_012,
        0.002_209_919_7,
        0.000_101_200_05,
        -0.000_771_208_67,
        4.772_755e-5,
        0.000_470_578_76,
        -0.000_535_220_14,
    ];

    /// Minimal noise shaping filter for 88.2 kHz (24 coefficients)
    pub const SHIBATA_882_ATH_A_0: [f32; 24] = [
        -0.812_750_8,
        -1.341_541_6e-7,
        1.400_317e-5,
        0.027_366_659,
        0.063_084_796,
        0.000_411_249_64,
        0.001_466_781_1,
        0.003_463_642_4,
        0.014_447_952,
        0.050_686_4,
        0.000_316_579_54,
        7.608_178e-7,
        -1.339_193_5e-6,
        -1.108_497_8e-6,
        -2.345_899_2e-7,
        -7.197_047_4e-9,
        0.000_240_975_3,
        0.000_813_391_8,
        0.002_707_262_8,
        1.228_903e-5,
        2.408_082e-6,
        -2.651_654_8e-6,
        -0.022_208_367,
        -1.809_095_4e-7,
    ];

    /// Conservative noise shaping filter for 88.2 kHz (32 coefficients)
    pub const SHIBATA_882_ATH_A_1: [f32; 32] = [
        -1.175_952_2,
        0.004_028_124,
        0.470_744_13,
        0.000_516_334_9,
        -0.034_613_37,
        -0.090_879_366,
        2.494_357_4e-5,
        0.040_280_38,
        0.084_476_25,
        0.020_952_063,
        -6.424_727e-5,
        -0.015_425_831,
        -0.000_348_468_02,
        0.000_214_603_9,
        0.038_064_55,
        0.007_522_898,
        0.000_105_720_07,
        0.000_888_932_85,
        0.005_120_796,
        0.004_709_166_5,
        0.001_308_845_1,
        0.001_061_635_2,
        5.314_624e-5,
        2.692_748_4e-5,
        -7.112_140_3e-6,
        -3.788_061_4e-5,
        0.000_150_480_61,
        0.001_454_448_3,
        0.000_337_949_87,
        0.000_629_115_85,
        1.767_152e-8,
        -1.289_341_8e-7,
    ];

    /// Strong noise shaping filter for 88.2 kHz (20 coefficients)
    pub const SHIBATA_882_ATH_A_2: [f32; 20] = [
        -2.075_203_7,
        1.431_611_1,
        4.101_862_2e-5,
        -0.307_477_86,
        -0.015_034_948,
        0.002_069_007_4,
        0.095_445_45,
        0.017_573_366,
        -0.001_514_684_4,
        -0.009_715_72,
        -0.003_230_015_7,
        0.001_166_222_2,
        0.012_702_43,
        0.013_680_535,
        0.000_326_957_12,
        0.000_334_812_4,
        -0.001_941_892,
        0.006_559_844_6,
        0.003_184_868_5,
        0.001_185_707_6,
    ];

    /// Minimal noise shaping filter for 96 kHz (32 coefficients)
    pub const SHIBATA_96_ATH_A_0: [f32; 32] = [
        -0.833_627_8,
        -4.766_351e-7,
        5.592_720_5e-5,
        0.000_917_676_1,
        0.085_019_3,
        0.000_308_640_97,
        2.747_484_9e-5,
        3.447_055_5e-5,
        0.006_816_617,
        0.005_103_240_3,
        0.048_310_29,
        3.419_442_5e-6,
        3.938_738_8e-8,
        -5.229_683e-6,
        -2.181_512_5e-5,
        -5.806_052_7e-6,
        -8.897_533e-6,
        2.879_307_4e-6,
        1.014_230_3e-5,
        0.000_883_434_84,
        6.652_17e-5,
        4.303_244_6e-7,
        -1.557_321e-6,
        -0.003_246_902_5,
        -0.013_371_953,
        -0.001_669_709_6,
        -0.000_337_457_5,
        -3.821_846_6e-5,
        -8.088_396e-5,
        -1.763_109_3e-5,
        -4.731_759e-6,
        -3.815_073_3e-7,
    ];

    /// Conservative noise shaping filter for 96 kHz (24 coefficients)
    pub const SHIBATA_96_ATH_A_1: [f32; 24] = [
        -1.226_027_3,
        0.001_465_178_8,
        0.485_106_86,
        0.000_164_458_08,
        -3.713_618_6e-5,
        -0.114_801_206,
        0.000_458_874_57,
        0.001_798_167_5,
        0.077_026_084,
        0.040_43,
        -4.641_455e-5,
        -0.000_400_403_28,
        -0.000_134_071_32,
        -0.003_484_065_2,
        0.000_485_598_9,
        0.019_503_146,
        0.017_391_57,
        0.001_972_813_2,
        8.821_947_4e-7,
        0.000_649_276_2,
        0.004_914_358,
        0.002_303_595_2,
        0.000_637_523_83,
        0.000_761_010_3,
    ];

    /// Strong noise shaping filter for 96 kHz (31 coefficients)
    pub const SHIBATA_96_ATH_A_2: [f32; 31] = [
        -2.104_111_4,
        1.410_141_7,
        0.003_514_738_8,
        -0.186_179_71,
        -0.111_176_77,
        0.001_362_945_1,
        0.055_446_718,
        0.056_859_914,
        0.003_957_323_3,
        -0.002_566_334_8,
        -0.014_090_753,
        -0.006_225_708_4,
        0.006_539_735,
        0.019_066_527,
        0.003_569_579_2,
        0.001_226_439_5,
        -0.000_114_401_024,
        0.000_198_087_28,
        0.003_230_664_9,
        0.004_677_78,
        0.001_040_733_2,
        0.000_973_290_94,
        0.000_780_345_5,
        0.000_388_532_27,
        -4.194_729_6e-5,
        -0.000_172_955_4,
        0.000_593_151_9,
        0.000_697_247_86,
        0.000_504_023_1,
        0.000_376_237_06,
        0.000_174_400_05,
    ];

    /// Minimal noise shaping filter for 192 kHz (20 coefficients)
    pub const SHIBATA_192_ATH_A_0: [f32; 20] = [
        -0.929_867_86,
        -2.375_700_4e-6,
        -1.323_920_4e-6,
        -4.533_644_6e-8,
        1.085_569_9e-6,
        7.519_394_7e-7,
        0.010_574_714,
        0.015_397_379,
        0.007_173_464_6,
        0.004_041_632_6,
        0.000_315_436_2,
        6.079_085e-6,
        2.561_475_2e-5,
        6.444_113_3e-6,
        0.000_143_420_2,
        9.988_663e-9,
        0.000_110_015_65,
        0.000_264_444_04,
        0.018_070_342,
        0.013_997_578,
    ];

    /// Conservative noise shaping filter for 192 kHz (43 coefficients)
    pub const SHIBATA_192_ATH_A_1: [f32; 43] = [
        -1.331_289_8,
        -0.000_433_482_64,
        0.004_089_822_5,
        0.459_057_36,
        0.000_301_990_48,
        -2.536_581_6e-5,
        0.000_537_581,
        -0.000_102_941_34,
        -0.123_569_97,
        3.465_125e-5,
        -0.001_193_787_9,
        0.000_649_263_8,
        0.008_296_027,
        0.031_590_05,
        0.001_986_456_6,
        0.005_588_731_7,
        0.004_601_458_6,
        0.004_578_168,
        5.671_228e-5,
        -0.000_111_495_31,
        -0.000_282_925_8,
        -0.000_341_488_3,
        -0.003_357_38,
        -0.002_394_140_7,
        0.000_414_986_57,
        6.498_299_6e-5,
        0.000_906_114_76,
        0.004_790_787,
        0.003_744_423_2,
        0.000_167_221_17,
        0.001_175_894_5,
        0.000_812_370_5,
        1.124_187_4e-5,
        0.000_116_574_41,
        0.000_147_670_6,
        1.753_169_6e-5,
        0.000_274_619,
        0.000_739_063_8,
        0.000_382_722_04,
        0.000_682_175_16,
        0.001_057_418_4,
        1.561_514_8e-6,
        2.158_449_8e-5,
    ];

    /// Strong noise shaping filter for 192 kHz (54 coefficients)
    pub const SHIBATA_192_ATH_A_2: [f32; 54] = [
        -2.117_482_7,
        0.793_001_3,
        0.588_716_5,
        0.004_517_062,
        2.240_059_6e-5,
        -0.349_810_66,
        -0.001_467_47,
        0.035_286_05,
        0.030_574_916,
        0.008_099_925,
        0.024_920_885,
        0.010_276_389,
        0.002_827_338_9,
        -0.011_965_872,
        0.001_178_735_8,
        -0.001_587_570_1,
        -0.001_221_955_2,
        -0.004_150_979,
        -0.000_236_603_75,
        0.000_234_691_37,
        -0.000_245_441_04,
        0.002_350_530_4,
        0.001_063_528,
        0.002_193_444_6,
        -0.000_186_019_5,
        0.000_534_442_1,
        0.000_564_826_8,
        6.555_315e-5,
        -0.000_503_513_44,
        -0.000_697_769_34,
        -0.000_215_430_79,
        -0.000_558_842_85,
        0.000_955_912_34,
        0.000_183_239_64,
        0.001_184_735,
        -5.595_707_6e-5,
        0.000_210_925_91,
        -9.261_416e-6,
        1.689_312_6e-5,
        0.000_102_918_99,
        8.705_23e-6,
        2.189_383_8e-5,
        -2.048_334_9e-5,
        9.314_836e-5,
        5.457_198_5e-5,
        -1.039_314_8e-5,
        4.186_463e-5,
        -3.314_268e-5,
        -4.641_25e-7,
        3.169_075_7e-5,
        -2.919_960_4e-5,
        4.137_143e-5,
        -3.097_004_5e-6,
        0.000_130_819_72,
    ];

    /// Minimal noise shaping filter for 22.05 kHz (7 coefficients)
    pub const SHIBATA_22_ATH_A_0: [f32; 7] = [
        -0.246_904_97,
        0.405_607_25,
        0.178_049_43,
        0.122_181_155,
        0.044_338_007,
        -7.425_220_7e-6,
        -0.002_911_780_5,
    ];

    /// Strong noise shaping filter for 22.05 kHz (12 coefficients)
    pub const SHIBATA_22_ATH_A_1: [f32; 12] = [
        -0.091_535_78,
        0.537_626_7,
        0.366_641_07,
        0.295_497_2,
        0.252_406_15,
        0.145_342_65,
        0.120_994_06,
        0.089_816_33,
        0.049_367_57,
        0.030_478_563,
        0.012_090_771,
        0.008_203_126_5,
    ];

    /// Minimal noise shaping filter for 8 kHz (8 coefficients)
    pub const SHIBATA_8_ATH_A_0: [f32; 8] = [
        0.761_596_44,
        0.194_095_1,
        -0.035_044_946,
        -2.645_898e-7,
        0.003_829_823,
        -1.070_960_3e-6,
        -0.004_370_146,
        0.001_062_222_2,
    ];

    /// Strong noise shaping filter for 8 kHz (7 coefficients)
    pub const SHIBATA_8_ATH_A_1: [f32; 7] = [
        1.027_286_8,
        0.570_397_44,
        0.228_542_6,
        0.112_836_38,
        0.045_456_283,
        0.015_480_343,
        -1.829_845_8e-7,
    ];

    /// Minimal noise shaping filter for 11.025 kHz (8 coefficients)
    pub const SHIBATA_11_ATH_A_0: [f32; 8] = [
        0.535_948_5,
        0.433_847_1,
        8.257_924e-7,
        -0.002_352_862_2,
        -0.020_154_648,
        0.007_696_024,
        0.0,
        2.101_561_4e-8,
    ];

    /// Strong noise shaping filter for 11.025 kHz (6 coefficients)
    pub const SHIBATA_11_ATH_A_1: [f32; 6] = [
        0.776_428,
        0.736_269_1,
        0.286_170_18,
        0.149_260_58,
        0.035_679_143,
        0.016_192_988,
    ];
}
