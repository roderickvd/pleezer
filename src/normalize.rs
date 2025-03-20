//! Audio normalization through feedforward limiting.
//!
//! This module implements a feedforward limiter in the log domain, based on:
//! Giannoulis, D., Massberg, M., & Reiss, J.D. (2012). Digital Dynamic Range Compressor Design,
//! A Tutorial and Analysis. Journal of The Audio Engineering Society, 60, 399-408.
//!
//! Features:
//! * Soft-knee limiting for natural sound
//! * Decoupled peak detection per channel
//! * Coupled gain reduction across channels
//! * Configurable attack/release times
//! * CPU-efficient processing with:
//!   - Specialized mono implementation
//!   - Optimized stereo processing
//!   - Generic multi-channel support
//!
//! # Architecture
//!
//! The limiter processes audio in these steps:
//! 1. Initial gain stage
//! 2. Half-wave rectification and dB conversion
//! 3. Soft-knee gain computation (optimized for typical below-threshold case)
//! 4. Smoothed peak detection (per channel)
//! 5. Maximum peak detection across channels (specialized per channel count)
//! 6. Gain reduction application (coupled across channels)
//!
//! # Example
//!
//! ```no_run
//! use std::time::Duration;
//! use pleezer::normalize::normalize;
//!
//! // Configure limiter with typical values
//! let normalized = normalize(
//!     source,
//!     1.0,             // Unity gain
//!     -1.0,            // Threshold (dB)
//!     4.0,             // Knee width (dB)
//!     Duration::from_millis(5),    // Attack time
//!     Duration::from_millis(100),  // Release time
//! );
//! ```

use std::time::Duration;

use rodio::{Sample, Source, source::SeekError};

use crate::util::{self, ToF32, ZERO_DB};

/// Creates a normalized audio filter with configurable limiting.
///
/// The limiter processes each channel independently for envelope detection but applies gain
/// reduction uniformly across all channels to preserve imaging. Uses specialized implementations
/// for mono and stereo audio, with a generic implementation for multi-channel sources.
///
/// # Arguments
///
/// * `input` - Audio source to process
/// * `ratio` - Initial gain scaling (1.0 = unity, applied before limiting)
/// * `threshold` - Level where limiting begins (dB, negative for headroom)
///    Typical value: -1 to -2 dB to prevent clipping
/// * `knee_width` - Range over which limiting gradually increases (dB)
///    Wider knee = smoother transition into limiting
///    Typical value: 3-4 dB for musical transparency
/// * `attack` - Time to respond to level increases
///    Shorter = faster limiting but may distort
///    Longer = more transparent but may overshoot
///    Typical value: 5 ms for quick response
/// * `release` - Time to recover after level decreases
///    Shorter = faster recovery but may pump
///    Longer = smoother but may duck subsequent peaks
///    Typical value: 100 ms for natural decay
///
/// # Returns
///
/// A `Normalize` filter that processes the input audio through the limiter.
pub fn normalize<I>(
    input: I,
    ratio: f32,
    threshold: f32,
    knee_width: f32,
    attack: Duration,
    release: Duration,
) -> Normalize<I>
where
    I: Source,
    I::Item: Sample,
{
    let sample_rate = input.sample_rate();
    let attack = duration_to_coefficient(attack, sample_rate);
    let release = duration_to_coefficient(release, sample_rate);
    let channels = input.channels() as usize;

    let base = NormalizeBase::new(ratio, threshold, knee_width, attack, release);

    match channels {
        1 => Normalize::Mono(NormalizeMono {
            input,
            base,
            normalisation_integrator: ZERO_DB,
            normalisation_peak: ZERO_DB,
        }),
        2 => Normalize::Stereo(NormalizeStereo {
            input,
            base,
            normalisation_integrators: [ZERO_DB; 2],
            normalisation_peaks: [ZERO_DB; 2],
            position: 0,
        }),
        n => Normalize::MultiChannel(NormalizeMulti {
            input,
            base,
            normalisation_integrators: vec![ZERO_DB; n],
            normalisation_peaks: vec![ZERO_DB; n],
            position: 0,
        }),
    }
}

