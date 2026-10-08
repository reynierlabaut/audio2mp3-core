//! Resampler polifásico con sinc enventanado (Blackman), en streaming.
//!
//! La salida n está en el instante exacto n·M/L de la entrada (L/M = tasa de
//! salida/entrada reducida), sin retardo añadido: el filtro es simétrico y
//! la historia empieza con ceros. El kernel se tabula en `new()` con
//! `PHASES` puntos por muestra de entrada y se interpola linealmente, así que
//! cualquier relación de tasas usa memoria acotada.
//!
//! Respuesta medida (tests de `convert`), relativa al Nyquist de la tasa más
//! baja: plana (SNR > 60 dB) hasta ~0.7; transición hasta ~1.0; atenuación
//! > 60 dB por encima. Ensanchar a 48 taps llevaría la banda plana a ~0.8.

use crate::error::Error;

/// Semiancho del filtro, en muestras de la tasa más baja (32 taps).
const HALF_TAPS: f64 = 16.0;
/// Corte respecto al Nyquist de la tasa más baja.
const CUTOFF: f64 = 0.9;
/// Puntos de la tabla del kernel por muestra de entrada.
const PHASES: usize = 128;

pub(super) struct Resampler {
    channels: usize,
    /// Misma tasa: se copian las muestras sin filtrar.
    passthrough: bool,
    l: u64,
    m: u64,
    /// Instante de la próxima salida en muestras de entrada: `ip + r / l`.
    ip: i64,
    r: u64,
    /// Semiancho del filtro en muestras de entrada (≥ W + 1).
    half: usize,
    width: f32,
    kernel: Vec<f32>,
    /// Historia intercalada; `hist[0]` es la trama de índice `base`.
    hist: Vec<f32>,
    base: i64,
    len: usize,
    cap: usize,
    taps: Vec<f32>,
    consumed: u64,
    produced: u64,
}

