//! audio2mp3-core: encoder MP3 (MPEG-1 Layer III).
//!
//! PCM S16 interleaved ──► audio2mp3-core ──► frames MP3
//!
//! Estado (Fase 1): bitstream, cabecera/side info, filterbank, MDCT,
//! aliasing, cuantización, partición y tablas listos. Pendientes:
//! - TODO(fase1): `huffman.rs` (selección de tablas, conteo y escritura).
//! - TODO(fase1): `encoder.rs` (`Mp3Encoder`, buffer de 1152 muestras,
//!   inversión de frecuencia de subbandas impares antes de la MDCT, rate loop).

#![deny(unsafe_code)]

// TODO(fase1): quitar los allow(dead_code) cuando `Mp3Encoder` conecte el pipeline.
#[allow(dead_code)]
mod bitstream;
mod config;
mod error;
#[allow(dead_code)]
mod filterbank;
#[allow(dead_code)]
mod frame;
#[allow(dead_code)]
mod mdct;
#[allow(dead_code)]
mod quantize;
#[allow(dead_code)]
mod tables;

pub use config::{Bitrate, EncoderConfig};
pub use error::Error;
