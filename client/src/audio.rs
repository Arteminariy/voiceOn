//! Звуковые устройства (cpal) и потоки обработки.
//!
//! В колбэках cpal нет блокировок и аллокаций: они только перекладывают
//! сэмплы в lock-free кольцевые буферы. Вся тяжёлая работа — в отдельных потоках.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};

use crate::capture::TxPipeline;
use crate::dsp::Resampler;
use crate::mixer::{FRAME_SAMPLES, Mixer, SAMPLE_RATE};
use crate::net::Shared;

pub struct DeviceLists {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

fn device_name(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|d| d.name().to_string())
}

pub fn list_devices() -> DeviceLists {
    let host = cpal::default_host();
    let collect = |it: Option<Vec<cpal::Device>>| -> Vec<String> {
        let mut v: Vec<String> =
            it.unwrap_or_default().iter().filter_map(device_name).collect();
        v.dedup();
        v
    };
    DeviceLists {
        inputs: collect(host.input_devices().ok().map(|d| d.collect())),
        outputs: collect(host.output_devices().ok().map(|d| d.collect())),
    }
}

fn find_device(input: bool, name: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if let Some(name) = name {
        let devs: Vec<cpal::Device> = if input {
            host.input_devices().map(|d| d.collect()).unwrap_or_default()
        } else {
            host.output_devices().map(|d| d.collect()).unwrap_or_default()
        };
        if let Some(d) = devs.into_iter().find(|d| device_name(d).as_deref() == Some(name)) {
            return Ok(d);
        }
    }
    let d = if input { host.default_input_device() } else { host.default_output_device() };
    d.ok_or_else(|| if input { "нет микрофона".into() } else { "нет устройства вывода".into() })
}

/// Предпочитаем 48 кГц (тогда ресемплер не нужен), иначе — конфиг устройства по умолчанию.
fn pick_config(dev: &cpal::Device, input: bool) -> Result<cpal::SupportedStreamConfig, String> {
    let default = if input { dev.default_input_config() } else { dev.default_output_config() }
        .map_err(|e| e.to_string())?;
    if default.sample_rate() == SAMPLE_RATE {
        return Ok(default);
    }
    let ranges: Vec<cpal::SupportedStreamConfigRange> = if input {
        dev.supported_input_configs().map(|r| r.collect()).unwrap_or_default()
    } else {
        dev.supported_output_configs().map(|r| r.collect()).unwrap_or_default()
    };
    let found = ranges.into_iter().find(|r| {
        r.channels() == default.channels()
            && r.sample_format() == default.sample_format()
            && r.min_sample_rate() <= SAMPLE_RATE
            && r.max_sample_rate() >= SAMPLE_RATE
    });
    Ok(found.map(|r| r.with_sample_rate(SAMPLE_RATE)).unwrap_or(default))
}