/// Converts a time duration to a smoothing coefficient.
///
/// Used for both attack and release filtering. Creates a coefficient that determines how quickly
/// the limiter responds to level changes:
/// * Longer times = higher coefficients = slower, smoother response
/// * Shorter times = lower coefficients = faster, more immediate response
///
/// Note: Coefficient is independent of channel count, making it suitable for all normalizer
/// variants.
///
/// # Arguments
///
/// * `duration` - Desired response time
/// * `sample_rate` - Audio sample rate in Hz
///
/// # Returns
///
/// Smoothing coefficient in the range [0.0, 1.0]
#[must_use]
fn duration_to_coefficient(duration: Duration, sample_rate: u32) -> f32 {
    f32::exp(-1.0 / (duration.as_secs_f32() * sample_rate.to_f32_lossy()))
}

/// Audio filter that applies normalization through feedforward limiting.
///
/// Processing stages:
/// 1. Initial gain scaling by `ratio`
/// 2. Peak detection above `threshold` (optimized for typical below-threshold case)
/// 3. Soft-knee limiting over `knee_width`
/// 4. Independent smoothing with `attack`/`release` filtering per channel
/// 5. Coupled gain reduction across all channels to preserve imaging
///
/// Uses specialized implementations:
/// * Mono: Direct single-channel processing
/// * Stereo: Optimized two-channel processing with efficient position tracking
/// * `MultiChannel`: Generic implementation for other channel counts
///
/// # Type Parameters
///
/// * `I` - Input audio source type
#[derive(Clone, Debug)]
pub enum Normalize<I>
where
    I: Source,
    I::Item: Sample,
{
    Mono(NormalizeMono<I>),
    Stereo(NormalizeStereo<I>),
    MultiChannel(NormalizeMulti<I>),
}

/// Common parameters and processing logic shared across all normalizer variants.
///
/// Handles:
/// * Parameter storage (ratio, threshold, knee width, attack/release)
/// * Per-channel state updates for peak detection
/// * Gain computation through soft-knee limiting
#[derive(Clone, Debug)]
struct NormalizeBase {
    /// Ratio of output to input level (1.0 = unity)
    ratio: f32,
    /// Level where limiting begins (dB)
    threshold: f32,
    /// Width of the soft-knee region (dB)
    knee_width: f32,
    /// Attack time constant (ms)
    attack: f32,
    /// Release time constant (ms)
    release: f32,
}

/// Mono channel normalizer optimized for single-channel processing
#[derive(Clone, Debug)]
pub struct NormalizeMono<I> {
    /// Input audio source
    input: I,
    /// Common normalizer parameters
    base: NormalizeBase,
    /// Normalisation integrator state
    normalisation_integrator: f32,
    /// Normalisation peak state
    normalisation_peak: f32,
}

/// Stereo channel normalizer with optimized two-channel processing
#[derive(Clone, Debug)]
pub struct NormalizeStereo<I> {
    /// Input audio source
    input: I,
    /// Common normalizer parameters
    base: NormalizeBase,
    /// Normalisation integrator states
    normalisation_integrators: [f32; 2],
    /// Normalisation peak states
    normalisation_peaks: [f32; 2],
    /// Current channel position
    position: u8,
}

/// Generic multi-channel normalizer for surround sound or other configurations
#[derive(Clone, Debug)]
pub struct NormalizeMulti<I> {
    /// Input audio source
    input: I,
    /// Common normalizer parameters
    base: NormalizeBase,
    /// Normalisation integrator states
    normalisation_integrators: Vec<f32>,
    /// Normalisation peak states
    normalisation_peaks: Vec<f32>,
    /// Current channel position
    position: usize,
}

/// Computes the gain reduction amount in dB based on input level.
///
/// Optimized for the most common case where samples are below threshold and no limiting is needed
/// (returns `ZERO_DB` early).
///
/// # Arguments
///
/// * `sample_f32` - Input sample value (with initial gain applied)
/// * `threshold` - Level where limiting begins (dB)
/// * `knee_width` - Width of soft knee region (dB)
///
/// # Returns
///
/// Amount of gain reduction to apply in dB
#[inline]
fn process_sample<S: Sample>(sample: S, threshold: f32, knee_width: f32) -> f32 {
    // Add slight DC offset. Some samples are silence, which is -inf dB and gets the limiter stuck.
    // Adding a small positive offset prevents this.
    let sample_f32 = sample.to_f32() + f32::MIN_POSITIVE;
    let bias_db = util::ratio_to_db(sample_f32.abs()) - threshold;
    let knee_boundary_db = bias_db * 2.0;
    if knee_boundary_db < -knee_width {
        ZERO_DB
    } else if knee_boundary_db.abs() <= knee_width {
        (knee_boundary_db + knee_width).powi(2) / (8.0 * knee_width)
    } else {
        bias_db
    }
}

