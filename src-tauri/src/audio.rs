use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

pub const RATE: usize = 16_000;

/// Microphone capture, downmixed and resampled to 16 kHz mono f32.
pub struct Recorder {
    pub samples: Arc<Mutex<Vec<f32>>>,
    pub level: Arc<AtomicU32>,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Recorder {
    pub fn start() -> Result<Self, String> {
        let samples = Arc::new(Mutex::new(Vec::with_capacity(RATE * 120)));
        let level = Arc::new(AtomicU32::new(0));
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let (s2, l2) = (samples.clone(), level.clone());
        // cpal streams are not Send on every platform: the stream lives and dies on this thread.
        let thread = std::thread::spawn(move || {
            let stream = match build_stream(s2, l2) {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
            let _ = ready_tx.send(Ok(()));
            let _ = stop_rx.recv();
            drop(stream);
        });
        ready_rx.recv().map_err(|e| e.to_string())??;
        Ok(Self { samples, level, stop: Some(stop_tx), thread: Some(thread) })
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Resampler {
    step: f64,
    pos: f64,
    acc: f32,
    n: u32,
}

impl Resampler {
    fn push(&mut self, s: f32, out: &mut Vec<f32>) {
        self.acc += s;
        self.n += 1;
        self.pos += self.step;
        if self.pos >= 1.0 {
            self.pos -= 1.0;
            out.push(self.acc / self.n as f32);
            self.acc = 0.0;
            self.n = 0;
        }
    }
}

fn build_stream(samples: Arc<Mutex<Vec<f32>>>, level: Arc<AtomicU32>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("aucun micro trouvé")?;
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    let channels = config.channels() as usize;
    let rate = config.sample_rate().0 as f64;
    let rs = Arc::new(Mutex::new(Resampler { step: RATE as f64 / rate, pos: 0.0, acc: 0.0, n: 0 }));
    let err = |e| eprintln!("audio: {e}");

    fn feed(data: &[f32], channels: usize, rs: &Mutex<Resampler>, samples: &Mutex<Vec<f32>>, level: &AtomicU32) {
        let mut out = Vec::with_capacity(data.len() / channels / 2 + 1);
        let mut rs = rs.lock();
        let mut sq = 0.0f32;
        for frame in data.chunks(channels) {
            let m = frame.iter().sum::<f32>() / channels as f32;
            sq += m * m;
            rs.push(m, &mut out);
        }
        let rms = (sq / (data.len() / channels).max(1) as f32).sqrt();
        level.store(rms.to_bits(), Ordering::Relaxed);
        samples.lock().extend_from_slice(&out);
    }

    let cfg: cpal::StreamConfig = config.clone().into();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &cfg,
            move |d: &[f32], _: &_| feed(d, channels, &rs, &samples, &level),
            err,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &cfg,
            move |d: &[i16], _: &_| {
                let f: Vec<f32> = d.iter().map(|&s| s as f32 / 32768.0).collect();
                feed(&f, channels, &rs, &samples, &level)
            },
            err,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &cfg,
            move |d: &[u16], _: &_| {
                let f: Vec<f32> = d.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).collect();
                feed(&f, channels, &rs, &samples, &level)
            },
            err,
            None,
        ),
        cpal::SampleFormat::I32 => device.build_input_stream(
            &cfg,
            move |d: &[i32], _: &_| {
                let f: Vec<f32> = d.iter().map(|&s| s as f32 / 2_147_483_648.0).collect();
                feed(&f, channels, &rs, &samples, &level)
            },
            err,
            None,
        ),
        f => return Err(format!("format audio non géré : {f:?}")),
    };
    stream.map_err(|e| e.to_string())
}

pub fn wav_bytes(samples: &[f32]) -> Vec<u8> {
    let spec = hound::WavSpec { channels: 1, sample_rate: RATE as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut cur = std::io::Cursor::new(Vec::with_capacity(samples.len() * 2 + 44));
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).expect("wav header");
        for &s in samples {
            let _ = w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16);
        }
        let _ = w.finalize();
    }
    cur.into_inner()
}

/// Index of the quietest 100 ms frame in `samples[from..to]`, to cut chunks between words.
pub fn quietest_cut(samples: &[f32], from: usize, to: usize) -> usize {
    let frame = RATE / 10;
    let mut best = (f32::MAX, to);
    let mut i = from;
    while i + frame <= to.min(samples.len()) {
        let e: f32 = samples[i..i + frame].iter().map(|s| s * s).sum();
        if e < best.0 {
            best = (e, i + frame / 2);
        }
        i += frame / 2;
    }
    best.1
}

pub fn peak_rms(samples: &[f32]) -> f32 {
    samples
        .chunks(RATE / 10)
        .map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt())
        .fold(0.0, f32::max)
}
