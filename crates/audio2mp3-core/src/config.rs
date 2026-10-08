use crate::error::Error;

/// Bitrates CBR soportados (MPEG-1 Layer III).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bitrate {
    Kbps64,
    Kbps96,
    Kbps128,
    Kbps160,
    Kbps192,
    Kbps224,
    Kbps256,
    Kbps320,
}

impl Bitrate {
    pub fn from_kbps(kbps: u16) -> Result<Self, Error> {
        match kbps {
            64 => Ok(Bitrate::Kbps64),
            96 => Ok(Bitrate::Kbps96),
            128 => Ok(Bitrate::Kbps128),
            160 => Ok(Bitrate::Kbps160),
            192 => Ok(Bitrate::Kbps192),
            224 => Ok(Bitrate::Kbps224),
            256 => Ok(Bitrate::Kbps256),
            320 => Ok(Bitrate::Kbps320),
            _ => Err(Error::InvalidBitrate),
        }
    }

    pub fn kbps(self) -> u16 {
        match self {
            Bitrate::Kbps64 => 64,
            Bitrate::Kbps96 => 96,
            Bitrate::Kbps128 => 128,
            Bitrate::Kbps160 => 160,
            Bitrate::Kbps192 => 192,
            Bitrate::Kbps224 => 224,
            Bitrate::Kbps256 => 256,
            Bitrate::Kbps320 => 320,
        }
    }

    /// `bitrate_index` de la cabecera MPEG-1 Layer III.
    pub(crate) fn index(self) -> u32 {
        match self {
            Bitrate::Kbps64 => 5,
            Bitrate::Kbps96 => 7,
            Bitrate::Kbps128 => 9,
            Bitrate::Kbps160 => 10,
            Bitrate::Kbps192 => 11,
            Bitrate::Kbps224 => 12,
            Bitrate::Kbps256 => 13,
            Bitrate::Kbps320 => 14,
        }
    }
}

/// Configuración del encoder.
#[derive(Clone, Copy, Debug)]
pub struct EncoderConfig {
    /// 32000 | 44100 | 48000
    pub sample_rate: u32,
    /// 1 | 2
    pub channels: u8,
    pub bitrate: Bitrate,
}

impl EncoderConfig {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        sample_rate_index(self.sample_rate)?;
        if !matches!(self.channels, 1 | 2) {
            return Err(Error::InvalidChannels);
        }
        Ok(())
    }
}

/// Campo `sampling_frequency` de la cabecera MPEG-1.
pub(crate) fn sample_rate_index(sample_rate: u32) -> Result<u32, Error> {
    match sample_rate {
        44_100 => Ok(0),
        48_000 => Ok(1),
        32_000 => Ok(2),
        _ => Err(Error::InvalidSampleRate),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrate_roundtrip() {
        for kbps in [64, 96, 128, 160, 192, 224, 256, 320] {
            assert_eq!(Bitrate::from_kbps(kbps).map(Bitrate::kbps), Ok(kbps));
        }
        for kbps in [0, 32, 100, 384] {
            assert_eq!(Bitrate::from_kbps(kbps), Err(Error::InvalidBitrate));
        }
    }

    #[test]
    fn validate_rejects_bad_config() {
        let ok = EncoderConfig {
            sample_rate: 44_100,
            channels: 2,
            bitrate: Bitrate::Kbps192,
        };
        assert_eq!(ok.validate(), Ok(()));
        let bad_rate = EncoderConfig {
            sample_rate: 22_050,
            ..ok
        };
        assert_eq!(bad_rate.validate(), Err(Error::InvalidSampleRate));
        let bad_ch = EncoderConfig { channels: 3, ..ok };
        assert_eq!(bad_ch.validate(), Err(Error::InvalidChannels));
    }
}
