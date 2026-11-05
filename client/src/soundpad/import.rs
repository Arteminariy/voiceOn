//! Импорт звуков: файл → декод → моно → 48 кГц → обрезка → фейды → нормализация → WAV.

use std::path::{Path, PathBuf};

use voiceon_core::dsp::rms;
use voiceon_core::mixer::SAMPLE_RATE;

/// Максимальная длина звука в слоте, секунд.
pub const MAX_CLIP_SECS: f32 = 10.0;
/// Источник длиннее этого отклоняется: мемы короткие, а память не резиновая.
const MAX_SOURCE_SECS: f32 = 180.0;
/// Целевой RMS после нормализации — соразмерно громкости голоса.
const TARGET_RMS: f32 = 0.1;
/// Потолок усиления при нормализации (+12 дБ): тише — значит судьба мема такая.
const MAX_GAIN: f32 = 4.0;
/// Пик после усиления не выше этого.
const PEAK_CEILING: f32 = 0.95;
/// Фейды по краям куска, чтобы не щёлкало.
const FADE_MS: usize = 5;

/// Декодирует файл в 48 кГц моно; дольше MAX_SOURCE_SECS — ошибка.
pub fn decode(path: &Path) -> Result<Vec<f32>, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::errors::Error as SymphoniaError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).map_err(|e| format!("не открыть файл: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|_| "непонятный формат файла (нужны mp3, wav, ogg или flac)".to_string())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("в файле нет звуковой дорожки")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("кодек не поддерживается: {e}"))?;

    let mut mono: Vec<f32> = Vec::new();
    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("чтение файла: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue, // битый пакет — пропускаем
            Err(e) => return Err(format!("декодирование: {e}")),
        };
        let spec = *decoded.spec();
        let channels = spec.channels.count().max(1);
        rate = spec.rate;
        let mut buf = SampleBuffer::<f32>::new(decoded.frames() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        for frame in buf.samples().chunks(channels) {
            mono.push(frame.iter().sum::<f32>() / channels as f32);
        }
        if rate > 0 && mono.len() as f32 / rate as f32 > MAX_SOURCE_SECS {
            return Err(format!(
                "файл длиннее {} мин — обрежь заранее",
                (MAX_SOURCE_SECS / 60.0) as u32
            ));
        }
    }
    if mono.is_empty() {
        return Err("в файле нет звука".into());
    }
    if rate == 0 {
        return Err("не удалось узнать частоту дискретизации".into());
    }
    if rate == SAMPLE_RATE {
        return Ok(mono);
    }
    resample(&mono, rate)
}

/// Линейный ресемпл в 48 кГц через sinc-интерполятор rubato.
fn resample(input: &[f32], from: u32) -> Result<Vec<f32>, String> {
    use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 128,
        window: WindowFunction::BlackmanHarris2,
    };
    let chunk = 4096.min(input.len().max(1));
    let mut rs = SincFixedIn::<f32>::new(SAMPLE_RATE as f64 / from as f64, 2.0, params, chunk, 1)
        .map_err(|e| format!("ресемплер: {e}"))?;
    let mut out = Vec::with_capacity(input.len() * SAMPLE_RATE as usize / from as usize + chunk);
    let mut pos = 0;
    while pos < input.len() {
        let n = chunk.min(input.len() - pos);
        let block = rs
            .process_partial(Some(&[&input[pos..pos + n]]), None)
            .map_err(|e| format!("ресемплинг: {e}"))?;
        out.extend_from_slice(&block[0]);
        pos += n;
    }
    // Добиваем хвост sinc-фильтра тишиной, иначе короткие файлы теряют конец.
    let delay = rs.output_delay();
    let want = (input.len() as f64 * SAMPLE_RATE as f64 / from as f64).round() as usize;
    let zeros = vec![0.0f32; chunk];
    for _ in 0..8 {
        if out.len() >= delay + want {
            break;
        }
        let block = rs
            .process_partial(Some(&[&zeros[..]]), None)
            .map_err(|e| format!("ресемплинг: {e}"))?;
        if block[0].is_empty() {
            break;
        }
        out.extend_from_slice(&block[0]);
    }
    // Синк-фильтр даёт задержку в начале — срезаем до точной длины.
    let start = delay.min(out.len());
    let end = (start + want).min(out.len());
    Ok(out[start..end].to_vec())
}

