//! Локальный проигрыватель саундпада: экран входа и предпрослушивание в диалоге
//! обрезки. Один поток вывода; инстансы и громкость — те же примитивы ядра (`Pad`),
//! что и в эфире, поэтому наложение, voice stealing и живая громкость совпадают.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use voiceon_core::pad::Pad;

use crate::audio;
use crate::dsp::Resampler;
use crate::mixer::{FRAME_SAMPLES, SAMPLE_RATE};

/// Локальный проигрыватель: поток вывода + поток-кормилец, миксующий инстансы.
pub struct LocalPad {
    /// Инстансы и громкость (атомик — слайдер действует на лету).
    pub pad: Arc<Pad>,
    _stream: cpal::Stream,
    stop: Arc<AtomicBool>,
    last_active: Arc<Mutex<Instant>>,
    feeder: Option<JoinHandle<()>>,
}

impl LocalPad {
    pub fn start(output: Option<&str>) -> Result<LocalPad, String> {
        let dev = audio::find_device(false, output)?;
        let cfg = audio::pick_config(&dev, false)?;
        let rate = cfg.sample_rate();
        let channels = cfg.channels() as usize;

        let (prod, cons) = HeapRb::<f32>::new(rate as usize / 2).split();
        let stop = Arc::new(AtomicBool::new(false));
        let last_active = Arc::new(Mutex::new(Instant::now()));
        let pad = Arc::new(Pad::new());

        let stream = match cfg.sample_format() {
            SampleFormat::F32 => build_output::<f32>(&dev, &cfg, channels, cons),
            SampleFormat::I16 => build_output::<i16>(&dev, &cfg, channels, cons),
            SampleFormat::I32 => build_output::<i32>(&dev, &cfg, channels, cons),
            SampleFormat::U16 => build_output::<u16>(&dev, &cfg, channels, cons),
            f => return Err(format!("формат вывода {f:?} не поддерживается")),
        }?;
        stream.play().map_err(|e| e.to_string())?;

        let (pad2, stop2, last2) = (pad.clone(), stop.clone(), last_active.clone());
        let feeder = std::thread::Builder::new()
            .name("voiceon-soundpad".into())
            .spawn(move || {
                let mut prod = prod;
                let mut rs = Resampler::new(SAMPLE_RATE, rate);
                let mut frame = [0.0f32; FRAME_SAMPLES];
                let mut resampled = Vec::with_capacity(2048);
                let low_water = (rate as usize) / 50; // ~20 мс
                while !stop2.load(Ordering::Relaxed) {
                    if prod.occupied_len() >= low_water {
                        std::thread::sleep(Duration::from_millis(3));
                        continue;
                    }
                    frame.fill(0.0);
                    pad2.mix_monitor(&mut frame, false);
                    if pad2.any_active() {
                        *last2.lock().unwrap() = Instant::now();
                    }
                    resampled.clear();
                    rs.process(&frame, &mut resampled);
                    let mut i = 0;
                    while i < resampled.len() && !stop2.load(Ordering::Relaxed) {
                        i += prod.push_slice(&resampled[i..]);
                        if i < resampled.len() {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        Ok(LocalPad { pad, _stream: stream, stop, last_active, feeder: Some(feeder) })
    }

    /// Сколько уже не играет ни один инстанс (для сброса потока в простое).
    pub fn idle_for(&self) -> Duration {
        self.last_active.lock().unwrap().elapsed()
    }
}

impl Drop for LocalPad {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.feeder.take() {
            let _ = t.join();
        }
    }
}

fn build_output<T>(
    dev: &cpal::Device,
    cfg: &cpal::SupportedStreamConfig,
    channels: usize,
    mut cons: ringbuf::HeapCons<f32>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    dev.build_output_stream::<T, _, _>(
        cfg.config(),
        move |data: &mut [T], _| {
            for frame in data.chunks_exact_mut(channels) {
                let s = T::from_sample(cons.try_pop().unwrap_or(0.0));
                for x in frame {
                    *x = s;
                }
            }
        },
        |_| {},
        None,
    )
    .map_err(|e| format!("вывод: {e}"))
}
