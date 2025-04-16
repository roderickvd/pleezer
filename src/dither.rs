use std::{sync::Arc, time::Duration};

use cpal::ChannelCount;
use rodio::{Source, source::SeekError};

use crate::volume::Volume;

pub fn dithered_volume<I>(input: I, volume: Arc<Volume>) -> DitheredVolume<I> {
    DitheredVolume {
        input,
        volume,
        rng: fastrand::Rng::new(),
        noise: 0.0,
    }
}

#[derive(Debug)]
pub struct DitheredVolume<I> {
    input: I,
    volume: Arc<Volume>,
    // Initialize a dedicated random number generator for more efficiency
    rng: fastrand::Rng,
    noise: f32,
}

impl<I> DitheredVolume<I>
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

impl<I> Iterator for DitheredVolume<I>
where
    I: Source,
{
    type Item = I::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.input.next().map(|sample| {
            let (volume, scale) = self.volume.get();
            let dither = if let Some(scale) = scale {
                // Scale the noise to the range -1.0..1.0
                let new_noise = self.rng.f32() * 2.0 - 1.0;
                // Generate a high-passed TPDF dither by reusing the noise from the last sample
                let tpdf = new_noise - self.noise;
                self.noise = new_noise;
                tpdf * scale
            } else {
                0.0
            };

            (sample + dither) * volume
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.input.size_hint()
    }
}

impl<I> Source for DitheredVolume<I>
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
        self.input.try_seek(pos)
    }
}
