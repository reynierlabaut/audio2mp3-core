//! Downmix a estéreo de más de 2 canales (orden estándar WAVE / Android).
//!
//! L' = L + 0.7071·C + 0.7071·Ls, R' = R + 0.7071·C + 0.7071·Rs; LFE se
//! descarta. Los pesos se normalizan por la mayor suma de una salida, así
//! que el resultado nunca satura.

use core::f32::consts::FRAC_1_SQRT_2 as C;

const MAX: usize = super::MAX_INPUT_CHANNELS as usize;

pub(super) struct Downmix {
    in_channels: usize,
    /// `None`: 1 o 2 canales, se copian sin cambios.
    weights: Option<[[f32; MAX]; 2]>,
}

/// (peso izquierdo, peso derecho) de cada canal según el número de canales.
fn layout(channels: usize) -> &'static [(f32, f32)] {
    const L: (f32, f32) = (1.0, 0.0);
    const R: (f32, f32) = (0.0, 1.0);
    const CENTER: (f32, f32) = (C, C);
    const LFE: (f32, f32) = (0.0, 0.0);
    const LS: (f32, f32) = (C, 0.0);
    const RS: (f32, f32) = (0.0, C);
    const BC: (f32, f32) = (0.5, 0.5);
    match channels {
        3 => &[L, R, CENTER],
        4 => &[L, R, LS, RS],
        5 => &[L, R, CENTER, LS, RS],
        6 => &[L, R, CENTER, LFE, LS, RS],
        7 => &[L, R, CENTER, LFE, BC, LS, RS],
        _ => &[L, R, CENTER, LFE, LS, RS, LS, RS],
    }
}

impl Downmix {
    pub fn new(in_channels: usize) -> Self {
        if in_channels <= 2 {
            return Self {
                in_channels,
                weights: None,
            };
        }
        let pairs = layout(in_channels);
        let sum_l: f32 = pairs.iter().map(|p| p.0).sum();
        let sum_r: f32 = pairs.iter().map(|p| p.1).sum();
        let norm = 1.0 / sum_l.max(sum_r).max(1.0);
        let mut weights = [[0.0; MAX]; 2];
        for (i, &(l, r)) in pairs.iter().enumerate().take(MAX) {
            weights[0][i] = l * norm;
            weights[1][i] = r * norm;
        }
        Self {
            in_channels,
            weights: Some(weights),
        }
    }

    pub fn out_channels(&self) -> usize {
        self.in_channels.min(2)
    }

    /// Escribe la trama mezclada en `out` y devuelve sus canales.
    pub fn apply(&self, input: &[f32], out: &mut [f32; 2]) -> usize {
        match &self.weights {
            None => {
                for (o, &s) in out.iter_mut().zip(input) {
                    *o = s;
                }
            }
            Some(weights) => {
                for (o, w) in out.iter_mut().zip(weights) {
                    *o = input.iter().zip(w).map(|(s, w)| s * w).sum();
                }
            }
        }
        self.out_channels()
    }
}
