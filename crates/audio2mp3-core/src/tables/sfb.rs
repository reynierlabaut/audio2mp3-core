//! Límites de las bandas de scalefactors para bloques largos
//! (Anexo B, tabla B.8). 22 bandas: `bands[i]..bands[i + 1]`.

pub(crate) type LongBands = [u16; 23];

const LONG_44100: LongBands = [
    0, 4, 8, 12, 16, 20, 24, 30, 36, 44, 52, 62, 74, 90, 110, 134, 162, 196, 238, 288, 342, 418,
    576,
];

const LONG_48000: LongBands = [
    0, 4, 8, 12, 16, 20, 24, 30, 36, 42, 50, 60, 72, 88, 106, 128, 156, 190, 230, 276, 330, 384,
    576,
];

const LONG_32000: LongBands = [
    0, 4, 8, 12, 16, 20, 24, 30, 36, 44, 54, 66, 82, 102, 126, 156, 194, 240, 296, 364, 448, 550,
    576,
];

/// Bandas largas para una tasa ya validada.
pub(crate) fn long_bands(sample_rate: u32) -> Option<&'static LongBands> {
    match sample_rate {
        44_100 => Some(&LONG_44100),
        48_000 => Some(&LONG_48000),
        32_000 => Some(&LONG_32000),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_are_even_increasing_and_cover_576() {
        for rate in [32_000, 44_100, 48_000] {
            let bands = long_bands(rate).unwrap();
            assert_eq!(bands[0], 0);
            assert_eq!(bands[22], 576);
            for pair in bands.windows(2) {
                assert!(pair[0] < pair[1]);
                assert_eq!(pair[1] % 2, 0);
            }
        }
        assert!(long_bands(22_050).is_none());
    }
}
