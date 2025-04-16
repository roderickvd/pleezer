use std::sync::atomic::{AtomicU32, Ordering};

use crate::{protocol::connect::Percentage, util::ToF32};

#[derive(Debug)]
pub struct Volume {
    volume: AtomicU32,
    dither: Option<Dither>,
}

#[derive(Debug)]
struct Dither {
    bits: usize,
    scale: AtomicU32,
}

#[expect(clippy::cast_possible_truncation)]
fn calculate_scale(bits: usize, volume: f32) -> f32 {
    // Scale to the magnitude of the volume
    let bits = bits.saturating_add_signed(volume.log2().floor() as isize);

    // 2 LSB of dither, scaling a number of unsigned bits to -1.0..1.0
    1.0 / (1_usize << bits.saturating_sub(1)).to_f32_lossy()
}

impl Volume {
    #[must_use]
    pub fn new(volume: Percentage, bits: Option<usize>) -> Self {
        let volume = volume.as_ratio();
        Self {
            volume: AtomicU32::new(volume.to_bits()),
            dither: bits.map(|bits| Dither {
                bits,
                scale: AtomicU32::new(calculate_scale(bits, volume).to_bits()),
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
            let scale = calculate_scale(dither.bits, volume);
            dither.scale.store(scale.to_bits(), Ordering::Relaxed);
        }
    }
}