impl NormalizeBase {
    fn new(ratio: f32, threshold: f32, knee_width: f32, attack: f32, release: f32) -> Self {
        Self {
            ratio,
            threshold,
            knee_width,
            attack,
            release,
        }
    }

    /// Updates the channel's envelope detection state.
    ///
    /// For each channel, processes:
    /// 1. Initial gain and dB conversion
    /// 2. Soft-knee limiting calculation
    /// 3. Envelope detection with attack/release filtering
    /// 4. Peak level tracking
    ///
    /// Note: Only updates state, gain application is handled by the variant implementations to
    /// allow for coupled gain reduction across channels.
    #[inline]
    fn process_channel<S: Sample>(&self, sample: S, integrator: &mut f32, peak: &mut f32) {
        // step 0: apply gain stage
        let sample = sample.amplify(self.ratio);

        // step 1-4: half-wave rectification and conversion into dB, and gain computer with soft
        // knee and subtractor
        let limiter_db = process_sample(sample, self.threshold, self.knee_width);

        // step 5: smooth, decoupled peak detector
        *integrator = f32::max(
            limiter_db,
            self.release * *integrator + (1.0 - self.release) * limiter_db,
        );
        *peak = self.attack * *peak + (1.0 - self.attack) * *integrator;
    }
}

impl<I> NormalizeMono<I>
where
    I: Source,
    I::Item: Sample,
{
    /// Processes the next mono sample through the limiter.
    ///
    /// Single channel implementation with direct state updates.
    #[inline]
    fn process_next(&mut self, sample: I::Item) -> I::Item {
        self.base.process_channel(
            sample,
            &mut self.normalisation_integrator,
            &mut self.normalisation_peak,
        );

        // steps 6-8: conversion into level and multiplication into gain stage
        sample.amplify(util::db_to_ratio(-self.normalisation_peak))
    }
}

impl<I> NormalizeStereo<I>
where
    I: Source,
    I::Item: Sample,
{
    /// Processes the next stereo sample through the limiter.
    ///
    /// Uses efficient channel position tracking with XOR toggle and direct array access for state
    /// updates.
    #[inline]
    fn process_next(&mut self, sample: I::Item) -> I::Item {
        let channel = self.position as usize;
        self.position ^= 1;

        self.base.process_channel(
            sample,
            &mut self.normalisation_integrators[channel],
            &mut self.normalisation_peaks[channel],
        );

        // steps 6-8: conversion into level and multiplication into gain stage. Find maximum peak
        // across both channels to couple the gain and maintain stereo imaging.
        let max_peak = f32::max(self.normalisation_peaks[0], self.normalisation_peaks[1]);
        sample.amplify(util::db_to_ratio(-max_peak))
    }
}

impl<I> NormalizeMulti<I>
where
    I: Source,
    I::Item: Sample,
{
    /// Processes the next multi-channel sample through the limiter.
    ///
    /// Generic implementation supporting arbitrary channel counts with `Vec`-based state storage.
    #[inline]
    fn process_next(&mut self, sample: I::Item) -> I::Item {
        let channel = self.position;
        self.position = (self.position + 1) % self.normalisation_integrators.len();

        self.base.process_channel(
            sample,
            &mut self.normalisation_integrators[channel],
            &mut self.normalisation_peaks[channel],
        );

        // steps 6-8: conversion into level and multiplication into gain stage. Find maximum peak
        // across all channels to couple the gain and maintain multi-channel imaging.
        let max_peak = self
            .normalisation_peaks
            .iter()
            .fold(ZERO_DB, |max, &peak| f32::max(max, peak));
        sample.amplify(util::db_to_ratio(-max_peak))
    }
}

