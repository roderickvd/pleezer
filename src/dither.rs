//! Audio dithering and noise shaping implementation.
//!
//! This module implements:
//! * Triangular PDF (TPDF) dithering for optimal noise characteristics
//! * Shibata noise shaping filters for psychoacoustic optimization
//! * Volume control with dither and noise shaping integration
//!
//! The noise shaping uses Shibata filter coefficients from SSRC, optimized for
//! different sample rates and aggressiveness levels. These push quantization
//! noise to less audible frequencies based on human hearing characteristics.

// This file contains Shibata noise shaping filter coefficients from SSRC
// (a fast and high quality sampling rate converter).
//
// The coefficients are written by Naoki Shibata (shibatch@users.sourceforge.net)
// and licensed under the GNU Lesser General Public License (LGPL) version 2.1.
// They are used in this project for audio dithering and noise shaping.
//
// Original homepage: <http://shibatch.sourceforge.net/>

use std::{sync::Arc, time::Duration};

use cpal::ChannelCount;
use rodio::{Source, source::SeekError};

use crate::{ringbuf::RingBuffer, util::UNITY_GAIN, volume::Volume};

/// Creates a new audio source with dithered volume control and optional noise shaping.
///
/// # Arguments
///
/// * `input` - The source audio stream
/// * `volume` - Volume control with optional dithering parameters
/// * `noise_shaping_profile` - Noise shaping aggression level:
///   - 0: Plain TPDF dither without shaping
///   - 1: Conservative noise shaping
///   - 2: Balanced noise shaping (recommended default)
///   - 3: Strong noise shaping
///   - 4-7: Very aggressive shaping (not recommended for playback)
///
/// # Implementation Details
///
/// * Uses TPDF dither to convert truncation to rounding
/// * Applies DC offset compensation
/// * For noise shaping profiles 1-7, uses Shibata filters optimized for 44.1/48kHz
/// * Manages headroom to prevent clipping
/// * Maintains error history for noise shaping feedback
///
/// The actual filter used depends on both the sample rate and chosen profile.
#[expect(clippy::too_many_lines)]
pub fn dithered_volume<I>(
    input: I,
    volume: Arc<Volume>,
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

    if noise_shaping_profile == 0 {
        debug!("noise shaping profile: disabled");
    } else {
        debug!("noise shaping profile: {}", noise_shaping_profile.min(7));
    }

    match (input.sample_rate(), noise_shaping_profile) {
        (44100, 1) => Box::new(DitheredVolume::<I, 12> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_0,
        }),
        (44100, 2) => Box::new(DitheredVolume::<I, 12> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_1,
        }),
        (44100, 3) => Box::new(DitheredVolume::<I, 24> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_2,
        }),
        (44100, 4) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_3,
        }),
        (44100, 5) => Box::new(DitheredVolume::<I, 20> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_4,
        }),
        (44100, 6) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_5,
        }),
        (44100, 7) => Box::new(DitheredVolume::<I, 20> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_441_ATH_A_6,
        }),
        (48000, 1) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_0,
        }),
        (48000, 2) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_1,
        }),
        (48000, 3) => Box::new(DitheredVolume::<I, 16> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_2,
        }),
        (48000, 4) => Box::new(DitheredVolume::<I, 19> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_3,
        }),
        (48000, 5) => Box::new(DitheredVolume::<I, 28> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_4,
        }),
        (48000, 6) => Box::new(DitheredVolume::<I, 20> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_5,
        }),
        (48000, 7) => Box::new(DitheredVolume::<I, 28> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &SHIBATA_48_ATH_A_6,
        }),
        _ => Box::new(DitheredVolume::<I, 0> {
            input,
            volume,
            rng: fastrand::Rng::new(),
            quantization_error_history: RingBuffer::new(),
            filter_coefficients: &[],
        }),
    }
}

/// Audio source with integrated dithering, noise shaping and volume control.
///
/// Processes audio samples with:
/// * Volume scaling
/// * TPDF dither when reducing bit depth
/// * Noise shaping using Shibata filters (when enabled)
/// * DC offset compensation
///
/// Type parameter N determines the noise shaping filter length,
/// varies by sample rate and chosen profile.
#[derive(Debug, Clone)]
pub struct DitheredVolume<I, const N: usize> {
    /// The underlying audio source
    input: I,

    /// Volume control with dithering parameters
    volume: Arc<Volume>,

    /// Fast random number generator for TPDF dither
    rng: fastrand::Rng,

    /// Ring buffer storing previous quantization errors for noise shaping
    quantization_error_history: RingBuffer<N>,

    /// Shibata filter coefficients for the current sample rate and profile
    filter_coefficients: &'static [f32; N],
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
        const DC_COMPENSATION: f32 = 0.5;

        self.input.next().map(|sample| {
            let mut volume = self.volume.volume();

            if let Some(quantization_step) = self.volume.quantization_step() {
                // Apply volume attenuation, preventing clipping at full scale
                volume = volume.min(UNITY_GAIN - quantization_step);

                // Calculate TPDF dither and DC compensation to convert truncation to rounding
                let dither = (self.rng.f32() - self.rng.f32()) * quantization_step;

                let dithered = if N > 0 {
                    // Noise shaping: apply filtered error feedback from previous samples to
                    // pre-compensate for quantization
                    let mut filtered_error = 0.0;
                    for i in 0..N {
                        filtered_error +=
                            self.filter_coefficients[i] * self.quantization_error_history.get(i);
                    }
                    let shaped_signal = sample + filtered_error + dither;

                    // Quantize signal as if it were to its output sample format
                    let quantized = (shaped_signal / quantization_step + DC_COMPENSATION).trunc()
                        * quantization_step;

                    // Calculate and store new error
                    let error = quantized - shaped_signal;
                    self.quantization_error_history.push(error);
                    quantized
                } else {
                    // No noise shaping: only apply dither
                    sample + dither
                };
                (dithered + DC_COMPENSATION * quantization_step) * volume
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
        let result = self.input.try_seek(pos);
        if result.is_ok() {
            self.quantization_error_history.reset();
        }
        result
    }
}

/// Module containing Shibata noise shaping filter coefficients.
///
/// These coefficients are from SSRC (Sample rate converter) by Naoki Shibata,
/// licensed under LGPL-2.1. They are designed for optimal perceptual noise shaping
/// based on human hearing characteristics.
mod coeffs {
    /// Conservative noise shaping filter for 44.1kHz (12 coefficients)
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

    /// Balanced noise shaping filter for 44.1kHz (12 coefficients)
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

    /// Strong noise shaping filter for 44.1kHz (24 coefficients)
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

    /// Aggressive noise shaping filter for 44.1kHz (16 coefficients)
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

    /// Very aggressive noise shaping filter for 44.1kHz (20 coefficients)
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

    /// Extremely aggressive noise shaping filter for 44.1kHz (16 coefficients)
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

    /// Maximum aggression noise shaping filter for 44.1kHz (20 coefficients)
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

    /// Conservative noise shaping filter for 48kHz (16 coefficients)
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

    /// Balanced noise shaping filter for 48kHz (16 coefficients)
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

    /// Strong noise shaping filter for 48kHz (16 coefficients)
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

    /// Aggressive noise shaping filter for 48kHz (19 coefficients)
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

    /// Very aggressive noise shaping filter for 48kHz (28 coefficients)
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

    /// Extremely aggressive noise shaping filter for 48kHz (20 coefficients)
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

    /// Maximum aggression noise shaping filter for 48kHz (28 coefficients)
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
}
