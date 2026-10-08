//! MDCT de bloques largos (36 → 18) y reducción de aliasing entre subbandas.

use core::f32::consts::PI;

use crate::tables::ALIAS_CI;

/// Muestras por subbanda en un granule.
pub(crate) const SUBBAND_LEN: usize = 18;
/// Líneas espectrales por granule.
pub(crate) const GRANULE_LINES: usize = 576;

/// MDCT con la ventana de bloque largo (`block_type = 0`) incorporada.
///
/// Escalada por 2/18 para que, con la IMDCT del decoder (sin escala) y el
/// solapamiento, el par MDCT/IMDCT tenga ganancia unidad.
pub(crate) struct Mdct {
    coef: [[f32; 2 * SUBBAND_LEN]; SUBBAND_LEN],
}

impl Mdct {
    pub fn new() -> Self {
        let mut coef = [[0.0; 2 * SUBBAND_LEN]; SUBBAND_LEN];
        let scale = 2.0 / SUBBAND_LEN as f32;
        for (k, row) in coef.iter_mut().enumerate() {
            for (n, c) in row.iter_mut().enumerate() {
                let window = (PI / 36.0 * (n as f32 + 0.5)).sin();
                let phase = PI / 72.0 * (2 * n + 1 + 18) as f32 * (2 * k + 1) as f32;
                *c = scale * window * phase.cos();
            }
        }
        Self { coef }
    }

    /// `input`: 18 muestras del granule anterior seguidas de las 18 actuales.
    pub fn forward(&self, input: &[f32; 2 * SUBBAND_LEN], out: &mut [f32; SUBBAND_LEN]) {
        for (o, row) in out.iter_mut().zip(&self.coef) {
            *o = row.iter().zip(input).map(|(c, x)| c * x).sum();
        }
    }
}

/// Butterflies de reducción de aliasing del encoder: inversa de la que
/// aplica el decoder (rotación con el ángulo opuesto).
pub(crate) struct AliasReduction {
    cs: [f32; 8],
    ca: [f32; 8],
}

impl AliasReduction {
    pub fn new() -> Self {
        let mut cs = [0.0; 8];
        let mut ca = [0.0; 8];
        for ((s, a), ci) in cs.iter_mut().zip(ca.iter_mut()).zip(ALIAS_CI) {
            let norm = (1.0 + ci * ci).sqrt();
            *s = 1.0 / norm;
            *a = ci / norm;
        }
        Self { cs, ca }
    }

    pub fn apply(&self, xr: &mut [f32; GRANULE_LINES]) {
        for sb in 1..32 {
            let edge = sb * SUBBAND_LEN;
            for (i, (&cs, &ca)) in self.cs.iter().zip(&self.ca).enumerate() {
                let lo = edge - 1 - i;
                let up = edge + i;
                let bu = xr[lo];
                let bd = xr[up];
                xr[lo] = bu * cs + bd * ca;
                xr[up] = bd * cs - bu * ca;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// IMDCT del decoder (§2.4.3.4.10.2), sin escala, con ventana de bloque largo.
    fn decoder_imdct(x: &[f32; SUBBAND_LEN]) -> [f32; 2 * SUBBAND_LEN] {
        let mut out = [0.0; 2 * SUBBAND_LEN];
        for (i, o) in out.iter_mut().enumerate() {
            let sum: f64 = x
                .iter()
                .enumerate()
                .map(|(k, &v)| {
                    let phase =
                        core::f64::consts::PI / 72.0 * (2 * i + 1 + 18) as f64 * (2 * k + 1) as f64;
                    f64::from(v) * phase.cos()
                })
                .sum();
            let window = (core::f64::consts::PI / 36.0 * (i as f64 + 0.5)).sin();
            *o = (sum * window) as f32;
        }
        out
    }

    #[test]
    fn mdct_imdct_overlap_add_reconstructs() {
        let mdct = Mdct::new();
        let signal: Vec<f32> = (0..18 * 12)
            .map(|n| ((n * 7919 % 211) as f32 / 105.0) - 1.0)
            .collect();
        let mut prev = [0.0f32; SUBBAND_LEN];
        let mut overlap = [0.0f32; SUBBAND_LEN];
        let mut output = Vec::new();
        for block in signal.as_chunks::<SUBBAND_LEN>().0 {
            let mut input = [0.0; 2 * SUBBAND_LEN];
            input[..SUBBAND_LEN].copy_from_slice(&prev);
            input[SUBBAND_LEN..].copy_from_slice(block);
            prev.copy_from_slice(block);
            let mut spec = [0.0; SUBBAND_LEN];
            mdct.forward(&input, &mut spec);
            let y = decoder_imdct(&spec);
            for i in 0..SUBBAND_LEN {
                output.push(overlap[i] + y[i]);
                overlap[i] = y[SUBBAND_LEN + i];
            }
        }
        // La salida va retrasada un bloque respecto a la entrada.
        for (out, inp) in output[SUBBAND_LEN..].iter().zip(&signal) {
            assert!((out - inp).abs() < 1e-4, "{out} != {inp}");
        }
    }

    #[test]
    fn alias_reduction_is_inverted_by_decoder_butterflies() {
        let alias = AliasReduction::new();
        let original: [f32; GRANULE_LINES] =
            core::array::from_fn(|i| ((i * 31 % 97) as f32) - 48.0);
        let mut xr = original;
        alias.apply(&mut xr);
        assert_ne!(xr, original);
        // Butterflies del decoder (§2.4.3.4.10.1).
        for sb in 1..32 {
            for i in 0..8 {
                let lo = sb * 18 - 1 - i;
                let up = sb * 18 + i;
                let (bu, bd) = (xr[lo], xr[up]);
                xr[lo] = bu * alias.cs[i] - bd * alias.ca[i];
                xr[up] = bd * alias.cs[i] + bu * alias.ca[i];
            }
        }
        for (a, b) in xr.iter().zip(&original) {
            assert!((a - b).abs() < 1e-3);
        }
    }
}
