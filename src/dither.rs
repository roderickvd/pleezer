use std::{sync::Arc, time::Duration};

use cpal::ChannelCount;
use rodio::{Source, source::SeekError};

use crate::{
    shape::{
        RingBuffer,
        coeffs::{
            SHIBATA_48_ATH_A_0, SHIBATA_48_ATH_A_1, SHIBATA_48_ATH_A_2, SHIBATA_48_ATH_A_3,
            SHIBATA_48_ATH_A_4, SHIBATA_48_ATH_A_5, SHIBATA_48_ATH_A_6, SHIBATA_441_ATH_A_0,
            SHIBATA_441_ATH_A_1, SHIBATA_441_ATH_A_2, SHIBATA_441_ATH_A_3, SHIBATA_441_ATH_A_4,
            SHIBATA_441_ATH_A_5, SHIBATA_441_ATH_A_6, TPDF_HIGH_PASS,
        },
    },
    util::UNITY_GAIN,
    volume::Volume,
};

#[expect(clippy::too_many_lines)]
pub fn dithered_volume<I>(
    input: I,
    volume: Arc<Volume>,
    noise_shaping: u8,
) -> Box<dyn Source<Item = I::Item> + Send>
where
    I: Source + Send + 'static,
{
    match (input.sample_rate(), noise_shaping) {
        (44100, 0) => Box::new(DitheredVolume::<I, 12> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_441_ATH_A_0,
        }),
        (44100, 1) => Box::new(DitheredVolume::<I, 12> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_441_ATH_A_1,
        }),
        (44100, 2) => Box::new(DitheredVolume::<I, 24> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_441_ATH_A_2,
        }),
        (44100, 3) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_441_ATH_A_3,
        }),
        (44100, 4) => Box::new(DitheredVolume::<I, 20> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_441_ATH_A_4,
        }),
        (44100, 5) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_441_ATH_A_5,
        }),
        (44100, 6) => Box::new(DitheredVolume::<I, 20> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_441_ATH_A_6,
        }),
        (48000, 0) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_48_ATH_A_0,
        }),
        (48000, 1) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_48_ATH_A_1,
        }),
        (48000, 2) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_48_ATH_A_2,
        }),
        (48000, 3) => Box::new(DitheredVolume::<I, 19> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_48_ATH_A_3,
        }),
        (48000, 4) => Box::new(DitheredVolume::<I, 28> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_48_ATH_A_4,
        }),
        (48000, 5) => Box::new(DitheredVolume::<I, 20> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_48_ATH_A_5,
        }),
        (48000, 6) => Box::new(DitheredVolume::<I, 28> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &SHIBATA_48_ATH_A_6,
        }),
        _ => Box::new(DitheredVolume::<I, 1> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            error_history: RingBuffer::new(),
            coeffs: &TPDF_HIGH_PASS,
        }),
    }
}

#[derive(Debug, Clone)]
pub struct DitheredVolume<I, const N: usize> {
    input: I,
    volume: Arc<Volume>,
    // Initialize a dedicated random number generator for more efficiency
    rng: fastrand::Rng,
    error_history: RingBuffer<N>,
    coeffs: &'static [f32; N],
}

impl<I, const N: usize> DitheredVolume<I, N>
where
    I: Source,
{
    /// Returns a reference to the inner source.
    #[inline]
    pub fn inner(&self) -> &I {
        &self.input
    }

    /// Returns a mutable reference to the inner source.
    #[inline]
    pub fn inner_mut(&mut self) -> &mut I {
        &mut self.input
    }

    /// Returns the inner source.
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
        self.input.next().map(|sample| {
            let mut volume = self.volume.volume();

            if let Some(scale) = self.volume.dither_scale() {
                // Prevent clipping at full scale
                volume = volume.min(UNITY_GAIN - scale);

                // TPDF in -1..1 (2 LSB) to the target bit depth
                let dither = (self.rng.f32() - self.rng.f32()) * scale;
                let output = (sample + dither) * volume;

                // Noise shaping
                let error = output - sample * volume;
                let mut shaped_error = 0.0;
                for i in 0..N {
                    shaped_error += self.coeffs[i] * self.error_history.get(i);
                }
                self.error_history.push(error);
                output + shaped_error
            } else {
                sample * volume
            }
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
    #[inline]
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    #[inline]
    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        self.input.sample_rate()
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }

    #[inline]
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.error_history.reset();
        self.input.try_seek(pos)
    }
}
