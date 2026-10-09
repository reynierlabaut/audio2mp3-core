//! Modelo psicoacústico en el dominio MDCT: ruido permitido por banda de
//! scalefactors para el outer loop.
//!
//! Por banda: energía y tonalidad (planitud espectral), propagación en la
//! escala Bark con la función de Schroeder, margen de enmascaramiento según
//! la tonalidad (≈ 5.5 dB para ruido, 14.5 + z dB para tonos) y, como
//! mínimo, el umbral absoluto de audición (fórmula de Terhardt).
//! Referencias: Schroeder, Atal, Hall (1979); Terhardt (1979); Johnston (1988).

use crate::mdct::GRANULE_LINES;
use crate::tables::sfb::{LongBands, ShortBands};

/// Rangos como máximo: 13 bandas cortas × 3 ventanas.
pub(crate) const MAX_RANGES: usize = 39;

/// Energía MDCT de una sinusoide a fondo de escala (amplitud 1.0): en una
/// banda larga y en una banda de una ventana corta. Medida en
/// `encoder::tests::full_scale_sine_energy`.
pub(crate) const FULL_SCALE_ENERGY_LONG: f32 = 0.78;
pub(crate) const FULL_SCALE_ENERGY_SHORT: f32 = 0.53;
/// Nivel de presión sonora que se asigna al fondo de escala.
const FULL_SCALE_SPL: f32 = 96.0;
/// Por encima, la curva de Terhardt crece tanto que eliminaría los agudos;
/// se congela en este valor.
const ATH_MAX_FREQ: f32 = 15_000.0;

/// Bandas de scalefactors de un tipo de bloque, en el orden del bitstream.
pub(crate) struct BandLayout {
    /// Líneas `[start, end)` de cada rango.
    pub ranges: [(u16, u16); MAX_RANGES],
    pub count: usize,
    /// Los rangos `0..sf_count` tienen scalefactor (índice = posición).
    pub sf_count: usize,
    /// Ventana (bloques cortos) o 0.
    window: [u8; MAX_RANGES],
    bark: [f32; MAX_RANGES],
    /// Energía del umbral absoluto en la banda.
    ath: [f32; MAX_RANGES],
    /// `spread[j][i]`: peso de la banda i al enmascarar la j (misma ventana),
    /// normalizado para que la suma de cada fila sea 1. Solo depende de las
    /// bandas, así que se calcula una vez.
    spread: [[f32; MAX_RANGES]; MAX_RANGES],
}

fn bark(freq: f32) -> f32 {
    13.0 * (0.000_76 * freq).atan() + 3.5 * (freq / 7_500.0).powi(2).atan()
}

/// Umbral absoluto de audición en dB SPL (Terhardt).
fn ath_db(freq: f32) -> f32 {
    let khz = freq.clamp(20.0, ATH_MAX_FREQ) / 1_000.0;
    3.64 * khz.powf(-0.8) - 6.5 * (-0.6 * (khz - 3.3).powi(2)).exp() + 1e-3 * khz.powi(4)
}

impl BandLayout {
    /// Bloques largos: 22 bandas, las 21 primeras con scalefactor.
    pub fn long(bands: &LongBands, sample_rate: u32) -> Self {
        let mut layout = Self::empty();
        let line_hz = sample_rate as f32 / 2.0 / GRANULE_LINES as f32;
        for (i, band) in bands.windows(2).enumerate().take(MAX_RANGES) {
            let &[start, end] = band else { continue };
            layout.set(
                i,
                (start, end),
                0,
                start,
                end,
                line_hz,
                FULL_SCALE_ENERGY_LONG,
            );
            layout.count = i + 1;
        }
        layout.sf_count = 21;
        layout.compute_spread();
        layout
    }

    /// Bloques cortos: banda s, ventana w → rango `3·s + w`; con
    /// scalefactor las bandas 0..12.
    pub fn short(bands: &ShortBands, sample_rate: u32) -> Self {
        let mut layout = Self::empty();
        let line_hz = sample_rate as f32 / 2.0 / 192.0;
        for (s, band) in bands.windows(2).enumerate() {
            let &[start, end] = band else { continue };
            let width = end - start;
            for w in 0..3u16 {
                let i = 3 * s + usize::from(w);
                if i >= MAX_RANGES {
                    continue;
                }
                let first = 3 * start + w * width;
                layout.set(
                    i,
                    (first, first + width),
                    w as u8,
                    start,
                    end,
                    line_hz,
                    FULL_SCALE_ENERGY_SHORT,
                );
                layout.count = i + 1;
            }
        }
        layout.sf_count = 36;
        layout.compute_spread();
        layout
    }

    fn empty() -> Self {
        Self {
            ranges: [(0, 0); MAX_RANGES],
            count: 0,
            sf_count: 0,
            window: [0; MAX_RANGES],
            bark: [0.0; MAX_RANGES],
            ath: [0.0; MAX_RANGES],
            spread: [[0.0; MAX_RANGES]; MAX_RANGES],
        }
    }

    /// `freq_start..freq_end`: líneas de frecuencia de la banda (por ventana).
    #[allow(clippy::too_many_arguments)]
    fn set(
        &mut self,
        i: usize,
        range: (u16, u16),
        window: u8,
        freq_start: u16,
        freq_end: u16,
        line_hz: f32,
        full_scale: f32,
    ) {
        let center = (f32::from(freq_start) + f32::from(freq_end)) / 2.0 * line_hz;
        // Frecuencia más sensible de la banda (mínimo de la curva).
        let ath = (freq_start..freq_end)
            .map(|l| ath_db((f32::from(l) + 0.5) * line_hz))
            .fold(f32::INFINITY, f32::min);
        if let (Some(r), Some(w), Some(b), Some(a)) = (
            self.ranges.get_mut(i),
            self.window.get_mut(i),
            self.bark.get_mut(i),
            self.ath.get_mut(i),
        ) {
            *r = range;
            *w = window;
            *b = bark(center);
            *a = full_scale * 10f32.powf((ath - FULL_SCALE_SPL) / 10.0);
        }
    }

