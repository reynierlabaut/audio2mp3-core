//! Conversión mínima de PCM para Android (feature `pcm-convert`, spec §9):
//! bytes del decoder (S16/F32, N canales, X Hz) → downmix a 1–2 canales →
//! resampling a 32/44.1/48 kHz → S16 intercalado.
//!
//! Procesa trama a trama (una muestra por canal), así que la salida es
//! idéntica sea cual sea el troceo de la entrada, incluso si un bloque corta
//! una muestra a la mitad (los bytes sobrantes se guardan para la siguiente
//! llamada).

mod downmix;
mod resample;

use crate::error::Error;
use downmix::Downmix;
use resample::Resampler;

/// Máximo de canales de entrada (7.1).
pub const MAX_INPUT_CHANNELS: u8 = 8;
const MIN_RATE: u32 = 8_000;
const MAX_RATE: u32 = 192_000;
const MAX_FRAME_BYTES: usize = MAX_INPUT_CHANNELS as usize * 4;

/// Formato de las muestras de entrada, en orden de bytes nativo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    S16,
    F32,
}

impl SampleFormat {
    fn bytes(self) -> usize {
        match self {
            SampleFormat::S16 => 2,
            SampleFormat::F32 => 4,
        }
    }
}

/// Tasa de salida para una tasa de entrada (ratios enteros cuando se puede).
pub fn output_rate(in_rate: u32) -> u32 {
    match in_rate {
        32_000 | 44_100 | 48_000 => in_rate,
        8_000 | 16_000 => 32_000,
        11_025 | 22_050 | 88_200 | 176_400 => 44_100,
        _ => 48_000,
    }
}

/// Convierte PCM arbitrario en S16 apto para `Mp3Encoder`.
///
/// Después de `new()`, `process_into` y `flush_into` no reservan memoria.
pub struct PcmConverter {
    format: SampleFormat,
    in_channels: usize,
    frame_bytes: usize,
    pending: [u8; MAX_FRAME_BYTES],
    pending_len: usize,
    downmix: Downmix,
    resampler: Resampler,
    out_rate: u32,
    finished: bool,
}

impl PcmConverter {
    /// `in_rate`: 8 000–192 000 Hz; `in_channels`: 1–8 (orden WAVE:
    /// L, R, C, LFE, Ls, Rs…).
    pub fn new(in_rate: u32, in_channels: u8, in_format: SampleFormat) -> Result<Self, Error> {
        if !(MIN_RATE..=MAX_RATE).contains(&in_rate) {
            return Err(Error::InvalidSampleRate);
        }
        if !(1..=MAX_INPUT_CHANNELS).contains(&in_channels) {
            return Err(Error::InvalidChannels);
        }
        let in_channels = usize::from(in_channels);
        let downmix = Downmix::new(in_channels);
        let out_rate = output_rate(in_rate);
        Ok(Self {
            format: in_format,
            in_channels,
            frame_bytes: in_channels * in_format.bytes(),
            pending: [0; MAX_FRAME_BYTES],
            pending_len: 0,
            resampler: Resampler::new(in_rate, out_rate, downmix.out_channels()),
            downmix,
            out_rate,
            finished: false,
        })
    }

    pub fn out_rate(&self) -> u32 {
        self.out_rate
    }

    pub fn out_channels(&self) -> u8 {
        self.downmix.out_channels() as u8
    }

    /// Máximo de muestras (intercaladas) que puede escribir `process_into`
    /// con `in_bytes` bytes de entrada.
    pub fn max_output_samples(&self, in_bytes: usize) -> usize {
        let frames = self.pending_len.saturating_add(in_bytes) / self.frame_bytes;
        self.resampler.max_output_frames(frames) * self.downmix.out_channels()
    }

    /// Máximo de muestras que puede escribir `flush_into`.
    pub fn max_flush_samples(&self) -> usize {
        self.resampler.max_flush_frames() * self.downmix.out_channels()
    }

