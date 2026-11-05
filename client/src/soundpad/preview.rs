//! Предпрослушивание звука на устройстве вывода без звонка: обрезка в диалоге
//! импорта и локальное проигрывание слотов на экране входа.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};

use crate::dsp::Resampler;
use crate::mixer::SAMPLE_RATE;
use crate::audio;

/// Играет `samples[start..end]` на устройство вывода; Drop останавливает.
// Зовётся из UI саундпада (задачи 8–9 плана).
#[allow(dead_code)]
pub struct Preview {
    _stream: cpal::Stream,
    stop: Arc<AtomicBool>,
    playing: Arc<AtomicBool>,
    feeder: Option<JoinHandle<()>>,
}

#[allow(dead_code)]
impl Preview {
    pub fn play(
        output: Option<&str>,
        samples: Arc<Vec<f32>>,
        start: usize,
        end: usize,
        gain: f32,
    ) -> Result<Preview, String> {
        let dev = audio::find_device(false, output)?;
        let cfg = audio::pick_config(&dev, false)?;
        let rate = cfg.sample_rate();
        let channels = cfg.channels() as usize;

        let (prod, cons) = HeapRb::<f32>::new(rate as usize / 2).split();
        let stop = Arc::new(AtomicBool::new(false));
        let playing = Arc::new(AtomicBool::new(true));

        let stream = match cfg.sample_format() {
            SampleFormat::F32 => build_output::<f32>(&dev, &cfg, channels, cons),
            SampleFormat::I16 => build_output::<i16>(&dev, &cfg, channels, cons),
            SampleFormat::I32 => build_output::<i32>(&dev, &cfg, channels, cons),
            SampleFormat::U16 => build_output::<u16>(&dev, &cfg, channels, cons),
            f => return Err(format!("формат вывода {f:?} не поддерживается")),
        }?;
        stream.play().map_err(|e| e.to_string())?;

        // Кормилец: 48 кГц → частота устройства, gain, в конце — стоп.
        let (stop2, playing2) = (stop.clone(), playing.clone());
        let feeder = std::thread::Builder::new()
            .name("voiceon-preview".into())
            .spawn(move || {
                let mut prod = prod;
                let mut rs = Resampler::new(SAMPLE_RATE, rate);
                let mut pos = start.min(samples.len());
                let end = end.min(samples.len());
                let mut resampled = Vec::with_capacity(2048);
                while pos < end && !stop2.load(Ordering::Relaxed) {
                    let n = 2048.min(end - pos);
                    resampled.clear();
                    rs.process(&samples[pos..pos + n], &mut resampled);
                    pos += n;
                    let mut i = 0;
                    while i < resampled.len() && !stop2.load(Ordering::Relaxed) {
                        for s in &mut resampled[i..] {
                            *s *= gain;
                        }
                        i += prod.push_slice(&resampled[i..]);
                        if i < resampled.len() {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    }
                }
                // Даём остатку кольца проиграться, прежде чем погасить флаг.
                while prod.occupied_len() > 0 && !stop2.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                playing2.store(false, Ordering::Relaxed);
            })
            .map_err(|e| e.to_string())?;

        Ok(Preview { _stream: stream, stop, playing, feeder: Some(feeder) })
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }
}

impl Drop for Preview {
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
