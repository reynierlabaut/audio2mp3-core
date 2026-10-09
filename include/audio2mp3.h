/*
 * audio2mp3.h — API C de audio2mp3-core (feature `ffi`).
 *
 * Escrito a mano. Los tests de crates/audio2mp3-core/src/ffi.rs comprueban
 * que los códigos y las funciones coinciden con la implementación.
 *
 * Reglas:
 * - Un handle no es thread-safe: un hilo por handle a la vez. Handles
 *   distintos pueden usarse en paralelo.
 * - Toda la memoria de entrada y salida la reserva y libera el llamador;
 *   Rust solo reserva el handle (create) y lo libera en destroy.
 * - Los buffers de entrada y salida no deben solaparse. `pcm` debe estar
 *   alineado a 2 bytes.
 * - Todas las funciones comprueban punteros nulos y devuelven
 *   AUDIO2MP3_ERR_NULL_POINTER.
 */

#ifndef AUDIO2MP3_H
#define AUDIO2MP3_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define AUDIO2MP3_OK                      0
#define AUDIO2MP3_ERR_INVALID_SAMPLE_RATE 1
#define AUDIO2MP3_ERR_INVALID_CHANNELS    2
#define AUDIO2MP3_ERR_INVALID_BITRATE     3
#define AUDIO2MP3_ERR_INVALID_INPUT       4
#define AUDIO2MP3_ERR_BUFFER_TOO_SMALL    5
#define AUDIO2MP3_ERR_FINISHED            6
#define AUDIO2MP3_ERR_NULL_POINTER        7
#define AUDIO2MP3_ERR_INTERNAL            99

typedef struct Audio2Mp3Encoder Audio2Mp3Encoder;

/* (major << 16) | (minor << 8) | patch */
uint32_t audio2mp3_version(void);

/* sample_rate: 32000 | 44100 | 48000; channels: 1 | 2;
 * bitrate_kbps: 64 | 96 | 128 | 160 | 192 | 224 | 256 | 320.
 * En caso de error *out_encoder queda en NULL. */
int32_t audio2mp3_encoder_create(uint32_t sample_rate, uint8_t channels,
                                 uint16_t bitrate_kbps,
                                 Audio2Mp3Encoder **out_encoder);

/* Bytes máximos que puede escribir encode con num_samples muestras
 * intercaladas (incluye lo acumulado). 0 si enc es NULL. */
size_t audio2mp3_encoder_max_output_size(const Audio2Mp3Encoder *enc,
                                         size_t num_samples);

/* Bytes máximos que puede escribir flush. 0 si enc es NULL. */
size_t audio2mp3_encoder_max_flush_size(const Audio2Mp3Encoder *enc);

/* Retardo del encoder en muestras por canal. 0 si enc es NULL. */
uint32_t audio2mp3_encoder_delay_samples(const Audio2Mp3Encoder *enc);

/* pcm: num_samples muestras S16 intercaladas (múltiplo de channels; puede
 * ser NULL si num_samples es 0). Si out_capacity es menor que
 * max_output_size devuelve BUFFER_TOO_SMALL sin consumir la entrada. */
int32_t audio2mp3_encoder_encode(Audio2Mp3Encoder *enc,
                                 const int16_t *pcm, size_t num_samples,
                                 uint8_t *out, size_t out_capacity,
                                 size_t *out_written);

/* Emite los últimos frames (incluido el retardo) y termina el encoder;
 * después encode y flush devuelven FINISHED. */
int32_t audio2mp3_encoder_flush(Audio2Mp3Encoder *enc,
                                uint8_t *out, size_t out_capacity,
                                size_t *out_written);

/* Acepta NULL. */
void audio2mp3_encoder_destroy(Audio2Mp3Encoder *enc);

/* Frame Info (cabecera Xing de CBR + extensión LAME): duración exacta y
 * retardo/padding para reproducción sin huecos.
 * 1. enable_info_frame antes del primer encode (si no, INVALID_INPUT):
 *    la salida empieza con un frame Info provisional de info_frame_size
 *    bytes.
 * 2. Tras flush, info_frame escribe el definitivo, que hay que copiar sobre
 *    los primeros info_frame_size bytes del archivo. Si no se hace, el
 *    archivo sigue siendo válido (sin duración exacta). */
int32_t audio2mp3_encoder_enable_info_frame(Audio2Mp3Encoder *enc);

/* 0 si el frame Info no está activado o enc es NULL. */
size_t audio2mp3_encoder_info_frame_size(const Audio2Mp3Encoder *enc);

/* Solo tras flush; si no, INVALID_INPUT. */
int32_t audio2mp3_encoder_info_frame(const Audio2Mp3Encoder *enc,
                                     uint8_t *out, size_t out_capacity,
                                     size_t *out_written);

/* ------------------------------------------------------------------
 * Converter (solo con la feature pcm-convert): PCM del decoder (S16/F32,
 * 1-8 canales en orden WAVE, 8000-192000 Hz) -> S16 intercalado de 1-2
 * canales a 32000/44100/48000 Hz, listo para el encoder.
 * Los bloques de entrada pueden cortar una muestra a la mitad.
 * ------------------------------------------------------------------ */

#define AUDIO2MP3_FORMAT_S16 0
#define AUDIO2MP3_FORMAT_F32 1

typedef struct Audio2Mp3Converter Audio2Mp3Converter;

/* in_format: AUDIO2MP3_FORMAT_S16 | AUDIO2MP3_FORMAT_F32 (orden de bytes
 * nativo); otro valor -> INVALID_INPUT. En caso de error *out_conv = NULL. */
int32_t audio2mp3_converter_create(uint32_t in_rate, uint8_t in_channels,
                                   uint8_t in_format,
                                   Audio2Mp3Converter **out_conv);

/* Tasa y canales de salida. 0 si c es NULL. */
uint32_t audio2mp3_converter_out_rate(const Audio2Mp3Converter *c);
uint8_t audio2mp3_converter_out_channels(const Audio2Mp3Converter *c);

/* Máximo de muestras (intercaladas) que escribe process con in_bytes bytes. */
size_t audio2mp3_converter_max_output_samples(const Audio2Mp3Converter *c,
                                              size_t in_bytes);

/* Máximo de muestras que escribe flush. */
size_t audio2mp3_converter_max_flush_samples(const Audio2Mp3Converter *c);

/* out_capacity y *out_written en muestras int16_t; out alineado a 2 bytes. */
int32_t audio2mp3_converter_process(Audio2Mp3Converter *c,
                                    const uint8_t *in, size_t in_bytes,
                                    int16_t *out, size_t out_capacity,
                                    size_t *out_written);

/* Emite las muestras retenidas por el filtro y termina el converter. */
int32_t audio2mp3_converter_flush(Audio2Mp3Converter *c,
                                  int16_t *out, size_t out_capacity,
                                  size_t *out_written);

/* Acepta NULL. */
void audio2mp3_converter_destroy(Audio2Mp3Converter *c);

#ifdef __cplusplus
}
#endif

#endif /* AUDIO2MP3_H */
