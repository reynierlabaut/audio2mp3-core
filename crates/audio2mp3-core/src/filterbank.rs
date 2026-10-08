//! Filterbank polifásico de análisis de 32 subbandas (§2.4.3.1, Anexo C.1.3).

use core::f32::consts::PI;

pub(crate) const SUBBANDS: usize = 32;
const FIFO_LEN: usize = 512;

/// Ventana de análisis C[0..511] del Anexo C.
pub(crate) type AnalysisWindow = [f32; FIFO_LEN];

/// Estado de análisis de un canal.
pub(crate) struct Filterbank {
    window: &'static AnalysisWindow,
    /// X[0] es la muestra más reciente.
    fifo: [f32; FIFO_LEN],
    /// M[i][k] = cos((2i + 1)(k − 16)π / 64).
    matrix: [[f32; 64]; SUBBANDS],
}

impl Filterbank {
    pub fn new(window: &'static AnalysisWindow) -> Self {
        let mut matrix = [[0.0; 64]; SUBBANDS];
        for (i, row) in matrix.iter_mut().enumerate() {
            for (k, m) in row.iter_mut().enumerate() {
                *m = ((2 * i + 1) as f32 * (k as f32 - 16.0) * PI / 64.0).cos();
            }
        }
        Self {
            window,
            fifo: [0.0; FIFO_LEN],
            matrix,
        }
    }

    /// Consume 32 muestras nuevas (normalizadas a [-1, 1), en orden temporal)
    /// y produce una muestra por subbanda.
    pub fn analyze(&mut self, input: &[f32; SUBBANDS], out: &mut [f32; SUBBANDS]) {
        self.fifo.copy_within(0..FIFO_LEN - SUBBANDS, SUBBANDS);
        // X[31] recibe la más antigua de las 32 nuevas y X[0] la más reciente.
        for (x, &sample) in self.fifo[..SUBBANDS].iter_mut().rev().zip(input) {
            *x = sample;
        }
        let mut y = [0.0f32; 64];
        // Y[i] = Σ_j C[i + 64j] · X[i + 64j]
        for (xs, cs) in self
            .fifo
            .as_chunks::<64>()
            .0
            .iter()
            .zip(self.window.as_chunks::<64>().0)
        {
            for ((acc, &x), &c) in y.iter_mut().zip(xs).zip(cs) {
                *acc += x * c;
            }
        }
        for (s, row) in out.iter_mut().zip(&self.matrix) {
            *s = row.iter().zip(&y).map(|(m, v)| m * v).sum();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ventana sintética: el test compara contra la definición del estándar,
    /// no depende de los valores de la tabla C.1.
    static TEST_WINDOW: AnalysisWindow = {
        let mut w = [0.0f32; FIFO_LEN];
        let mut i = 0;
        while i < FIFO_LEN {
            w[i] = ((i * 37 % 101) as f32 - 50.0) / 5000.0;
            i += 1;
        }
        w
    };

    /// Implementación literal y lenta del estándar, en f64.
    struct Reference {
        x: [f64; FIFO_LEN],
    }

    impl Reference {
        fn analyze(&mut self, input: &[f32; SUBBANDS]) -> [f64; SUBBANDS] {
            for i in (SUBBANDS..FIFO_LEN).rev() {
                self.x[i] = self.x[i - SUBBANDS];
            }
            for (n, &sample) in input.iter().enumerate() {
                self.x[31 - n] = f64::from(sample);
            }
            let z: Vec<f64> = (0..FIFO_LEN)
                .map(|i| f64::from(TEST_WINDOW[i]) * self.x[i])
                .collect();
            let y: Vec<f64> = (0..64)
                .map(|i| (0..8).map(|j| z[i + 64 * j]).sum())
                .collect();
            core::array::from_fn(|i| {
                (0..64)
                    .map(|k| {
                        let m = ((2 * i + 1) as f64 * (k as f64 - 16.0) * core::f64::consts::PI
                            / 64.0)
                            .cos();
                        m * y[k]
                    })
                    .sum()
            })
        }
    }

    #[test]
    fn matches_reference_definition() {
        let mut fb = Filterbank::new(&TEST_WINDOW);
        let mut reference = Reference { x: [0.0; FIFO_LEN] };
        for block in 0..40 {
            let input: [f32; SUBBANDS] =
                core::array::from_fn(|n| (((block * 32 + n) as f32) * 0.37).sin() * 0.9);
            let mut out = [0.0; SUBBANDS];
            fb.analyze(&input, &mut out);
            let expected = reference.analyze(&input);
            for (a, b) in out.iter().zip(&expected) {
                assert!(
                    (f64::from(*a) - b).abs() < 1e-4,
                    "bloque {block}: {a} != {b}"
                );
            }
        }
    }
}