/// Работающие потоки звука. Пока объект жив — звонок звучит.
pub struct AudioIo {
    _input: cpal::Stream,
    _output: cpal::Stream,
    threads: Vec<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl Drop for AudioIo {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// Запоминает ошибку потока для UI. Переполнение/опустошение буфера (Xrun) — обычное дело, его игнорируем.
fn report_error(shared: &Shared, e: cpal::Error) {
    if e.kind() == cpal::ErrorKind::Xrun {
        return;
    }
    if let Ok(mut st) = shared.state.lock() {
        st.audio_error = Some((e.to_string(), std::time::Instant::now()));
    }
    (shared.repaint)();
}

pub fn start(
    shared: Arc<Shared>,
    tx: TxPipeline,
    input_name: Option<&str>,
    output_name: Option<&str>,
) -> Result<AudioIo, String> {
    // ---- Захват ----
    let in_dev = find_device(true, input_name)?;
    let in_cfg = pick_config(&in_dev, true)?;
    let in_rate = in_cfg.sample_rate();
    let in_ch = in_cfg.channels() as usize;
    let (in_prod, mut in_cons) = HeapRb::<f32>::new(in_rate as usize / 2).split();
    let err_shared = shared.clone();
    let input = match in_cfg.sample_format() {
        SampleFormat::F32 => build_input::<f32>(&in_dev, &in_cfg, in_ch, in_prod, err_shared),
        SampleFormat::I16 => build_input::<i16>(&in_dev, &in_cfg, in_ch, in_prod, err_shared),
        SampleFormat::I32 => build_input::<i32>(&in_dev, &in_cfg, in_ch, in_prod, err_shared),
        SampleFormat::U16 => build_input::<u16>(&in_dev, &in_cfg, in_ch, in_prod, err_shared),
        f => Err(format!("формат микрофона {f:?} не поддерживается")),
    }?;

    // ---- Воспроизведение ----
    let out_dev = find_device(false, output_name)?;
    let out_cfg = pick_config(&out_dev, false)?;
    let out_rate = out_cfg.sample_rate();
    let out_ch = out_cfg.channels() as usize;
    let (mut out_prod, out_cons) = HeapRb::<f32>::new(out_rate as usize / 2).split();
    let err_shared = shared.clone();
    let output = match out_cfg.sample_format() {
        SampleFormat::F32 => build_output::<f32>(&out_dev, &out_cfg, out_ch, out_cons, err_shared),
        SampleFormat::I16 => build_output::<i16>(&out_dev, &out_cfg, out_ch, out_cons, err_shared),
        SampleFormat::I32 => build_output::<i32>(&out_dev, &out_cfg, out_ch, out_cons, err_shared),
        SampleFormat::U16 => build_output::<u16>(&out_dev, &out_cfg, out_ch, out_cons, err_shared),
        f => Err(format!("формат вывода {f:?} не поддерживается")),
    }?;

    input.play().map_err(|e| e.to_string())?;
    output.play().map_err(|e| e.to_string())?;

    let mut threads = Vec::new();

    // Поток захвата: ресемплинг до 48 кГц и конвейер передачи.
    let sh = shared.clone();
    threads.push(
        std::thread::Builder::new()
            .name("voiceon-capture".into())
            .spawn(move || {
                let mut tx = tx;
                let mut rs = Resampler::new(in_rate, SAMPLE_RATE);
                let mut chunk = vec![0.0f32; 1024];
                let mut resampled = Vec::with_capacity(2048);
                while !sh.stop.load(Ordering::Relaxed) {
                    let n = in_cons.pop_slice(&mut chunk);
                    if n == 0 {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    resampled.clear();
                    rs.process(&chunk[..n], &mut resampled);
                    tx.push(&resampled);
                }
            })
            .map_err(|e| e.to_string())?,
    );

    // Поток воспроизведения: часы звуковой карты задают темп извлечения из
    // jitter buffer — держим в кольце ~20–40 мс и доливаем по 20 мс.
    let sh = shared.clone();
    threads.push(
        std::thread::Builder::new()
            .name("voiceon-playback".into())
            .spawn(move || {
                let mut mixer = Mixer::new();
                let mut rs = Resampler::new(SAMPLE_RATE, out_rate);
                let mut frame = [0.0f32; FRAME_SAMPLES];
                let mut resampled = Vec::with_capacity(2048);
                let low_water = (out_rate as usize) / 50; // 20 мс
                while !sh.stop.load(Ordering::Relaxed) {
                    if out_prod.occupied_len() >= low_water {
                        std::thread::sleep(Duration::from_millis(3));
                        continue;
                    }
                    mixer.mix(&sh, &mut frame);
                    resampled.clear();
                    rs.process(&frame, &mut resampled);
                    out_prod.push_slice(&resampled);
                }
            })
            .map_err(|e| e.to_string())?,
    );

    Ok(AudioIo { _input: input, _output: output, threads, shared })
}

fn build_input<T>(
    dev: &cpal::Device,
    cfg: &cpal::SupportedStreamConfig,
    channels: usize,
    mut prod: ringbuf::HeapProd<f32>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let inv = 1.0 / channels as f32;
    dev.build_input_stream::<T, _, _>(
        cfg.config(),
        move |data: &[T], _| {
            for frame in data.chunks_exact(channels) {
                let mut s = 0.0f32;
                for &x in frame {
                    s += x.to_sample::<f32>();
                }
                let _ = prod.try_push(s * inv);
            }
        },
        move |e| report_error(&shared, e),
        None,
    )
    .map_err(|e| format!("микрофон: {e}"))
}

fn build_output<T>(
    dev: &cpal::Device,
    cfg: &cpal::SupportedStreamConfig,
    channels: usize,
    mut cons: ringbuf::HeapCons<f32>,
    shared: Arc<Shared>,
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
        move |e| report_error(&shared, e),
        None,
    )
    .map_err(|e| format!("вывод: {e}"))
}
