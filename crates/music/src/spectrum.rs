use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use rtrb::{PopError, RingBuffer};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

const N_BANDS: usize = 32;
const FFT_SIZE: usize = 1024;
const RING_CAPACITY: usize = FFT_SIZE * 8;
const MIN_FREQ: f32 = 40.;
const MAX_FREQ: f32 = 16_000.;
const GAIN: f32 = 8.;
const ATTACK: f32 = 0.9;
const DECAY: f32 = 0.12;
const IDLE_POLL: Duration = Duration::from_millis(4);
const PULSE_BASS: (f32, f32) = (18., 140.);
const PULSE_MIDS: (f32, f32) = (140., 850.);
const PULSE_HIGHS: f32 = 850.;

#[derive(Clone)]
pub struct Spectrum {
    bands: Arc<Vec<AtomicU32>>,
    peak: Arc<AtomicU32>,
    rms: Arc<AtomicU32>,
    bass: Arc<AtomicU32>,
    mids: Arc<AtomicU32>,
    highs: Arc<AtomicU32>,
}

impl Spectrum {
    pub fn new() -> Self {
        Self {
            bands: Arc::new((0..N_BANDS).map(|_| AtomicU32::new(0)).collect()),
            peak: Arc::new(AtomicU32::new(0)),
            rms: Arc::new(AtomicU32::new(0)),
            bass: Arc::new(AtomicU32::new(0)),
            mids: Arc::new(AtomicU32::new(0)),
            highs: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn bands(&self) -> Vec<f32> {
        self.bands
            .iter()
            .map(|band| f32::from_bits(band.load(Ordering::Relaxed)))
            .collect()
    }

    pub fn peak(&self) -> f32 {
        load(&self.peak)
    }

    pub fn rms(&self) -> f32 {
        load(&self.rms)
    }

    pub fn bass(&self) -> f32 {
        load(&self.bass)
    }

    pub fn mids(&self) -> f32 {
        load(&self.mids)
    }

    pub fn highs(&self) -> f32 {
        load(&self.highs)
    }

    fn set(&self, index: usize, value: f32) {
        self.bands[index].store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn attach(&self, rate: u32, channels: u16) -> Tap {
        let (producer, consumer) = RingBuffer::<f32>::new(RING_CAPACITY);
        let target = self.clone();
        let spawned = std::thread::Builder::new()
            .name("spectrum".to_owned())
            .spawn(move || analyze(consumer, target, rate.max(1), channels.max(1) as usize));
        if let Err(error) = spawned {
            log::error!("spectrum: cannot spawn analyzer thread: {error}");
        }
        Tap { producer }
    }
}

impl Default for Spectrum {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Tap {
    producer: rtrb::Producer<f32>,
}

impl Tap {
    pub fn push(&mut self, sample: f32) {
        self.producer.push(sample).ok();
    }
}

fn analyze(mut consumer: rtrb::Consumer<f32>, spectrum: Spectrum, rate: u32, channels: usize) {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let window = hann_window();
    let edges = band_edges(rate as f32);
    let mut smoothed = [0f32; N_BANDS];
    let mut mono = [0f32; FFT_SIZE];
    let mut filled = 0usize;
    let mut lane = vec![0f32; channels];
    let mut lane_index = 0usize;
    let mut buffer = vec![Complex32::default(); FFT_SIZE];

    loop {
        let sample = match consumer.pop() {
            Ok(sample) => sample,
            Err(PopError::Empty) if consumer.is_abandoned() => return,
            Err(PopError::Empty) => {
                std::thread::sleep(IDLE_POLL);
                continue;
            }
        };

        lane[lane_index] = sample;
        lane_index += 1;
        if lane_index < channels {
            continue;
        }
        lane_index = 0;

        mono[filled] = lane.iter().sum::<f32>() / channels as f32;
        filled += 1;
        if filled < FFT_SIZE {
            continue;
        }
        filled = 0;

        let mut peak = 0f32;
        let mut energy = 0f32;
        for sample in mono {
            peak = peak.max(sample.abs());
            energy += sample * sample;
        }
        store(&spectrum.peak, peak);
        store(&spectrum.rms, (energy / FFT_SIZE as f32).sqrt());

        for ((slot, sample), weight) in buffer.iter_mut().zip(mono).zip(&window) {
            *slot = Complex32::new(sample * weight, 0.);
        }
        fft.process(&mut buffer);
        store(
            &spectrum.bass,
            range_mean(&buffer, rate, PULSE_BASS.0, PULSE_BASS.1),
        );
        store(
            &spectrum.mids,
            range_mean(&buffer, rate, PULSE_MIDS.0, PULSE_MIDS.1),
        );
        store(
            &spectrum.highs,
            range_mean(&buffer, rate, PULSE_HIGHS, rate as f32 / 2.),
        );

        for (band, edge) in edges.windows(2).enumerate() {
            let lo = edge[0];
            let hi = edge[1].max(lo + 1);
            let magnitude = buffer[lo..hi]
                .iter()
                .map(|bin| bin.norm())
                .fold(0f32, f32::max);
            let target = (magnitude * GAIN / (FFT_SIZE as f32 / 2.)).sqrt().min(1.);
            let rate = match target > smoothed[band] {
                true => ATTACK,
                false => DECAY,
            };
            smoothed[band] += (target - smoothed[band]) * rate;
            spectrum.set(band, smoothed[band]);
        }
    }
}

fn hann_window() -> [f32; FFT_SIZE] {
    let mut window = [0f32; FFT_SIZE];
    for (i, value) in window.iter_mut().enumerate() {
        *value = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (FFT_SIZE - 1) as f32).cos();
    }
    window
}

fn band_edges(rate: f32) -> Vec<usize> {
    let nyquist = rate / 2.;
    let max_freq = MAX_FREQ.min(nyquist);
    let min_freq = MIN_FREQ.min(max_freq * 0.5).max(1.);
    let bin_hz = rate / FFT_SIZE as f32;

    (0..=N_BANDS)
        .map(|i| {
            let t = i as f32 / N_BANDS as f32;
            let freq = min_freq * (max_freq / min_freq).powf(t);
            ((freq / bin_hz) as usize).clamp(1, FFT_SIZE / 2 - 1)
        })
        .collect()
}

fn range_mean(buffer: &[Complex32], rate: u32, hz_lo: f32, hz_hi: f32) -> f32 {
    let nyquist = rate as f32 / 2.;
    let hz_hi = hz_hi.min(nyquist);
    if hz_lo >= hz_hi {
        return 0.;
    }
    let bin_hz = rate as f32 / FFT_SIZE as f32;
    let lo = ((hz_lo / bin_hz) as usize).clamp(1, FFT_SIZE / 2 - 1);
    let hi = ((hz_hi / bin_hz) as usize).clamp(lo, FFT_SIZE / 2 - 1);
    let span = &buffer[lo..=hi];
    let sum: f32 = span.iter().map(|bin| bin.norm() / FFT_SIZE as f32).sum();
    sum / span.len() as f32
}

fn store(slot: &AtomicU32, value: f32) {
    slot.store(value.clamp(0., 1.).to_bits(), Ordering::Relaxed);
}

fn load(slot: &AtomicU32) -> f32 {
    f32::from_bits(slot.load(Ordering::Relaxed)).clamp(0., 1.)
}
