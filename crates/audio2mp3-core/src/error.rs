use core::fmt;

/// Errores de la API pública.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Sample rate distinto de 32 000, 44 100 o 48 000 Hz.
    InvalidSampleRate,
    /// Número de canales distinto de 1 o 2.
    InvalidChannels,
    /// Bitrate no soportado.
    InvalidBitrate,
    /// Entrada mal formada (p. ej. muestras no múltiplo de `channels`).
    InvalidInput,
    /// El buffer de salida es menor que el tamaño máximo requerido.
    BufferTooSmall,
    /// El encoder ya se vació con `flush`.
    Finished,
    /// Bug interno. No debería ocurrir nunca; existe para no hacer panic.
    Internal,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Error::InvalidSampleRate => "sample rate no soportado (32000, 44100 o 48000 Hz)",
            Error::InvalidChannels => "número de canales no soportado (1 o 2)",
            Error::InvalidBitrate => "bitrate no soportado",
            Error::InvalidInput => "entrada PCM inválida",
            Error::BufferTooSmall => "buffer de salida demasiado pequeño",
            Error::Finished => "el encoder ya terminó (flush)",
            Error::Internal => "error interno del encoder",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for Error {}
