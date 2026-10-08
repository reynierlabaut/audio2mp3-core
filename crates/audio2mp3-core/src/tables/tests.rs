//! VerificaciÃ³n de las tablas generadas, independiente de su origen.

use super::huffman::{PairTable, COUNT1_A, COUNT1_B, PAIR_TABLES};
use super::window::{ANALYSIS_WINDOW, D_Q16};
use crate::filterbank::{Filterbank, SUBBANDS};

// ------------------------------------------------------------------ Ventana

#[test]
fn window_symmetry_and_known_values() {
    assert_eq!(D_Q16[0], 0);
    // D[256] = 1.144989014, D[224] = âˆ’0.572036743, D[32] = âˆ’0.000442505.
    assert_eq!(D_Q16[256], 75038);
    assert_eq!(D_Q16[224], -37489);
    assert_eq!(D_Q16[32], -29);
    for i in 1..512 {
        let mirrored = if i % 64 == 0 { D_Q16[i] } else { -D_Q16[i] };
        assert_eq!(D_Q16[512 - i], mirrored, "D[{}]", 512 - i);
    }
    for (c, d) in ANALYSIS_WINDOW.iter().zip(D_Q16) {
        assert_eq!(*c, d as f32 / 32.0 / 65536.0);
    }
}

/// SÃ­ntesis polifÃ¡sica del decoder, tal como la define el estÃ¡ndar (Â§2.4.3.2).
struct Synthesis {
    v: [f64; 1024],
}

impl Synthesis {
    fn run(&mut self, s: &[f32; SUBBANDS]) -> [f64; SUBBANDS] {
        self.v.copy_within(0..1024 - 64, 64);
        for i in 0..64 {
            self.v[i] = s
                .iter()
                .enumerate()
                .map(|(k, &x)| {
                    let n =
                        ((16 + i) as f64 * (2 * k + 1) as f64 * core::f64::consts::PI / 64.0).cos();
                    n * f64::from(x)
                })
                .sum();
        }
        let mut out = [0f64; SUBBANDS];
        for (j, o) in out.iter_mut().enumerate() {
            for i in 0..8 {
                let d0 = f64::from(D_Q16[64 * i + j]) / 65536.0;
                let d1 = f64::from(D_Q16[64 * i + 32 + j]) / 65536.0;
                *o += d0 * self.v[128 * i + j] + d1 * self.v[128 * i + 96 + j];
            }
        }
        out
    }
}

/// AnÃ¡lisis (nuestro, con C = D/32) + sÃ­ntesis (decoder) debe reconstruir la
/// entrada con un retardo fijo. Un solo coeficiente mal rompe la reconstrucciÃ³n.
#[test]
fn analysis_and_standard_synthesis_reconstruct_input() {
    let blocks = 200;
    let input: Vec<f32> = (0..blocks * SUBBANDS)
        .map(|n| {
            let t = n as f32;
            0.4 * (t * 0.031).sin() + 0.3 * (t * 0.77).sin() + 0.2 * (t * 2.9).cos()
        })
        .collect();
    let mut fb = Filterbank::new(&ANALYSIS_WINDOW);
    let mut synth = Synthesis { v: [0.0; 1024] };
    let mut output = Vec::with_capacity(input.len());
    for block in input.as_chunks::<SUBBANDS>().0 {
        let mut sub = [0.0; SUBBANDS];
        fb.analyze(block, &mut sub);
        output.extend(synth.run(&sub));
    }
    // Retardo total del par anÃ¡lisis/sÃ­ntesis: 481 muestras.
    let delay = 481;
    let skip = 1024;
    let (mut signal, mut noise) = (0f64, 0f64);
    for (o, &x) in output[skip + delay..].iter().zip(&input[skip..]) {
        signal += f64::from(x).powi(2);
        noise += (o - f64::from(x)).powi(2);
    }
    let snr = 10.0 * (signal / noise).log10();
    assert!(snr > 80.0, "SNR de reconstrucciÃ³n = {snr:.1} dB");
}

// ------------------------------------------------------------------ Huffman

fn assert_complete_prefix_code(codes: &[(u32, u8)], name: &str) {
    let kraft: f64 = codes.iter().map(|&(_, l)| 0.5f64.powi(i32::from(l))).sum();
    assert!((kraft - 1.0).abs() < 1e-12, "{name}: Kraft = {kraft}");
    for (i, &(ca, la)) in codes.iter().enumerate() {
        assert!(
            la > 0 && u32::from(la) <= 32 && (la == 32 || ca >> la == 0),
            "{name}[{i}]"
        );
        for &(cb, lb) in &codes[i + 1..] {
            let l = la.min(lb);
            assert_ne!(
                ca >> (la - l),
                cb >> (lb - l),
                "{name}: un cÃ³digo es prefijo de otro"
            );
        }
    }
}

#[test]
fn pair_tables_are_complete_prefix_codes() {
    for (t, table) in PAIR_TABLES.iter().enumerate() {
        if let Some(PairTable {
            xlen, codes, lens, ..
        }) = table
        {
            let n = usize::from(*xlen).pow(2);
            assert_eq!((codes.len(), lens.len()), (n, n), "tabla {t}");
            let pairs: Vec<(u32, u8)> = codes.iter().copied().zip(lens.iter().copied()).collect();
            assert_complete_prefix_code(&pairs, &format!("tabla {t}"));
        }
    }
}

#[test]
fn pair_table_dimensions_and_linbits() {
    let xlen = [0, 2, 3, 3, 0, 4, 4, 6, 6, 6, 8, 8, 8, 16, 0, 16];
    let linbits = [1, 2, 3, 4, 6, 8, 10, 13, 4, 5, 6, 7, 8, 9, 11, 13];
    for (t, table) in PAIR_TABLES.iter().enumerate() {
        match (t, table) {
            (0 | 4 | 14, None) => {}
            (0..=15, Some(tb)) => {
                assert_eq!(usize::from(tb.xlen), xlen[t], "tabla {t}");
                assert_eq!(tb.linbits, 0, "tabla {t}");
            }
            (16..=31, Some(tb)) => {
                assert_eq!(tb.xlen, 16, "tabla {t}");
                assert_eq!(tb.linbits, linbits[t - 16], "tabla {t}");
            }
            _ => panic!("tabla {t} inesperada"),
        }
    }
}

#[test]
fn table_1_matches_standard() {
    let t1 = PAIR_TABLES[1].as_ref().unwrap();
    // (x, y): 00 â†’ "1", 01 â†’ "001", 10 â†’ "01", 11 â†’ "000"
    assert_eq!(t1.codes, &[0b1, 0b001, 0b01, 0b000]);
    assert_eq!(t1.lens, &[1, 3, 2, 3]);
}

#[test]
fn count1_tables_match_standard() {
    let a: Vec<(u32, u8)> = COUNT1_A.iter().map(|&(c, l)| (u32::from(c), l)).collect();
    assert_complete_prefix_code(&a, "count1 A");
    // Tabla A (B.7, quadruples): vwxy 0000 â†’ "1", 1111 â†’ "000001", 1011 â†’ "000000".
    assert_eq!(COUNT1_A[0b0000], (0b1, 1));
    assert_eq!(COUNT1_A[0b1111], (0b000001, 6));
    assert_eq!(COUNT1_A[0b1011], (0b000000, 6));
    // Tabla B: cÃ³digo de 4 bits = vwxy invertido.
    for (vwxy, &(code, len)) in COUNT1_B.iter().enumerate() {
        assert_eq!((code, len), (!(vwxy as u8) & 0xF, 4));
    }
}