    fn compute_spread(&mut self) {
        for (j, row) in self.spread.iter_mut().enumerate().take(self.count) {
            let (Some(&zj), Some(&wj)) = (self.bark.get(j), self.window.get(j)) else {
                continue;
            };
            for ((weight, &zi), &wi) in row
                .iter_mut()
                .zip(&self.bark)
                .zip(&self.window)
                .take(self.count)
            {
                *weight = if wi == wj { spreading(zj - zi) } else { 0.0 };
            }
            let norm: f32 = row.iter().sum();
            for weight in row.iter_mut() {
                *weight /= norm.max(1e-9);
            }
        }
    }
}

/// Atenuación de la función de propagación de Schroeder para una distancia
/// `dz` en Bark (enmascarado − enmascarador), en escala lineal de potencia.
fn spreading(dz: f32) -> f32 {
    let x = dz + 0.474;
    let db = 15.81 + 7.5 * x - 17.5 * (1.0 + x * x).sqrt();
    10f32.powf(db / 10.0)
}

/// Ruido permitido (energía de distorsión) en cada rango de `layout` para
/// el espectro `xr`.
pub(crate) fn allowed_distortion(
    layout: &BandLayout,
    xr: &[f32; GRANULE_LINES],
    out: &mut [f32; MAX_RANGES],
) {
    let mut energy = [0.0f32; MAX_RANGES];
    let mut tonality = [0.0f32; MAX_RANGES];
    for ((e, t), &(start, end)) in energy
        .iter_mut()
        .zip(tonality.iter_mut())
        .zip(&layout.ranges)
        .take(layout.count)
    {
        let Some(lines) = xr.get(usize::from(start)..usize::from(end)) else {
            continue;
        };
        let n = lines.len().max(1) as f32;
        let (mut sum, mut log_sum) = (0.0f32, 0.0f32);
        for &x in lines {
            let p = x * x + 1e-20;
            sum += p;
            log_sum += p.ln();
        }
        *e = sum;
        // Planitud espectral: 0 dB ruido plano, muy negativa para tonos.
        let flatness_db = 10.0 * ((log_sum / n).exp() / (sum / n)).log10();
        *t = (flatness_db / -60.0).clamp(0.0, 1.0);
    }
    let bands = out
        .iter_mut()
        .zip(&layout.spread)
        .zip(&layout.bark)
        .zip(&tonality)
        .zip(&layout.ath);
    for ((((xmin, weights), &z), &alpha), &ath) in bands.take(layout.count) {
        let spread: f32 = weights.iter().zip(&energy).map(|(w, e)| w * e).sum();
        let offset_db = alpha * (14.5 + z) + (1.0 - alpha) * 5.5;
        let masking = spread * 10f32.powf(-offset_db / 10.0);
        *xmin = masking.max(ath);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::sfb::{long_bands, short_bands};

    #[test]
    fn layouts_cover_the_granule() {
        for rate in [32_000, 44_100, 48_000] {
            let long = BandLayout::long(long_bands(rate).unwrap(), rate);
            assert_eq!((long.count, long.sf_count), (22, 21));
            assert_eq!(long.ranges[21].1, 576);
            let short = BandLayout::short(short_bands(rate).unwrap(), rate);
            assert_eq!((short.count, short.sf_count), (39, 36));
            let mut covered = [false; 576];
            for &(a, b) in &short.ranges[..short.count] {
                for c in &mut covered[usize::from(a)..usize::from(b)] {
                    assert!(!*c);
                    *c = true;
                }
            }
            assert!(covered.iter().all(|&c| c));
        }
    }

    #[test]
    fn spreading_is_unity_at_zero_and_decays() {
        assert!((spreading(0.0) - 1.0).abs() < 0.01);
        assert!(spreading(3.0) < 0.01 && spreading(-3.0) < spreading(0.0));
        // Enmascara más hacia arriba que hacia abajo en frecuencia.
        assert!(spreading(1.0) > spreading(-1.0));
    }

    #[test]
    fn ath_is_lowest_around_3_khz() {
        assert!(ath_db(3_300.0) < ath_db(1_000.0));
        assert!(ath_db(3_300.0) < ath_db(10_000.0));
        assert!(ath_db(100.0) > 20.0);
    }

    #[test]
    fn tones_mask_less_than_noise() {
        let layout = BandLayout::long(long_bands(44_100).unwrap(), 44_100);
        let mut tone = [0.0f32; 576];
        tone[100] = 1.0;
        let noise: [f32; 576] = core::array::from_fn(|i| {
            if (90..110).contains(&i) {
                ((i * 7919 % 13) as f32 - 6.0) / 25.0
            } else {
                0.0
            }
        });
        let (mut t, mut n) = ([0.0; MAX_RANGES], [0.0; MAX_RANGES]);
        allowed_distortion(&layout, &tone, &mut t);
        allowed_distortion(&layout, &noise, &mut n);
        let band = layout
            .ranges
            .iter()
            .position(|&(a, b)| (a..b).contains(&100))
            .unwrap();
        let ratio = |x: &[f32], e: f32| 10.0 * (x[band] / e).log10();
        let e_tone = 1.0;
        let e_noise: f32 = (layout.ranges[band].0..layout.ranges[band].1)
            .map(|i| noise[usize::from(i)].powi(2))
            .sum();
        assert!(ratio(&t, e_tone) < ratio(&n, e_noise) - 10.0);
    }
}
