//! Real-time spectrum visualizer.
//!
//! Reads raw PCM from an MPD `fifo` audio output and sends logarithmic bar
//! magnitudes to the UI. The reader runs on a blocking thread because FIFOs
//! block until a writer appears.
//!
//! MPD's fifo output must be configured separately, for example:
//!
//! ```text
//! audio_output {
//!     type "fifo"
//!     name "neo-ncmp visualizer"
//!     path "/tmp/mpd.fifo"
//!     format "44100:16:2"
//! }
//! ```

use std::fs::OpenOptions;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use realfft::{RealFftPlanner, RealToComplex};
use tokio::sync::mpsc::UnboundedSender;

use crate::mpd_worker::Msg;

const FFT_SIZE: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSpec {
    pub sample_rate: u32,
    pub channels: u8,
    pub bits: u8,
}

pub fn spawn(fifo: PathBuf, spec: AudioSpec, bars: usize, tx: UnboundedSender<Msg>) {
    if spec.bits != 16 || spec.channels == 0 || spec.sample_rate == 0 {
        let _ = tx.send(Msg::VisualizerError(Some(
            "the visualizer currently supports 16-bit PCM with at least one channel".into(),
        )));
        return;
    }
    tokio::task::spawn_blocking(move || reader_loop(fifo, spec, bars.max(1), tx));
}