/// min/max по `buckets` столбцам — волна для диалога обрезки.
pub fn peaks(samples: &[f32], buckets: usize) -> Vec<(f32, f32)> {
    if samples.is_empty() || buckets == 0 {
        return Vec::new();
    }
    let step = samples.len().div_ceil(buckets);
    samples
        .chunks(step)
        .map(|c| {
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for &s in c {
                lo = lo.min(s);
                hi = hi.max(s);
            }
            (lo, hi)
        })
        .collect()
}

/// Обрезка [start, end) секунд (не длиннее MAX_CLIP_SECS), фейды, нормализация.
/// Короче 100 мс — пустой клип (звонить с этим в слот бессмысленно).
pub fn prepare_clip(source: &[f32], start: f32, end: f32) -> Vec<f32> {
    let dur = source.len() as f32 / SAMPLE_RATE as f32;
    if dur < 0.1 {
        return Vec::new();
    }
    let start = start.clamp(0.0, dur - 0.1);
    let end = end.clamp(start + 0.1, dur).min(start + MAX_CLIP_SECS);
    let (a, b) = ((start * SAMPLE_RATE as f32) as usize, (end * SAMPLE_RATE as f32) as usize);
    let mut clip = source[a.min(source.len())..b.min(source.len())].to_vec();

    let fade = FADE_MS * SAMPLE_RATE as usize / 1000;
    let fade = fade.min(clip.len() / 2);
    for i in 0..fade {
        let t = i as f32 / fade as f32;
        clip[i] *= t;
        let j = clip.len() - 1 - i;
        clip[j] *= t;
    }
    normalize(&mut clip);
    clip
}

/// RMS к TARGET_RMS, усиление ≤ MAX_GAIN, пики ≤ PEAK_CEILING; тишину не трогаем.
fn normalize(samples: &mut [f32]) {
    let r = rms(samples);
    if r < 1e-6 {
        return;
    }
    let gain = (TARGET_RMS / r).min(MAX_GAIN);
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs())) * gain;
    let total = if peak > PEAK_CEILING { gain * PEAK_CEILING / peak } else { gain };
    for s in samples.iter_mut() {
        *s *= total;
    }
}

/// Пишет 48 кГц моно 16-бит WAV.
pub fn save_wav(samples: &[f32], path: &Path) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| format!("wav: {e}"))?;
    for &s in samples {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16).map_err(|e| format!("wav: {e}"))?;
    }
    w.finalize().map_err(|e| format!("wav: {e}"))
}

/// Читает WAV слота обратно в сэмплы; битый или усечённый файл — ошибка.
pub fn load_wav(path: &Path) -> Result<Vec<f32>, String> {
    let r = hound::WavReader::open(path).map_err(|e| format!("wav: {e}"))?;
    let spec = r.spec();
    if spec.sample_rate != SAMPLE_RATE || spec.channels != 1 {
        return Err("не файл саундпада (нужен 48 кГц моно)".into());
    }
    let raw: Vec<i16> = r
        .into_samples::<i16>()
        .collect::<Result<Vec<i16>, _>>()
        .map_err(|e| format!("wav: {e}"))?;
    let out: Vec<f32> = raw.into_iter().map(|s| s as f32 / 32768.0).collect();
    if out.len() as f32 / SAMPLE_RATE as f32 > MAX_CLIP_SECS + 1.0 {
        return Err("файл слота длиннее положенного".into());
    }
    Ok(out)
}

