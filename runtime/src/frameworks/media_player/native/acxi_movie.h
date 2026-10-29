// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
#ifndef ACXI_MOVIE_H
#define ACXI_MOVIE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct AcxiMovie AcxiMovie;

typedef struct AcxiMovieInfo {
    uint32_t width, height;
    uint32_t frame_rate_num, frame_rate_den;
    uint32_t audio_sample_rate, audio_channels, audio_bits;
    int64_t duration_100ns;
} AcxiMovieInfo;

enum { ACXI_MOVIE_VIDEO = 1, ACXI_MOVIE_AUDIO = 2 };
typedef struct AcxiMovieSample {
    uint32_t kind;
    int64_t pts_100ns, duration_100ns;
    const uint8_t *data;
    size_t bytes;
} AcxiMovieSample;

/* Thread-confined decoder. All calls, including close, use the opening thread.
 * Source MP4 bytes are copied; the caller can release input after open returns.
 * Video output is packed top-to-bottom RGBA8 with alpha=255.
 * Audio output is original-rate interleaved signed 16-bit little-endian PCM.
 * No D3D device manager, DXVA surface or graphics presenter is created.
 */
AcxiMovie *acxi_movie_open(const void *mp4, size_t bytes,
                          char *error, size_t error_size);
const AcxiMovieInfo *acxi_movie_info(const AcxiMovie *movie);
/* 1 = sample, 0 = complete EOF, -1 = error. Sample memory remains valid until
 * the next read or close. Audio and video samples retain their media PTS. */
int acxi_movie_read(AcxiMovie *movie, AcxiMovieSample *sample);
const char *acxi_movie_error(const AcxiMovie *movie);
void acxi_movie_close(AcxiMovie *movie);

#ifdef __cplusplus
}
#endif
#endif
