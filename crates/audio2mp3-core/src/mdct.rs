//! MDCT con conmutación de bloques (36 → 18 largos, 3 × 12 → 6 cortos) y
//! reducción de aliasing entre subbandas (§2.4.3.4.10, Anexo C.1.5.3).

use core::f32::consts::PI;

use crate::tables::ALIAS_CI;

/// Muestras por subbanda en un granule.
pub(crate) const SUBBAND_LEN: usize = 18;
/// Líneas espectrales por granule.
pub(crate) const GRANULE_LINES: usize = 576;
/// Líneas por ventana corta en una subbanda.
const SHORT_LEN: usize = 6;

/// `block_type` del side info.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BlockType {
    #[default]
    Normal,
    /// Transición de bloques largos a cortos.
    Start,
    /// Tres ventanas cortas de 12 muestras.
    Short,
    /// Transición de bloques cortos a largos.
    Stop,
}

impl BlockType {
    pub fn code(self) -> u32 {
        match self {
            BlockType::Normal => 0,
            BlockType::Start => 1,
            BlockType::Short => 2,
            BlockType::Stop => 3,
        }
    }

    /// Tipo de un granule según si él y sus vecinos necesitan bloques cortos.
    /// Un bloque largo entre dos cortos no existe: se hace corto.
    pub fn choose(previous: BlockType, short_now: bool, short_next: bool) -> BlockType {
        let after_short = previous == BlockType::Short;
        match (short_now, after_short, short_next) {
            (true, _, _) | (false, true, true) => BlockType::Short,
            (false, true, false) => BlockType::Stop,
            (false, false, true) => BlockType::Start,
            (false, false, false) => BlockType::Normal,
        }
    }
}

/// Ventana de bloque largo de tipo `block` en la posición `n` (0..36).
fn long_window(block: BlockType, n: usize) -> f32 {
    let long = (PI / 36.0 * (n as f32 + 0.5)).sin();
    let short = |i: usize| (PI / 12.0 * (i as f32 + 0.5)).sin();
    match (block, n) {
        (BlockType::Start, 18..=23) | (BlockType::Stop, 12..=17) => 1.0,
        (BlockType::Start, 24..=29) => short(n - 18),
        (BlockType::Start, 30..) | (BlockType::Stop, 0..=5) => 0.0,
        (BlockType::Stop, 6..=11) => short(n - 6),
        _ => long,
    }
}

/// MDCT de las ventanas larga, start, stop y corta, con la ventana y la
/// escala incorporadas: 2/18 en bloques largos y 2/6 en los cortos, para que
/// con la IMDCT del decoder (sin escala) y el solapamiento la ganancia sea 1.
pub(crate) struct Mdct {
    /// Índice: 0 normal, 1 start, 2 stop.
    long: [[[f32; 2 * SUBBAND_LEN]; SUBBAND_LEN]; 3],
    short: [[f32; 2 * SHORT_LEN]; SHORT_LEN],
}

impl Mdct {
    pub fn new() -> Self {
        let mut long = [[[0.0; 2 * SUBBAND_LEN]; SUBBAND_LEN]; 3];
        let scale = 2.0 / SUBBAND_LEN as f32;
        for (table, block) in
            long.iter_mut()
                .zip([BlockType::Normal, BlockType::Start, BlockType::Stop])
        {
            for (k, row) in table.iter_mut().enumerate() {
                for (n, c) in row.iter_mut().enumerate() {
                    let phase = PI / 72.0 * (2 * n + 1 + 18) as f32 * (2 * k + 1) as f32;
                    *c = scale * long_window(block, n) * phase.cos();
                }
            }
        }
        let mut short = [[0.0; 2 * SHORT_LEN]; SHORT_LEN];
        let scale = 2.0 / SHORT_LEN as f32;
        for (m, row) in short.iter_mut().enumerate() {
            for (p, c) in row.iter_mut().enumerate() {
                let window = (PI / 12.0 * (p as f32 + 0.5)).sin();
                let phase = PI / 24.0 * (2 * p + 1 + 6) as f32 * (2 * m + 1) as f32;
                *c = scale * window * phase.cos();
            }
        }
        Self { long, short }
    }

