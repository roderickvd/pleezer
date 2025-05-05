//! Volume control and dithering management.
//!
//! This module provides volume control with integrated dithering support for
//! high-quality audio processing. It handles:
//!
//! * Volume adjustment with atomic operations
//! * Bit depth management and dithering
//! * Dynamic quantization step calculation
//! * Effective bit depth tracking
//!
//! # Volume Control
//!
//! Volume is managed using thread-safe atomic operations:
//! * Linear scale from 0.0 to 1.0 (0% to 100%)
//! * Default volume is 1.0 (100%)
//! * Changes are immediately reflected across all threads
//!
//! # Dithering
//!
//! When configured with DAC bit depth information, provides:
//! * Automatic dithering for bit depth reduction
//! * Dynamic quantization step calculation
//! * Volume-aware dither scaling
//! * Source/destination bit depth tracking
//!
//! # Example
//!
//! ```rust
//! use pleezer::volume::Volume;
//!
//! // Create volume control with 20-bit DAC
//! let volume = Volume::new(1.0, Some(20.0));
//!
//! // Set volume to 50%
//! volume.set_volume(0.5);
//!
//! // Configure track bit depth (e.g., 16-bit source)
//! volume.set_track_bit_depth(Some(16));
//!
//! // Get effective bit depth after volume scaling
//! if let Some(bits) = volume.effective_bit_depth() {
//!     println!("Effective bit depth: {}", bits);
//! }
//! ```

use std::sync::atomic::{AtomicU32, Ordering};

use crate::{
    dither::DC_COMPENSATION,
    track::DEFAULT_BITS_PER_SAMPLE,
    util::{ToF32, UNITY_GAIN},
};

/// Volume control with integrated dithering support.
///
/// Provides thread-safe volume control and optional dithering:
/// * Atomic volume adjustments
/// * Source/destination bit depth management
/// * Dynamic quantization step calculation
/// * Volume-aware dither scaling
#[derive(Debug)]
pub struct Volume {
    /// Current volume level stored as bits of an f32.
    /// Uses atomic storage for thread-safe access.
    volume: AtomicU32,

    /// Optional dithering configuration.
    /// None if dithering is disabled (no DAC bit depth provided).
    dither: Option<Dither>,
}

/// Dithering configuration and state.
///
/// Manages the parameters needed for dithering:
/// * DAC (output device) bit depth
/// * Source material bit depth
/// * Current quantization step size
#[derive(Debug)]
struct Dither {
    /// Bit depth of the DAC (output device).
    /// Fixed value determined at initialization.
    dac_bit_depth: f32,

    /// Current track/source material bit depth.
    /// Stored atomically for thread-safe updates.
    track_bit_depth: AtomicU32,

    /// Current quantization step size for dithering.
    /// Stored as bits of an f32 for atomic updates.
    quantization_step: AtomicU32,
}

impl Default for Volume {
    /// Creates a new Volume instance with default settings:
    /// * Volume set to 100% (1.0)
    /// * Dithering disabled
    fn default() -> Self {
        Self {
            volume: AtomicU32::new(DEFAULT_VOLUME.to_bits()),
            dither: None,
        }
    }
}

/// Default volume level.
///
/// Constant value of 100% (1.0) used as initial volume setting.
pub const DEFAULT_VOLUME: f32 = UNITY_GAIN;

impl Volume {
    /// Creates a new volume control with optional dithering support.
    ///
    /// # Arguments
    ///
    /// * `volume` - Initial volume level (0.0 to 1.0)
    /// * `dac_bits` - DAC bit depth for dithering configuration. If None, dithering is disabled.
    ///
    /// # Example
    ///
    /// ```rust
    /// // Create volume control with 24-bit DAC
    /// let volume = Volume::new(1.0, Some(24.0));
    /// ```
    #[must_use]
    pub fn new(volume: f32, dac_bits: Option<f32>) -> Self {
        let track_bits = DEFAULT_BITS_PER_SAMPLE;
        Self {
            volume: AtomicU32::new(volume.to_bits()),
            dither: dac_bits.map(|dac_bits| Dither {
                dac_bit_depth: dac_bits,
                track_bit_depth: AtomicU32::new(track_bits),
                quantization_step: AtomicU32::new(
                    calculate_quantization_step(dac_bits, track_bits, volume).to_bits(),
                ),
            }),
        }
    }

    /// Returns the current quantization step size used for dithering.
    ///
    /// The quantization step determines the magnitude of dither noise added
    /// when reducing bit depth. This value is automatically adjusted based on:
    /// * Current volume level
    /// * DAC bit depth
    /// * Source material bit depth
    ///
    /// Returns `None` if dithering is disabled.
    #[must_use]
    pub fn quantization_step(&self) -> Option<f32> {
        self.dither
            .as_ref()
            .map(|dither| f32::from_bits(dither.quantization_step.load(Ordering::Relaxed)))
    }

