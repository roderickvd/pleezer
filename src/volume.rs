use std::sync::atomic::{AtomicU32, Ordering};

use crate::{
    track::DEFAULT_BITS_PER_SAMPLE,
    util::{ToF32, UNITY_GAIN},
};

#[derive(Debug)]
pub struct Volume {
    volume: AtomicU32,
    dither: Option<Dither>,
}

#[derive(Debug)]
struct Dither {
    dac_bits: f32,
    track_bits: AtomicU32,
    scale: AtomicU32,
}

impl Default for Volume {
    fn default() -> Self {
        Self {
            volume: AtomicU32::new(Self::DEFAULT_VOLUME.to_bits()),
            dither: None,
        }
    }
}

impl Volume {
    /// Default volume level.
    ///
    /// Constant value of 100% (1.0) used as initial volume setting.
    pub const DEFAULT_VOLUME: f32 = UNITY_GAIN;

    #[must_use]
    pub fn new(volume: f32, dac_bits: Option<f32>) -> Self {
        let track_bits = DEFAULT_BITS_PER_SAMPLE;
        Self {
            volume: AtomicU32::new(volume.to_bits()),
            dither: dac_bits.map(|dac_bits| Dither {
                dac_bits,
                track_bits: AtomicU32::new(track_bits),
                scale: AtomicU32::new(calculate_scale(dac_bits, track_bits, volume).to_bits()),
            }),
        }
    }

    #[must_use]
    pub fn dither_scale(&self) -> Option<f32> {
        self.dither
            .as_ref()
            .map(|dither| f32::from_bits(dither.scale.load(Ordering::Relaxed)))
    }

    #[must_use]
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, volume: f32) -> f32 {
        if let Some(dither) = self.dither.as_ref() {
            let scale = calculate_scale(dither.dac_bits, self.track_bits(), volume);
            dither.scale.store(scale.to_bits(), Ordering::Relaxed);
        }

        // set volume last: in case of low volume before, dithering would be at a fairly
        // low significant bits, which could lead to audible artifacts if the volume were
        // raised before (race condition)
        let previous = self.volume.swap(volume.to_bits(), Ordering::Relaxed);
        f32::from_bits(previous)
    }

    #[must_use]
    pub fn track_bits(&self) -> u32 {
        self.dither
            .as_ref()
            .map_or(DEFAULT_BITS_PER_SAMPLE, |dither| {
                dither.track_bits.load(Ordering::Relaxed)
            })
    }

    pub fn set_track_bits(&self, track_bits: Option<u32>) {
        if let Some(dither) = self.dither.as_ref() {
            let track_bits = track_bits.unwrap_or(DEFAULT_BITS_PER_SAMPLE);
            let scale = calculate_scale(dither.dac_bits, track_bits, self.volume());
            dither.track_bits.store(track_bits, Ordering::Relaxed);
            dither.scale.store(scale.to_bits(), Ordering::Relaxed);
        }
    }

    #[must_use]
    pub fn dither_bits(&self) -> Option<f32> {
        self.dither
            .as_ref()
            .map(|dither| calculate_dither_bits(dither.dac_bits, self.track_bits(), self.volume()))
    }
}

#[must_use]
fn calculate_dither_bits(dac_bits: f32, track_bits: u32, volume: f32) -> f32 {
    // Scale to the magnitude of the volume, but not exceeding the track bits
    // and preventing -infinity
    f32::min(track_bits.to_f32_lossy(), dac_bits + volume.log2()).max(0.0)
}

#[must_use]
fn calculate_scale(dac_bits: f32, track_bits: u32, volume: f32) -> f32 {
    // 2 LSB of dither, scaling a number of unsigned bits to -1.0..1.0
    1.0 / f32::powf(2.0, calculate_dither_bits(dac_bits, track_bits, volume))
}