    /// `input`: 18 muestras del granule anterior seguidas de las 18 actuales.
    /// En bloques cortos `out[3·m + w]` es la línea m de la ventana w, que
    /// cubre `input[6 + 6w .. 18 + 6w]` (el orden que espera el decoder).
    pub fn forward(
        &self,
        block: BlockType,
        input: &[f32; 2 * SUBBAND_LEN],
        out: &mut [f32; SUBBAND_LEN],
    ) {
        let table = match block {
            BlockType::Short => {
                for (w, segment) in input
                    .windows(2 * SHORT_LEN)
                    .skip(SHORT_LEN)
                    .step_by(SHORT_LEN)
                    .take(3)
                    .enumerate()
                {
                    for (m, row) in self.short.iter().enumerate() {
                        if let Some(o) = out.get_mut(3 * m + w) {
                            *o = row.iter().zip(segment).map(|(c, x)| c * x).sum();
                        }
                    }
                }
                return;
            }
            BlockType::Normal => &self.long[0],
            BlockType::Start => &self.long[1],
            BlockType::Stop => &self.long[2],
        };
        for (o, row) in out.iter_mut().zip(table) {
            *o = row.iter().zip(input).map(|(c, x)| c * x).sum();
        }
    }
}

/// Butterflies de reducción de aliasing del encoder: inversa de la que
/// aplica el decoder (rotación con el ángulo opuesto). No se aplica a
/// bloques cortos.
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
    use core::f64::consts::PI as PI64;

    /// IMDCT del decoder (§2.4.3.4.10.2), sin escala, con la ventana del tipo.
    fn decoder_imdct(block: BlockType, x: &[f32; SUBBAND_LEN]) -> [f32; 2 * SUBBAND_LEN] {
        let mut out = [0.0f32; 2 * SUBBAND_LEN];
        if block == BlockType::Short {
            for w in 0..3 {
                for p in 0..12 {
                    let sum: f64 = (0..6)
                        .map(|m| {
                            let phase = PI64 / 24.0 * (2 * p + 1 + 6) as f64 * (2 * m + 1) as f64;
                            f64::from(x[w + 3 * m]) * phase.cos()
                        })
                        .sum();
                    let window = (PI64 / 12.0 * (p as f64 + 0.5)).sin();
                    out[6 * w + 6 + p] += (sum * window) as f32;
                }
            }
            return out;
        }
        for (i, o) in out.iter_mut().enumerate() {
            let sum: f64 = x
                .iter()
                .enumerate()
                .map(|(k, &v)| {
                    let phase = PI64 / 72.0 * (2 * i + 1 + 18) as f64 * (2 * k + 1) as f64;
                    f64::from(v) * phase.cos()
                })
                .sum();
            *o = (sum * f64::from(long_window(block, i))) as f32;
        }
        out
    }

    fn roundtrip(blocks: &[BlockType]) {
        let mdct = Mdct::new();
        let signal: Vec<f32> = (0..18 * blocks.len())
            .map(|n| ((n * 7919 % 211) as f32 / 105.0) - 1.0)
            .collect();
        let mut prev = [0.0f32; SUBBAND_LEN];
        let mut overlap = [0.0f32; SUBBAND_LEN];
        let mut output = Vec::new();
        for (block, &bt) in signal.as_chunks::<SUBBAND_LEN>().0.iter().zip(blocks) {
            let mut input = [0.0; 2 * SUBBAND_LEN];
            input[..SUBBAND_LEN].copy_from_slice(&prev);
            input[SUBBAND_LEN..].copy_from_slice(block);
            prev.copy_from_slice(block);
            let mut spec = [0.0; SUBBAND_LEN];
            mdct.forward(bt, &input, &mut spec);
            let y = decoder_imdct(bt, &spec);
            for i in 0..SUBBAND_LEN {
                output.push(overlap[i] + y[i]);
                overlap[i] = y[SUBBAND_LEN + i];
            }
        }
        // La salida va retrasada un bloque respecto a la entrada.
        for (n, (out, inp)) in output[SUBBAND_LEN..].iter().zip(&signal).enumerate() {
            assert!(
                (out - inp).abs() < 1e-4,
                "{blocks:?}, muestra {n}: {out} != {inp}"
            );
        }
    }

    #[test]
    fn long_blocks_reconstruct() {
        roundtrip(&[BlockType::Normal; 12]);
    }

    #[test]
    fn block_switching_reconstructs() {
        use BlockType::{Normal as N, Short as S, Start as A, Stop as P};
        roundtrip(&[N, N, A, S, S, P, N, A, S, P, A, S, P, N]);
    }

    #[test]
    fn choose_never_puts_long_between_short_blocks() {
        use BlockType::{Normal as N, Short as S, Start as A, Stop as P};
        assert_eq!(BlockType::choose(N, false, false), N);
        assert_eq!(BlockType::choose(N, false, true), A);
        assert_eq!(BlockType::choose(A, true, false), S);
        assert_eq!(BlockType::choose(S, false, false), P);
        assert_eq!(BlockType::choose(S, false, true), S);
        assert_eq!(BlockType::choose(P, false, true), A);
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
