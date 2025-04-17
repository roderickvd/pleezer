use std::sync::atomic::{AtomicU32, Ordering};

use crate::{protocol::connect::Percentage, track::DEFAULT_BITS_PER_SAMPLE, util::ToF32};

#[derive(Debug)]
pub struct Volume {
    volume: AtomicU32,
    dither: Option<Dither>,
}

#[derive(Debug)]
struct Dither {
    dither_bits: usize,
    scale: AtomicU32,
}

impl Volume {
    #[must_use]
    pub fn new(volume: Percentage, dither_bits: Option<usize>) -> Self {
        let volume = volume.as_ratio();
        Self {
            volume: AtomicU32::new(volume.to_bits()),
            dither: dither_bits.map(|dither_bits| Dither {
                dither_bits,
                scale: AtomicU32::new(calculate_scale(dither_bits, volume).to_bits()),
            }),
        }
    }

    pub fn get(&self) -> (f32, Option<f32>) {
        (
            f32::from_bits(self.volume.load(Ordering::Relaxed)),
            self.dither
                .as_ref()
                .map(|dither| f32::from_bits(dither.scale.load(Ordering::Relaxed))),
        )
    }

    pub fn set(&self, volume: f32) {
        self.volume.store(volume.to_bits(), Ordering::Relaxed);
        if let Some(dither) = self.dither.as_ref() {
            let scale = calculate_scale(dither.dither_bits, volume);
            dither.scale.store(scale.to_bits(), Ordering::Relaxed);
        }
    }
}

#[must_use]
fn calculate_scale(dither_bits: usize, volume: f32) -> f32 {
    // Scale to the magnitude of the volume
    let bits_of_interest = f32::min(
        DEFAULT_BITS_PER_SAMPLE.to_f32_lossy(),
        dither_bits.to_f32_lossy() + volume.log2(),
    );

    // 2 LSB of dither, scaling a number of unsigned bits to -1.0..1.0
    1.0 / f32::powf(2.0, bits_of_interest)
}