    /// Returns the current volume level (0.0 to 1.0).
    ///
    /// Uses relaxed atomic ordering as volume changes don't need
    /// to be strictly synchronized.
    #[must_use]
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    /// Sets a new volume level and updates dithering parameters.
    ///
    /// # Arguments
    ///
    /// * `volume` - New volume level (0.0 to 1.0)
    ///
    /// # Returns
    ///
    /// Previous volume level
    ///
    /// # Thread Safety
    ///
    /// Uses atomic operations to ensure thread-safe updates of:
    /// * Volume level
    /// * Quantization step size
    /// * Dithering parameters
    pub fn set_volume(&self, volume: f32) -> f32 {
        let mut new = volume;
        if let Some(dither) = self.dither.as_ref() {
            let quantization_step =
                calculate_quantization_step(dither.dac_bit_depth, self.track_bit_depth(), volume);
            dither
                .quantization_step
                .store(quantization_step.to_bits(), Ordering::Relaxed);

            // Preventing clipping at full scale
            new = new.min(UNITY_GAIN - (1.0 + DC_COMPENSATION) * quantization_step);
        }

        // set volume last: in case of low volume before, dithering would be at a fairly
        // low significant bits, which could lead to audible artifacts if the volume were
        // raised before (race condition)
        let previous = self.volume.swap(new.to_bits(), Ordering::Relaxed);
        f32::from_bits(previous)
    }

    /// Returns the current track bit depth setting.
    ///
    /// This represents the bit depth of the source audio material.
    /// If no dithering is configured, returns the default bit depth.
    #[must_use]
    pub fn track_bit_depth(&self) -> u32 {
        self.dither
            .as_ref()
            .map_or(DEFAULT_BITS_PER_SAMPLE, |dither| {
                dither.track_bit_depth.load(Ordering::Relaxed)
            })
    }

    /// Sets the track bit depth and updates the quantization parameters.
    ///
    /// # Arguments
    ///
    /// * `track_bits` - The bit depth of the source material. If `None`, uses the default bit
    ///   depth.
    ///
    /// This updates both the track bit depth and recalculates the quantization step
    /// based on the DAC bit depth, track bit depth, and current volume settings.
    /// Has no effect if dithering is disabled.
    pub fn set_track_bit_depth(&self, track_bits: Option<u32>) {
        if let Some(dither) = self.dither.as_ref() {
            let track_bits = track_bits.unwrap_or(DEFAULT_BITS_PER_SAMPLE);
            let quant_level =
                calculate_quantization_step(dither.dac_bit_depth, track_bits, self.volume());
            dither.track_bit_depth.store(track_bits, Ordering::Relaxed);
            dither
                .quantization_step
                .store(quant_level.to_bits(), Ordering::Relaxed);
        }
    }

    /// Calculates and returns the effective bit depth of the audio output.
    ///
    /// The effective bit depth takes into account:
    /// * The DAC's bit depth
    /// * The source material's bit depth
    /// * The current volume setting
    ///
    /// Returns `None` if dithering is disabled.
    ///
    /// This is useful for understanding the actual resolution of the audio output
    /// after volume adjustments and dithering are applied.
    #[must_use]
    pub fn effective_bit_depth(&self) -> Option<f32> {
        self.dither.as_ref().map(|dither| {
            calculate_effective_bit_depth(
                dither.dac_bit_depth,
                self.track_bit_depth(),
                self.volume(),
            )
        })
    }
}

/// Calculates the effective quantization resolution based on system parameters.
///
/// # Arguments
///
/// * `dac_bits` - The bit depth of the DAC
/// * `track_bits` - The bit depth of the source material
/// * `volume` - The current volume setting (in linear scale)
///
/// # Returns
///
/// The effective resolution for quantization calculation, taking into account:
/// * DAC capabilities
/// * Source material bit depth
/// * Volume attenuation
/// * Minimum 6-bit resolution for clean fade-outs
///
/// The minimum 6-bit (36dB) depth ensures:
/// * Smooth volume transitions during fades
/// * Steps below human Just Noticeable Difference (~0.5-1dB)
/// * Proper dither behavior at low volumes
/// * Prevention of quantization artifacts
#[must_use]
fn calculate_effective_bit_depth(dac_bits: f32, track_bits: u32, volume: f32) -> f32 {
    // Scale to the magnitude of the volume, but not exceeding the track bits
    // and preventing -infinity
    f32::min(track_bits.to_f32_lossy(), dac_bits + volume.log2()).max(6.0)
}

/// Calculates the quantization step size for dithering.
///
/// # Arguments
///
/// * `dac_bits` - The bit depth of the DAC
/// * `track_bits` - The bit depth of the source material
/// * `volume` - The current volume setting (in linear scale)
///
/// # Returns
///
/// The step size for optimal dither noise based on the effective quantization resolution.
#[must_use]
fn calculate_quantization_step(dac_bits: f32, track_bits: u32, volume: f32) -> f32 {
    1.0 / f32::powf(
        2.0,
        calculate_effective_bit_depth(dac_bits, track_bits, volume) - 1.0,
    )
}
