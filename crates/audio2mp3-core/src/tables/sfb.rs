//! Límites de las bandas de scalefactors (Anexo B, tabla B.8).
//! Largas: 22 bandas sobre 576 líneas. Cortas: 13 bandas sobre las 192
//! líneas de cada una de las 3 ventanas. Banda i: `bands[i]..bands[i + 1]`.

pub(crate) type LongBands = [u16; 23];
pub(crate) type ShortBands = [u16; 14];

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

const SHORT_44100: ShortBands = [0, 4, 8, 12, 16, 22, 30, 40, 52, 66, 84, 106, 136, 192];
const SHORT_48000: ShortBands = [0, 4, 8, 12, 16, 22, 28, 38, 50, 64, 80, 100, 126, 192];
const SHORT_32000: ShortBands = [0, 4, 8, 12, 16, 22, 30, 42, 58, 78, 104, 138, 180, 192];

/// Bandas largas para una tasa ya validada.
pub(crate) fn long_bands(sample_rate: u32) -> Option<&'static LongBands> {
    match sample_rate {
        44_100 => Some(&LONG_44100),
        48_000 => Some(&LONG_48000),
        32_000 => Some(&LONG_32000),
        _ => None,
    }
}

/// Bandas cortas (por ventana) para una tasa ya validada.
pub(crate) fn short_bands(sample_rate: u32) -> Option<&'static ShortBands> {
    match sample_rate {
        44_100 => Some(&SHORT_44100),
        48_000 => Some(&SHORT_48000),
        32_000 => Some(&SHORT_32000),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_are_even_increasing_and_cover_the_granule() {
        for rate in [32_000, 44_100, 48_000] {
            let long = long_bands(rate).unwrap();
            assert_eq!((long[0], long[22]), (0, 576));
            let short = short_bands(rate).unwrap();
            assert_eq!((short[0], short[13]), (0, 192));
            for pair in long.windows(2).chain(short.windows(2)) {
                assert!(pair[0] < pair[1]);
                assert_eq!(pair[1] % 2, 0);
            }
            // Con conmutación de bloques, region0 termina en la línea 36 en
            // ambos casos: banda larga 8 y 3 bandas cortas × 3 ventanas.
            assert_eq!(long[8], 36);
            assert_eq!(short[3] * 3, 36);
        }
        assert!(long_bands(22_050).is_none());
        assert!(short_bands(22_050).is_none());
    }
}