impl<I> Normalize<I>
where
    I: Source,
    I::Item: Sample,
{
    /// Returns a reference to the inner audio source.
    ///
    /// Routes through the enum variant to access the underlying source, preserving the specialized
    /// implementation structure while allowing source inspection.
    ///
    /// Useful for inspecting source properties without consuming the filter.
    #[inline]
    pub fn inner(&self) -> &I {
        match self {
            Normalize::Mono(mono) => &mono.input,
            Normalize::Stereo(stereo) => &stereo.input,
            Normalize::MultiChannel(multi) => &multi.input,
        }
    }

    /// Returns a mutable reference to the inner audio source.
    ///
    /// Routes through the enum variant to access the underlying source, maintaining the
    /// specialized implementation structure while allowing source modification.
    ///
    /// Essential for operations like seeking that need to modify the source.
    #[inline]
    pub fn inner_mut(&mut self) -> &mut I {
        match self {
            Normalize::Mono(mono) => &mut mono.input,
            Normalize::Stereo(stereo) => &mut stereo.input,
            Normalize::MultiChannel(multi) => &mut multi.input,
        }
    }

    /// Consumes the filter and returns the inner audio source.
    ///
    /// Dismantles the normalizer variant to extract the source, allowing the audio pipeline to
    /// continue without normalization overhead.
    ///
    /// Useful when normalization is no longer needed but source should continue.
    #[inline]
    pub fn into_inner(self) -> I {
        match self {
            Normalize::Mono(mono) => mono.input,
            Normalize::Stereo(stereo) => stereo.input,
            Normalize::MultiChannel(multi) => multi.input,
        }
    }
}

impl<I> Iterator for Normalize<I>
where
    I: Source,
    I::Item: Sample,
{
    type Item = I::Item;

    /// Provides the next processed sample.
    ///
    /// Routes processing to the appropriate channel-specific implementation:
    /// * Mono: Direct single-channel processing
    /// * Stereo: Optimized two-channel processing
    /// * `MultiChannel`: Generic multi-channel processing
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Normalize::Mono(mono) => {
                let sample = mono.input.next()?;
                Some(mono.process_next(sample))
            }
            Normalize::Stereo(stereo) => {
                let sample = stereo.input.next()?;
                Some(stereo.process_next(sample))
            }
            Normalize::MultiChannel(multi) => {
                let sample = multi.input.next()?;
                Some(multi.process_next(sample))
            }
        }
    }

    /// Provides size hints from the inner source.
    ///
    /// Delegates directly to the source to maintain accurate collection sizing.
    /// Used by collection operations for optimization.
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner().size_hint()
    }
}

impl<I> Source for Normalize<I>
where
    I: Source,
    I::Item: Sample,
{
    /// Returns the number of samples in the current audio frame.
    ///
    /// Delegates to inner source to maintain frame alignment.
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        self.inner().current_frame_len()
    }

    /// Returns the number of channels in the audio stream.
    ///
    /// Channel count determines which normalizer variant is used:
    /// * 1: Mono
    /// * 2: Stereo
    /// * >2: MultiChannel
    fn channels(&self) -> u16 {
        self.inner().channels()
    }

    /// Returns the audio sample rate in Hz.
    fn sample_rate(&self) -> u32 {
        self.inner().sample_rate()
    }

    /// Returns the total duration of the audio.
    ///
    /// Returns None for streams without known duration.
    fn total_duration(&self) -> Option<Duration> {
        self.inner().total_duration()
    }

    /// Attempts to seek to the specified position.
    ///
    /// Resets limiter state to prevent artifacts after seeking:
    /// * Mono: Direct reset of integrator and peak values
    /// * Stereo: Efficient array fill for both channels
    /// * `MultiChannel`: Resets all channel states via fill
    ///
    /// # Arguments
    ///
    /// * `target` - Position to seek to
    ///
    /// # Errors
    ///
    /// Returns error if the underlying source fails to seek
    fn try_seek(&mut self, target: Duration) -> Result<(), SeekError> {
        self.inner_mut().try_seek(target)?;

        match self {
            Normalize::Mono(mono) => {
                mono.normalisation_integrator = ZERO_DB;
                mono.normalisation_peak = ZERO_DB;
            }
            Normalize::Stereo(stereo) => {
                stereo.normalisation_integrators.fill(ZERO_DB);
                stereo.normalisation_peaks.fill(ZERO_DB);
            }
            Normalize::MultiChannel(multi) => {
                multi.normalisation_integrators.fill(ZERO_DB);
                multi.normalisation_peaks.fill(ZERO_DB);
            }
        }

        Ok(())
    }
}
