//! Декодирование и сведение потоков участников в один 20-мс кадр 48 кГц.

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use crate::dsp::soft_clip;
use crate::jitter::Frame;
use crate::net::Shared;

pub const SAMPLE_RATE: u32 = 48_000;
pub const FRAME_SAMPLES: usize = 960; // 20 мс

pub struct Mixer {
    decoders: HashMap<u32, opus::Decoder>,
    tmp: Vec<f32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer::new()
    }
}

impl Mixer {
    pub fn new() -> Mixer {
        Mixer { decoders: HashMap::new(), tmp: vec![0.0; FRAME_SAMPLES] }
    }

    /// Достаёт по кадру из каждого jitter buffer и сводит их в `out`.
    pub fn mix(&mut self, shared: &Shared, out: &mut [f32; FRAME_SAMPLES]) {
        out.fill(0.0);
        let frames: Vec<(u32, Frame)> = {
            let mut jbs = shared.jitter.lock().unwrap();
            self.decoders.retain(|sid, _| jbs.contains_key(sid));
            jbs.iter_mut().map(|(&sid, jb)| (sid, jb.pop())).collect()
        };
        if frames.iter().all(|(_, f)| *f == Frame::Silent) {
            return;
        }
        let gains: HashMap<u32, f32> = {
            let st = shared.state.lock().unwrap();
            frames
                .iter()
                .map(|(sid, _)| {
                    let g = st.nick_of(*sid).and_then(|n| st.volumes.get(n)).copied().unwrap_or(1.0);
                    (*sid, g)
                })
                .collect()
        };
        let deaf = shared.controls.deafened.load(Ordering::Relaxed);

        for (sid, frame) in frames {
            let (data, fec): (&[u8], bool) = match &frame {
                Frame::Silent => continue,
                Frame::Packet(d) => (d, false),
                Frame::Fec(d) => (d, true),
                Frame::Lost => (&[], false),
            };
            let dec = match self.decoders.entry(sid) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    match opus::Decoder::new(SAMPLE_RATE, opus::Channels::Mono) {
                        Ok(d) => e.insert(d),
                        Err(_) => continue,
                    }
                }
            };
            // Декодируем даже в режиме «без звука», чтобы состояние декодера не рвалось.
            let n = match dec.decode_float(data, &mut self.tmp, fec) {
                Ok(n) => n.min(FRAME_SAMPLES),
                Err(_) => continue,
            };
            if deaf {
                continue;
            }
            let g = gains.get(&sid).copied().unwrap_or(1.0);
            for (o, s) in out.iter_mut().zip(&self.tmp[..n]) {
                *o += s * g;
            }
        }
        for s in out.iter_mut() {
            *s = soft_clip(*s);
        }
    }
}