/// Каталог со звуками слотов.
pub fn sounds_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("voiceon").join("sounds"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("voiceon-test-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sine(freq: f32, rate: u32, secs: f32, amp: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n).map(|i| (i as f32 * freq * std::f32::consts::TAU / rate as f32).sin() * amp).collect()
    }

    fn rms_of(v: &[f32]) -> f32 {
        (v.iter().map(|s| s * s).sum::<f32>() / v.len() as f32).sqrt()
    }

    fn write_wav(path: &std::path::Path, channels: u16, rate: u32, secs: f32) {
        let n = (rate as f32 * secs) as usize;
        write_wav_samples(path, channels, rate, n);
    }

    fn write_wav_samples(path: &std::path::Path, channels: u16, rate: u32, samples: usize) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..samples {
            let s = ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5 * 32767.0) as i16;
            for _ in 0..channels {
                w.write_sample(s).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    #[test]
    fn trim_clamps_to_ten_seconds() {
        let src = vec![0.5f32; 20 * 48000];
        let clip = prepare_clip(&src, 5.0, 20.0);
        let want = (MAX_CLIP_SECS * 48000.0) as usize;
        assert!((clip.len() as i64 - want as i64).abs() <= 2, "{}", clip.len());
    }

    #[test]
    fn fades_silence_endpoints() {
        let src = vec![0.5f32; 3 * 48000];
        let clip = prepare_clip(&src, 0.0, 2.0);
        assert_eq!(clip[0], 0.0);
        assert_eq!(*clip.last().unwrap(), 0.0);
    }

    #[test]
    fn normalize_hits_target_rms() {
        let src = sine(440.0, 48000, 2.0, 0.05); // RMS ≈ 0.035, усиление ≈ 2.8 < MAX_GAIN
        let clip = prepare_clip(&src, 0.0, 2.0);
        let r = rms_of(&clip);
        assert!((r - TARGET_RMS).abs() / TARGET_RMS < 0.1, "{r}");
    }

    #[test]
    fn normalize_silent_clip_no_panic() {
        let src = vec![0.0f32; 48000];
        let clip = prepare_clip(&src, 0.0, 1.0);
        assert!(clip.iter().all(|s| s.is_finite() && *s == 0.0));
    }

    #[test]
    fn wav_roundtrip() {
        let d = tmpdir("wav");
        let p = d.join("a.wav");
        let src = sine(440.0, 48000, 1.0, 0.5);
        save_wav(&src, &p).unwrap();
        let back = load_wav(&p).unwrap();
        assert_eq!(back.len(), src.len());
        assert!((back[100] - src[100]).abs() < 0.0001);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn load_wav_rejects_truncated() {
        let d = tmpdir("trunc");
        let p = d.join("t.wav");
        let src = sine(440.0, 48000, 0.1, 0.5);
        save_wav(&src, &p).unwrap();
        // Валидный заголовок, но данные обрезаны.
        let mut bytes = std::fs::read(&p).unwrap();
        bytes.truncate(bytes.len() - 100);
        std::fs::write(&p, bytes).unwrap();
        assert!(load_wav(&p).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn decode_wav_file() {
        let d = tmpdir("dec");
        let p = d.join("s.wav");
        write_wav(&p, 2, 44100, 1.0);
        let out = decode(&p).unwrap();
        assert!((out.len() as i64 - 48000).abs() <= 480, "{}", out.len());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn decode_garbage_errors() {
        let d = tmpdir("garbage");
        let p = d.join("g.mp3");
        std::fs::write(&p, [0xDEu8, 0xAD, 0xBE, 0xEF].repeat(64)).unwrap();
        assert!(decode(&p).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn decode_too_long_errors() {
        let d = tmpdir("long");
        let p = d.join("l.wav");
        write_wav(&p, 1, 48000, 181.0);
        let e = decode(&p).unwrap_err();
        assert!(e.contains("мин"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn decode_short_file_keeps_length() {
        let d = tmpdir("short");
        let p = d.join("s.wav");
        write_wav(&p, 1, 44100, 0.01); // 10 мс
        let out = decode(&p).unwrap();
        assert!((out.len() as i64 - 480).abs() <= 4, "{}", out.len());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn decode_tiny_file_not_empty() {
        let d = tmpdir("tiny");
        let p = d.join("t.wav");
        write_wav_samples(&p, 1, 44100, 100); // 100 сэмплов
        let out = decode(&p).unwrap();
        assert!(out.len() >= 100, "{}", out.len());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn prepare_clip_tolerates_tiny_and_near_end() {        // Пустой и короче 100 мс — пустой клип, без паники.
        assert!(prepare_clip(&[], 0.0, 1.0).is_empty());
        assert!(prepare_clip(&vec![0.5; 480], 0.0, 1.0).is_empty()); // 10 мс
        assert!(prepare_clip(&vec![0.5; 4799], 0.0, 1.0).is_empty()); // 99.9 мс
        // Ровно 100 мс — уже валидно.
        assert_eq!(prepare_clip(&vec![0.5; 4800], 0.0, 1.0).len(), 4800);
        // start в последних 100 мс — отступает назад, не паникует.
        assert_eq!(prepare_clip(&vec![0.5; 48000], 0.99, 1.0).len(), 4800);
    }
}