fn reader_loop(fifo: PathBuf, spec: AudioSpec, bars: usize, tx: UnboundedSender<Msg>) {
    let frame_bytes = 2 * spec.channels as usize;
    let window_bytes = FFT_SIZE * frame_bytes;
    let mut analyzer = SpectrumAnalyzer::new(spec.sample_rate, FFT_SIZE, bars);
    let mut buffer = vec![0u8; 16 * 1024];
    let mut pending: Vec<u8> = Vec::with_capacity(window_bytes * 2);
    let mut first_frame = true;

    loop {
        let _ = tx.send(Msg::VisualizerError(Some(format!(
            "waiting for audio at {}",
            fifo.display()
        ))));
        let mut file = match OpenOptions::new().read(true).open(&fifo) {
            Ok(file) => file,
            Err(error) => {
                let _ = tx.send(Msg::VisualizerError(Some(format!(
                    "could not open {}: {error}",
                    fifo.display()
                ))));
                if tx.is_closed() {
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let _ = tx.send(Msg::VisualizerError(None));
        pending.clear();

        loop {
            match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    pending.extend_from_slice(&buffer[..read]);
                    while pending.len() >= window_bytes {
                        let chunk: Vec<u8> = pending.drain(..window_bytes).collect();
                        let samples = decode_mono(&chunk, FFT_SIZE, spec.channels);
                        let values = analyzer.analyze(&samples);
                        if first_frame {
                            first_frame = false;
                            tracing::info!(
                                max = values.iter().copied().fold(0.0f32, f32::max),
                                "visualizer received its first audio frame"
                            );
                        }
                        if tx.send(Msg::Visualizer(values)).is_err() {
                            return;
                        }
                    }
                }
                Err(error) => {
                    let _ = tx.send(Msg::VisualizerError(Some(format!(
                        "audio read error: {error}"
                    ))));
                    break;
                }
            }
        }

        if tx.is_closed() {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Decodes interleaved little-endian 16-bit PCM to mono `f32` samples.
pub fn decode_mono(chunk: &[u8], frames: usize, channels: u8) -> Vec<f32> {
    let frame_bytes = 2 * channels as usize;
    let mut samples = vec![0.0f32; frames];
    for (index, frame) in chunk.chunks_exact(frame_bytes).enumerate().take(frames) {
        let mut sum = 0.0;
        for channel in 0..channels as usize {
            let offset = channel * 2;
            sum += i16::from_le_bytes([frame[offset], frame[offset + 1]]) as f32 / 32_768.0;
        }
        samples[index] = sum / channels as f32;
    }
    samples
}

/// FFT analyser with windowing, logarithmic bands, and peak-hold smoothing.
pub struct SpectrumAnalyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    input: Vec<f32>,
    output: Vec<realfft::num_complex::Complex<f32>>,
    scratch: Vec<realfft::num_complex::Complex<f32>>,
    window: Vec<f32>,
    bands: Vec<std::ops::Range<usize>>,
    smoothed: Vec<f32>,
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: u32, fft_size: usize, bars: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);
        let input = fft.make_input_vec();
        let output = fft.make_output_vec();
        let scratch = fft.make_scratch_vec();
        let window = (0..fft_size)
            .map(|index| {
                let phase = 2.0 * std::f32::consts::PI * index as f32 / (fft_size - 1) as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect();
        let bands = log_bands(sample_rate, fft_size, bars);
        let smoothed = vec![0.0; bands.len()];
        Self {
            fft,
            input,
            output,
            scratch,
            window,
            bands,
            smoothed,
        }
    }

    pub fn analyze(&mut self, samples: &[f32]) -> Vec<f32> {
        for (index, value) in self.input.iter_mut().enumerate() {
            *value = samples.get(index).copied().unwrap_or(0.0) * self.window[index];
        }
        if self
            .fft
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
            .is_err()
        {
            return self.smoothed.clone();
        }

        let scale = self.input.len() as f32;
        for (smooth, band) in self.smoothed.iter_mut().zip(self.bands.iter()) {
            let magnitude = self.output[band.clone()]
                .iter()
                .map(|value| (value.re * value.re + value.im * value.im).sqrt())
                .fold(0.0f32, f32::max);
            let decibels = 20.0 * (magnitude / scale).max(1e-9).log10();
            let value = ((decibels + 60.0) / 60.0).clamp(0.0, 1.0);
            // Peak-hold with exponential falloff; this looks much better than
            // raw magnitudes at a 20 fps update rate.
            *smooth = (*smooth * 0.72).max(value);
        }
        self.smoothed.clone()
    }
}

/// Logarithmically spaced FFT bin ranges from roughly 30 Hz to 18 kHz.
pub fn log_bands(sample_rate: u32, fft_size: usize, bars: usize) -> Vec<std::ops::Range<usize>> {
    let bars = bars.max(1);
    let bins = (fft_size / 2).max(2);
    let sample_rate = sample_rate.max(1) as f32;
    let low = 30.0f32;
    let high = (sample_rate / 2.0).min(18_000.0).max(low * 2.0);
    let ratio = high / low;
    (0..bars)
        .map(|index| {
            let start_hz = low * ratio.powf(index as f32 / bars as f32);
            let end_hz = low * ratio.powf((index + 1) as f32 / bars as f32);
            let start = ((start_hz * fft_size as f32 / sample_rate).floor() as usize)
                .max(1)
                .min(bins - 1);
            let end = ((end_hz * fft_size as f32 / sample_rate).ceil() as usize)
                .max(start + 1)
                .min(bins);
            start..end
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_are_monotonic_and_non_empty() {
        let bands = log_bands(44_100, 2048, 64);
        assert_eq!(bands.len(), 64);
        for window in &bands {
            assert!(window.start < window.end);
            assert!(window.end <= 1024);
        }
        for pair in bands.windows(2) {
            assert!(pair[0].end <= pair[1].start + 1);
        }
    }

    #[test]
    fn sine_wave_peaks_in_the_expected_band() {
        let sample_rate = 44_100;
        let frequency = 1_000.0f32;
        let samples: Vec<f32> = (0..FFT_SIZE)
            .map(|index| {
                (2.0 * std::f32::consts::PI * frequency * index as f32 / sample_rate as f32).sin()
            })
            .collect();

        let bars = 64;
        let mut analyzer = SpectrumAnalyzer::new(sample_rate, FFT_SIZE, bars);
        let values = analyzer.analyze(&samples);
        let peak = values
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.partial_cmp(right.1).unwrap())
            .unwrap()
            .0;

        let expected_bin = (frequency * FFT_SIZE as f32 / sample_rate as f32).round() as usize;
        let expected_band = log_bands(sample_rate, FFT_SIZE, bars)
            .iter()
            .position(|window| window.contains(&expected_bin))
            .unwrap();
        assert!(
            (peak as isize - expected_band as isize).abs() <= 2,
            "peak bar {peak}, expected around {expected_band}"
        );
    }

    #[test]
    fn decodes_interleaved_pcm_to_mono() {
        let frames = 4;
        let mut pcm = Vec::new();
        for value in [0i16, 1000, -1000, 32767] {
            pcm.extend_from_slice(&value.to_le_bytes());
            pcm.extend_from_slice(&value.to_le_bytes());
        }
        let samples = decode_mono(&pcm, frames, 2);
        assert_eq!(samples.len(), 4);
        assert!((samples[1] - 1000.0 / 32_768.0).abs() < 1e-6);
        assert!((samples[2] + 1000.0 / 32_768.0).abs() < 1e-6);
        assert!((samples[3] - 32767.0 / 32_768.0).abs() < 1e-6);
    }

    #[test]
    fn pcm_sine_reaches_the_expected_band() {
        let sample_rate = 44_100;
        let frequency = 1_000.0f32;
        let mut pcm = Vec::new();
        for index in 0..FFT_SIZE {
            let value =
                (2.0 * std::f32::consts::PI * frequency * index as f32 / sample_rate as f32).sin();
            let encoded = (value * 32_000.0) as i16;
            pcm.extend_from_slice(&encoded.to_le_bytes());
            pcm.extend_from_slice(&encoded.to_le_bytes());
        }
        let samples = decode_mono(&pcm, FFT_SIZE, 2);
        let mut analyzer = SpectrumAnalyzer::new(sample_rate, FFT_SIZE, 64);
        let values = analyzer.analyze(&samples);
        assert!(
            values.iter().copied().fold(0.0f32, f32::max) > 0.4,
            "decoded PCM should produce visible bars: {values:?}"
        );
    }

    #[test]
    fn silence_produces_quiet_bars() {
        let mut analyzer = SpectrumAnalyzer::new(44_100, FFT_SIZE, 32);
        let values = analyzer.analyze(&vec![0.0; FFT_SIZE]);
        assert!(values.iter().all(|value| *value < 0.01));
    }
}