/// Kernel en |t| (muestras de entrada), interpolando la tabla.
fn kernel_at(kernel: &[f32], width: f32, t: f32) -> f32 {
    let t = t.abs();
    if t >= width {
        return 0.0;
    }
    let x = t * PHASES as f32;
    let i = x as usize;
    let f = x - i as f32;
    match (kernel.get(i), kernel.get(i + 1)) {
        (Some(&a), Some(&b)) => a + (b - a) * f,
        _ => 0.0,
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32, channels: usize) -> Self {
        let g = gcd(u64::from(in_rate), u64::from(out_rate)).max(1);
        let (l, m) = (u64::from(out_rate) / g, u64::from(in_rate) / g);
        let passthrough = in_rate == out_rate;
        // Al bajar de tasa el filtro se estira (antialiasing al Nyquist de salida).
        let scale = (f64::from(out_rate) / f64::from(in_rate)).min(1.0);
        let width = HALF_TAPS / scale;
        let half = width.ceil() as usize + 1;
        let fc = CUTOFF * scale;
        let points = half * PHASES + 2;
        let kernel = if passthrough {
            Vec::new()
        } else {
            (0..points)
                .map(|i| {
                    let t = i as f64 / PHASES as f64;
                    if t >= width {
                        return 0.0;
                    }
                    let x = core::f64::consts::PI * fc * t;
                    let sinc = if x == 0.0 { 1.0 } else { x.sin() / x };
                    let a = core::f64::consts::PI * t / width;
                    let blackman = 0.42 + 0.5 * a.cos() + 0.08 * (2.0 * a).cos();
                    (fc * sinc * blackman) as f32
                })
                .collect()
        };
        let cap = 2 * half + 4;
        let mut hist = Vec::new();
        let mut taps = Vec::new();
        if !passthrough {
            hist = vec![0.0; cap * channels];
            taps = vec![0.0; 2 * half];
        }
        Self {
            channels,
            passthrough,
            l,
            m,
            ip: 0,
            r: 0,
            half,
            width: width as f32,
            kernel,
            hist,
            // Historia inicial: `half` tramas de silencio antes del inicio.
            base: -(half as i64),
            len: half,
            cap,
            taps,
            consumed: 0,
            produced: 0,
        }
    }

    pub fn max_output_frames(&self, frames: usize) -> usize {
        if self.passthrough {
            return frames;
        }
        let frames = frames as u64;
        let max = frames
            .saturating_mul(self.l)
            .div_ceil(self.m)
            .saturating_add(1);
        usize::try_from(max).unwrap_or(usize::MAX)
    }

    pub fn max_flush_frames(&self) -> usize {
        if self.passthrough {
            return 0;
        }
        let max = (self.half as u64 + 2)
            .saturating_mul(self.l)
            .div_ceil(self.m)
            + 2;
        usize::try_from(max).unwrap_or(usize::MAX)
    }

    /// Añade una trama y escribe las salidas que ya se pueden calcular.
    pub fn push(&mut self, frame: &[f32], out: &mut [i16]) -> Result<usize, Error> {
        if self.passthrough {
            let dst = out.get_mut(..frame.len()).ok_or(Error::Internal)?;
            for (o, &s) in dst.iter_mut().zip(frame) {
                *o = super::to_i16(s);
            }
            return Ok(frame.len());
        }
        self.append(frame)?;
        self.consumed += 1;
        self.emit(out, None)
    }

    /// Completa con silencio hasta producir ceil(entrada · L / M) salidas.
    pub fn flush(&mut self, out: &mut [i16]) -> Result<usize, Error> {
        if self.passthrough {
            return Ok(0);
        }
        let target = self.consumed.saturating_mul(self.l).div_ceil(self.m);
        let silence = [0.0f32; 2];
        let frame = silence.get(..self.channels).ok_or(Error::Internal)?;
        let mut written = 0;
        while self.produced < target {
            self.append(frame)?;
            written += self.emit(out.get_mut(written..).ok_or(Error::Internal)?, Some(target))?;
        }
        Ok(written)
    }

    fn append(&mut self, frame: &[f32]) -> Result<(), Error> {
        if self.len == self.cap {
            // Las tramas anteriores a ip − half + 1 ya no se usan.
            let keep_from = self.ip - self.half as i64 + 1;
            let drop = usize::try_from(keep_from - self.base)
                .unwrap_or(0)
                .min(self.len);
            if drop == 0 {
                return Err(Error::Internal);
            }
            let ch = self.channels;
            self.hist.copy_within(drop * ch..self.len * ch, 0);
            self.len -= drop;
            self.base += drop as i64;
        }
        let at = self.len * self.channels;
        self.hist
            .get_mut(at..at + self.channels)
            .ok_or(Error::Internal)?
            .copy_from_slice(frame);
        self.len += 1;
        Ok(())
    }

    fn emit(&mut self, out: &mut [i16], limit: Option<u64>) -> Result<usize, Error> {
        let mut written = 0;
        let ch = self.channels;
        let half = self.half as i64;
        // Se necesita la trama ip + half.
        while self.ip + half < self.base + self.len as i64
            && limit.is_none_or(|l| self.produced < l)
        {
            let frac = self.r as f32 / self.l as f32;
            // Taps k = ip − half + 1 ..= ip + half, con t = p − k = frac + (ip − k).
            let first = self.ip - half + 1;
            for (j, w) in self.taps.iter_mut().enumerate() {
                let t = frac + (half - 1 - j as i64) as f32;
                *w = kernel_at(&self.kernel, self.width, t);
            }
            let start = usize::try_from(first - self.base).map_err(|_| Error::Internal)?;
            let dst = out.get_mut(written..written + ch).ok_or(Error::Internal)?;
            for (c, o) in dst.iter_mut().enumerate() {
                let mut acc = 0.0f32;
                for (j, &w) in self.taps.iter().enumerate() {
                    acc += w * self
                        .hist
                        .get((start + j) * ch + c)
                        .copied()
                        .ok_or(Error::Internal)?;
                }
                *o = super::to_i16(acc);
            }
            written += ch;
            self.produced += 1;
            self.r += self.m;
            self.ip += (self.r / self.l) as i64;
            self.r %= self.l;
        }
        Ok(written)
    }
}