    /// Convierte `input` (bytes crudos del decoder) y escribe S16 intercalado
    /// en `out`. Devuelve las muestras escritas. Si `out` es menor que
    /// `max_output_samples`, devuelve `BufferTooSmall` sin consumir nada.
    pub fn process_into(&mut self, input: &[u8], out: &mut [i16]) -> Result<usize, Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        if out.len() < self.max_output_samples(input.len()) {
            return Err(Error::BufferTooSmall);
        }
        let mut written = 0;
        let mut rest = input;
        if self.pending_len > 0 {
            let take = (self.frame_bytes - self.pending_len).min(rest.len());
            let (head, tail) = rest.split_at(take);
            self.pending
                .get_mut(self.pending_len..self.pending_len + take)
                .ok_or(Error::Internal)?
                .copy_from_slice(head);
            self.pending_len += take;
            rest = tail;
            if self.pending_len == self.frame_bytes {
                let frame = self.pending;
                self.pending_len = 0;
                let frame = frame.get(..self.frame_bytes).ok_or(Error::Internal)?;
                written +=
                    self.push_frame(frame, out.get_mut(written..).ok_or(Error::Internal)?)?;
            }
        }
        let whole = rest.len() - rest.len() % self.frame_bytes;
        let (frames, tail) = rest.split_at(whole);
        for frame in frames.chunks_exact(self.frame_bytes) {
            written += self.push_frame(frame, out.get_mut(written..).ok_or(Error::Internal)?)?;
        }
        if !tail.is_empty() {
            self.pending
                .get_mut(..tail.len())
                .ok_or(Error::Internal)?
                .copy_from_slice(tail);
            self.pending_len = tail.len();
        }
        Ok(written)
    }

    /// Emite las últimas muestras retenidas por el filtro y termina. Los bytes
    /// de una trama incompleta se descartan.
    pub fn flush_into(&mut self, out: &mut [i16]) -> Result<usize, Error> {
        if self.finished {
            return Err(Error::Finished);
        }
        if out.len() < self.max_flush_samples() {
            return Err(Error::BufferTooSmall);
        }
        let written = self.resampler.flush(out)?;
        self.pending_len = 0;
        self.finished = true;
        Ok(written)
    }

    fn push_frame(&mut self, frame: &[u8], out: &mut [i16]) -> Result<usize, Error> {
        let mut samples = [0.0f32; MAX_INPUT_CHANNELS as usize];
        let bytes = self.format.bytes();
        for (s, raw) in samples
            .iter_mut()
            .zip(frame.chunks_exact(bytes))
            .take(self.in_channels)
        {
            *s = decode_sample(self.format, raw);
        }
        let mut mixed = [0.0f32; 2];
        let channels = self.downmix.apply(
            samples.get(..self.in_channels).ok_or(Error::Internal)?,
            &mut mixed,
        );
        self.resampler
            .push(mixed.get(..channels).ok_or(Error::Internal)?, out)
    }
}

/// Muestra en unidades de S16 (−32768..32767) como f32.
fn decode_sample(format: SampleFormat, raw: &[u8]) -> f32 {
    match (format, raw) {
        (SampleFormat::S16, &[a, b]) => f32::from(i16::from_ne_bytes([a, b])),
        (SampleFormat::F32, &[a, b, c, d]) => {
            let v = f32::from_ne_bytes([a, b, c, d]);
            // F32 → S16 con saturación: clamp(−1, 1) · 32767 (NaN → 0).
            if v.is_nan() {
                0.0
            } else {
                v.clamp(-1.0, 1.0) * 32767.0
            }
        }
        _ => 0.0,
    }
}

/// Redondeo con saturación a S16.
fn to_i16(v: f32) -> i16 {
    v.round().clamp(-32768.0, 32767.0) as i16
}

#[cfg(test)]
mod tests;
