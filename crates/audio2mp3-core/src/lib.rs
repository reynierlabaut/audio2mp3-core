//! audio2mp3-core: encoder MP3 (MPEG-1 Layer III).
//!
//! PCM S16 interleaved ──► audio2mp3-core ──► frames MP3
//!
//! ```
//! use audio2mp3_core::{Bitrate, EncoderConfig, Mp3Encoder};
//!
//! let config = EncoderConfig { sample_rate: 44_100, channels: 2, bitrate: Bitrate::Kbps192 };
//! let mut encoder = Mp3Encoder::new(config)?;
//! let pcm = vec![0i16; 44_100 * 2]; // 1 s de silencio estéreo
//! let mut mp3 = Vec::new();
//! encoder.encode(&pcm, &mut mp3)?;
//! encoder.flush(&mut mp3)?;
//! # Ok::<(), audio2mp3_core::Error>(())
//! ```

#![deny(unsafe_code)]

mod bitstream;
mod config;
#[cfg(feature = "pcm-convert")]
mod convert;
mod encoder;
mod error;
#[cfg(feature = "ffi")]
#[allow(unsafe_code)]
pub mod ffi;
mod filterbank;
mod frame;
mod huffman;
mod mdct;
mod quantize;
mod reservoir;
mod tables;

pub use config::{Bitrate, EncoderConfig};
#[cfg(feature = "pcm-convert")]
pub use convert::{output_rate, PcmConverter, SampleFormat, MAX_INPUT_CHANNELS};
pub use encoder::Mp3Encoder;
pub use error::Error;
