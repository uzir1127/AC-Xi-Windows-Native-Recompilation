// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
/* Native Windows movie decoding for ACXi. All presentation remains Vulkan.
 * This file is independently authored; Media Foundation supplies MP4/H264/AAC
 * decoding and optional CPU color conversion, without a D3D device manager.
 */
#define COBJMACROS
#define WIN32_LEAN_AND_MEAN
#define _WIN32_WINNT 0x0601
#include <windows.h>
#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <mferror.h>
#include <propvarutil.h>
#include <objbase.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>
#include "acxi_movie.h"

/* Present in Windows 7+ mfplat.dll; some MinGW header versions omit it. */
HRESULT WINAPI MFCreateMFByteStreamOnStream(IStream *, IMFByteStream **);

struct AcxiMovie {
    IStream *stream;
    IMFByteStream *byte_stream;
    IMFSourceReader *reader;
    DWORD video_stream, audio_stream;
    int video_eos, audio_eos;
    int uninitialize_com, shutdown_mf;
    LONG video_stride;
    uint32_t natural_width, natural_height, coded_width, coded_height;
    uint32_t crop_x, crop_y;
    uint8_t *buffer;
    size_t buffer_capacity;
    AcxiMovieInfo info;
    char error[256];
};

static int fail(AcxiMovie *movie, const char *operation, HRESULT hr)
{
    snprintf(movie->error, sizeof movie->error, "%s: HRESULT 0x%08lx",
             operation, (unsigned long)hr);
    return -1;
}

static int reserve(AcxiMovie *movie, size_t bytes)
{
    void *buffer;
    if (bytes <= movie->buffer_capacity) return 1;
    buffer = realloc(movie->buffer, bytes);
    if (!buffer) return fail(movie, "Allocating decoded movie sample", E_OUTOFMEMORY);
    movie->buffer = buffer;
    movie->buffer_capacity = bytes;
    return 1;
}

static HRESULT update_video_info(AcxiMovie *movie)
{
    IMFMediaType *type = NULL;
    UINT64 frame_size = 0, frame_rate = 0;
    UINT32 stride = 0;
    HRESULT hr = IMFSourceReader_GetCurrentMediaType(movie->reader, movie->video_stream, &type);
    if (FAILED(hr)) return hr;
    hr = IMFMediaType_GetUINT64(type, &MF_MT_FRAME_SIZE, &frame_size);
    if (SUCCEEDED(hr)) {
        MFVideoArea aperture;
        UINT32 aperture_bytes = 0;
        movie->coded_width = (uint32_t)(frame_size >> 32);
        movie->coded_height = (uint32_t)frame_size;
        movie->info.width = movie->coded_width;
        movie->info.height = movie->coded_height;
        movie->crop_x = movie->crop_y = 0;
        /* H264 output may be macroblock padded (ACXi is 480x272 coded but
         * 480x270 visible). Preserve the stream's display aperture. */
        HRESULT aperture_hr = IMFMediaType_GetBlob(type, &MF_MT_MINIMUM_DISPLAY_APERTURE,
            (UINT8 *)&aperture, sizeof aperture, &aperture_bytes);
        if (FAILED(aperture_hr)) aperture_hr = IMFMediaType_GetBlob(type, &MF_MT_GEOMETRIC_APERTURE,
            (UINT8 *)&aperture, sizeof aperture, &aperture_bytes);
        if (SUCCEEDED(aperture_hr) && aperture_bytes == sizeof aperture &&
            aperture.OffsetX.value >= 0 && aperture.OffsetY.value >= 0 &&
            aperture.OffsetX.fract == 0 && aperture.OffsetY.fract == 0 &&
            aperture.Area.cx > 0 && aperture.Area.cy > 0 &&
            (uint64_t)aperture.OffsetX.value + aperture.Area.cx <= movie->coded_width &&
            (uint64_t)aperture.OffsetY.value + aperture.Area.cy <= movie->coded_height) {
            movie->crop_x = (uint32_t)aperture.OffsetX.value;
            movie->crop_y = (uint32_t)aperture.OffsetY.value;
            movie->info.width = (uint32_t)aperture.Area.cx;
            movie->info.height = (uint32_t)aperture.Area.cy;
        } else if (movie->natural_width && movie->natural_height &&
                   movie->natural_width <= movie->coded_width && movie->natural_height <= movie->coded_height) {
            movie->info.width = movie->natural_width;
            movie->info.height = movie->natural_height;
        }
        if (SUCCEEDED(IMFMediaType_GetUINT64(type, &MF_MT_FRAME_RATE, &frame_rate))) {
            movie->info.frame_rate_num = (uint32_t)(frame_rate >> 32);
            movie->info.frame_rate_den = (uint32_t)frame_rate;
        }
        if (SUCCEEDED(IMFMediaType_GetUINT32(type, &MF_MT_DEFAULT_STRIDE, &stride)))
            movie->video_stride = (LONG)stride;
        else
            movie->video_stride = (LONG)(movie->coded_width * 4);
    }
    IMFMediaType_Release(type);
    return hr;
}

static HRESULT update_audio_info(AcxiMovie *movie)
{
    IMFMediaType *type = NULL;
    HRESULT hr = IMFSourceReader_GetCurrentMediaType(movie->reader, movie->audio_stream, &type);
    if (FAILED(hr)) return hr;
    hr = IMFMediaType_GetUINT32(type, &MF_MT_AUDIO_SAMPLES_PER_SECOND, &movie->info.audio_sample_rate);
    if (SUCCEEDED(hr)) hr = IMFMediaType_GetUINT32(type, &MF_MT_AUDIO_NUM_CHANNELS, &movie->info.audio_channels);
    if (SUCCEEDED(hr)) hr = IMFMediaType_GetUINT32(type, &MF_MT_AUDIO_BITS_PER_SAMPLE, &movie->info.audio_bits);
    IMFMediaType_Release(type);
    return hr;
}

static HRESULT set_output(AcxiMovie *movie, DWORD stream, int video)
{
    IMFMediaType *type = NULL;
    HRESULT hr = MFCreateMediaType(&type);
    if (SUCCEEDED(hr)) hr = IMFMediaType_SetGUID(type, &MF_MT_MAJOR_TYPE, video ? &MFMediaType_Video : &MFMediaType_Audio);
    if (SUCCEEDED(hr)) hr = IMFMediaType_SetGUID(type, &MF_MT_SUBTYPE, video ? &MFVideoFormat_RGB32 : &MFAudioFormat_PCM);
    if (SUCCEEDED(hr) && !video) hr = IMFMediaType_SetUINT32(type, &MF_MT_AUDIO_BITS_PER_SAMPLE, 16);
    if (SUCCEEDED(hr)) hr = IMFSourceReader_SetCurrentMediaType(movie->reader, stream, NULL, type);
    if (type) IMFMediaType_Release(type);
    return hr;
}

AcxiMovie *acxi_movie_open(const void *mp4, size_t bytes, char *error, size_t error_size)
{
    AcxiMovie *movie = calloc(1, sizeof *movie);
    IMFAttributes *attributes = NULL;
    HGLOBAL memory = NULL;
    void *destination;
    HRESULT hr;
    const char *operation = "Opening movie";
    if (!movie) {
        if (error && error_size) snprintf(error, error_size, "Out of memory");
        return NULL;
    }
    movie->video_stream = movie->audio_stream = MF_SOURCE_READER_INVALID_STREAM_INDEX;
    if (!mp4 || bytes == 0) { hr = E_INVALIDARG; goto failed; }
    operation = "Initializing COM";
    hr = CoInitializeEx(NULL, COINIT_MULTITHREADED);
    if (SUCCEEDED(hr)) movie->uninitialize_com = 1;
    else if (hr != RPC_E_CHANGED_MODE) goto failed;
    operation = "Starting Media Foundation";
    hr = MFStartup(MF_VERSION, MFSTARTUP_LITE);
    if (FAILED(hr)) goto failed;
    movie->shutdown_mf = 1;
    operation = "Copying MP4 into memory stream";
    memory = GlobalAlloc(GMEM_MOVEABLE, bytes);
    if (!memory) { hr = E_OUTOFMEMORY; goto failed; }
    destination = GlobalLock(memory);
    if (!destination) { hr = E_OUTOFMEMORY; goto failed; }
    memcpy(destination, mp4, bytes);
    GlobalUnlock(memory);
    hr = CreateStreamOnHGlobal(memory, TRUE, &movie->stream);
    if (FAILED(hr)) goto failed;
    memory = NULL; /* IStream now owns and releases the allocation. */
    operation = "Creating movie byte stream";
    hr = MFCreateMFByteStreamOnStream(movie->stream, &movie->byte_stream);
    if (FAILED(hr)) goto failed;
    operation = "Configuring software movie decoder";
    hr = MFCreateAttributes(&attributes, 3);
    if (SUCCEEDED(hr)) hr = IMFAttributes_SetUINT32(attributes, &MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, FALSE);
    if (SUCCEEDED(hr)) hr = IMFAttributes_SetUINT32(attributes, &MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, TRUE);
    if (SUCCEEDED(hr)) hr = IMFAttributes_SetUINT32(attributes, &MF_SOURCE_READER_DISABLE_DXVA, TRUE);
    /* MF_SOURCE_READER_D3D_MANAGER is deliberately never supplied. */
    if (FAILED(hr)) goto failed;
    operation = "Opening MP4 source reader";
    hr = MFCreateSourceReaderFromByteStream(movie->byte_stream, attributes, &movie->reader);
    if (FAILED(hr)) goto failed;
    IMFAttributes_Release(attributes); attributes = NULL;
    operation = "Discovering movie streams";
    for (DWORD index = 0; index < 32; index++) {
        IMFMediaType *type = NULL;
        GUID major;
        hr = IMFSourceReader_GetNativeMediaType(movie->reader, index, 0, &type);
        if (hr == MF_E_INVALIDSTREAMNUMBER) break;
        if (FAILED(hr)) goto failed;
        hr = IMFMediaType_GetGUID(type, &MF_MT_MAJOR_TYPE, &major);
        if (FAILED(hr)) { IMFMediaType_Release(type); goto failed; }
        if (IsEqualGUID(&major, &MFMediaType_Video) && movie->video_stream == MF_SOURCE_READER_INVALID_STREAM_INDEX) {
            UINT64 natural_size = 0;
            movie->video_stream = index;
            if (SUCCEEDED(IMFMediaType_GetUINT64(type, &MF_MT_FRAME_SIZE, &natural_size))) {
                movie->natural_width = (uint32_t)(natural_size >> 32);
                movie->natural_height = (uint32_t)natural_size;
            }
        }
        else if (IsEqualGUID(&major, &MFMediaType_Audio) && movie->audio_stream == MF_SOURCE_READER_INVALID_STREAM_INDEX)
            movie->audio_stream = index;
        IMFMediaType_Release(type);
    }
    hr = IMFSourceReader_SetStreamSelection(movie->reader, MF_SOURCE_READER_ALL_STREAMS, FALSE);
    if (FAILED(hr)) goto failed;
    if (movie->video_stream != MF_SOURCE_READER_INVALID_STREAM_INDEX) {
        operation = "Selecting RGBA movie video";
        hr = set_output(movie, movie->video_stream, 1);
        if (SUCCEEDED(hr)) hr = update_video_info(movie);
        if (SUCCEEDED(hr)) hr = IMFSourceReader_SetStreamSelection(movie->reader, movie->video_stream, TRUE);
        if (FAILED(hr)) goto failed;
    } else movie->video_eos = 1;
    if (movie->audio_stream != MF_SOURCE_READER_INVALID_STREAM_INDEX) {
        operation = "Selecting PCM movie audio";
        hr = set_output(movie, movie->audio_stream, 0);
        if (SUCCEEDED(hr)) hr = update_audio_info(movie);
        if (SUCCEEDED(hr)) hr = IMFSourceReader_SetStreamSelection(movie->reader, movie->audio_stream, TRUE);
        if (FAILED(hr)) goto failed;
    } else movie->audio_eos = 1;
    if (movie->video_eos && movie->audio_eos) { hr = MF_E_INVALIDMEDIATYPE; goto failed; }
    {
        PROPVARIANT duration;
        PropVariantInit(&duration);
        hr = IMFSourceReader_GetPresentationAttribute(movie->reader, MF_SOURCE_READER_MEDIASOURCE, &MF_PD_DURATION, &duration);
        if (SUCCEEDED(hr) && duration.vt == VT_UI8)
            movie->info.duration_100ns = (int64_t)duration.uhVal.QuadPart;
        PropVariantClear(&duration);
    }
    return movie;
failed:
    fail(movie, operation, hr);
    if (error && error_size) snprintf(error, error_size, "%s", movie->error);
    if (attributes) IMFAttributes_Release(attributes);
    if (memory) GlobalFree(memory);
    acxi_movie_close(movie);
    return NULL;
}

const AcxiMovieInfo *acxi_movie_info(const AcxiMovie *movie) { return &movie->info; }
const char *acxi_movie_error(const AcxiMovie *movie) { return movie->error; }

static int copy_video(AcxiMovie *movie, IMFMediaBuffer *buffer, AcxiMovieSample *output)
{
    IMF2DBuffer *buffer2d = NULL;
    BYTE *scan0 = NULL, *base = NULL;
    LONG pitch = movie->video_stride;
    DWORD bytes = 0;
    HRESULT hr;
    int locked2d = 0;
    uint32_t w = movie->info.width, h = movie->info.height;
    size_t packed_size;
    if (w == 0 || h == 0 || w > SIZE_MAX / 4 / h)
        return fail(movie, "Invalid video dimensions", MF_E_INVALIDMEDIATYPE);
    packed_size = (size_t)w * h * 4;
    if (reserve(movie, packed_size) < 0) return -1;
    hr = IMFMediaBuffer_QueryInterface(buffer, &IID_IMF2DBuffer, (void **)&buffer2d);
    if (SUCCEEDED(hr)) {
        hr = IMF2DBuffer_Lock2D(buffer2d, &scan0, &pitch);
        locked2d = SUCCEEDED(hr);
        if (!locked2d) { IMF2DBuffer_Release(buffer2d); buffer2d = NULL; }
    }
    if (!locked2d) {
        hr = IMFMediaBuffer_Lock(buffer, &base, NULL, &bytes);
        if (FAILED(hr)) return fail(movie, "Locking video sample", hr);
        if ((uint64_t)(pitch < 0 ? -(int64_t)pitch : pitch) * movie->coded_height > bytes) {
            IMFMediaBuffer_Unlock(buffer);
            return fail(movie, "Video sample buffer is too small", MF_E_BUFFERTOOSMALL);
        }
        scan0 = base;
        if (pitch < 0) scan0 += (size_t)(movie->coded_height - 1) * (size_t)(-pitch);
    }
    if ((uint64_t)(pitch < 0 ? -(int64_t)pitch : pitch) < (uint64_t)w * 4) {
        if (locked2d) { IMF2DBuffer_Unlock2D(buffer2d); IMF2DBuffer_Release(buffer2d); }
        else IMFMediaBuffer_Unlock(buffer);
        return fail(movie, "Invalid video row pitch", MF_E_INVALIDMEDIATYPE);
    }
    for (uint32_t y = 0; y < h; y++) {
        const uint8_t *in = scan0 + (ptrdiff_t)(y + movie->crop_y) * pitch + (size_t)movie->crop_x * 4;
        uint8_t *out = movie->buffer + (size_t)y * w * 4;
        for (uint32_t x = 0; x < w; x++) {
            out[x * 4 + 0] = in[x * 4 + 2];
            out[x * 4 + 1] = in[x * 4 + 1];
            out[x * 4 + 2] = in[x * 4 + 0];
            out[x * 4 + 3] = 255;
        }
    }
    if (locked2d) { IMF2DBuffer_Unlock2D(buffer2d); IMF2DBuffer_Release(buffer2d); }
    else IMFMediaBuffer_Unlock(buffer);
    output->data = movie->buffer;
    output->bytes = packed_size;
    output->kind = ACXI_MOVIE_VIDEO;
    return 1;
}

int acxi_movie_read(AcxiMovie *movie, AcxiMovieSample *output)
{
    if (!movie || !output) return -1;
    memset(output, 0, sizeof *output);
    if (movie->video_eos && movie->audio_eos) return 0;
    for (unsigned guard = 0; guard < 10000; guard++) {
        DWORD stream = 0, flags = 0;
        LONGLONG pts = 0, duration = 0;
        IMFSample *sample = NULL;
        IMFMediaBuffer *buffer = NULL;
        HRESULT hr = IMFSourceReader_ReadSample(movie->reader, MF_SOURCE_READER_ANY_STREAM, 0, &stream, &flags, &pts, &sample);
        if (FAILED(hr)) return fail(movie, "Decoding movie sample", hr);
        if (flags & MF_SOURCE_READERF_ERROR) {
            if (sample) IMFSample_Release(sample);
            return fail(movie, "Media source reader reported failure", E_FAIL);
        }
        if (flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED) {
            hr = stream == movie->video_stream ? update_video_info(movie) : update_audio_info(movie);
            if (FAILED(hr)) { if (sample) IMFSample_Release(sample); return fail(movie, "Updating movie output format", hr); }
        }
        if (flags & MF_SOURCE_READERF_ENDOFSTREAM) {
            if (stream == movie->video_stream) movie->video_eos = 1;
            if (stream == movie->audio_stream) movie->audio_eos = 1;
            IMFSourceReader_SetStreamSelection(movie->reader, stream, FALSE);
            if (sample) IMFSample_Release(sample);
            if (movie->video_eos && movie->audio_eos) return 0;
            continue;
        }
        if (!sample) continue;
        output->pts_100ns = pts;
        if (SUCCEEDED(IMFSample_GetSampleDuration(sample, &duration))) output->duration_100ns = duration;
        hr = IMFSample_ConvertToContiguousBuffer(sample, &buffer);
        IMFSample_Release(sample);
        if (FAILED(hr)) return fail(movie, "Obtaining decoded movie bytes", hr);
        if (stream == movie->video_stream) {
            int result = copy_video(movie, buffer, output);
            IMFMediaBuffer_Release(buffer);
            return result;
        } else if (stream == movie->audio_stream) {
            BYTE *data = NULL;
            DWORD bytes = 0;
            hr = IMFMediaBuffer_Lock(buffer, &data, NULL, &bytes);
            if (FAILED(hr)) { IMFMediaBuffer_Release(buffer); return fail(movie, "Locking PCM sample", hr); }
            if (reserve(movie, bytes) < 0) { IMFMediaBuffer_Unlock(buffer); IMFMediaBuffer_Release(buffer); return -1; }
            memcpy(movie->buffer, data, bytes);
            IMFMediaBuffer_Unlock(buffer);
            IMFMediaBuffer_Release(buffer);
            output->kind = ACXI_MOVIE_AUDIO;
            output->data = movie->buffer;
            output->bytes = bytes;
            return 1;
        }
        IMFMediaBuffer_Release(buffer);
    }
    return fail(movie, "Too many empty movie samples", E_FAIL);
}

void acxi_movie_close(AcxiMovie *movie)
{
    if (!movie) return;
    if (movie->reader) IMFSourceReader_Release(movie->reader);
    if (movie->byte_stream) IMFByteStream_Release(movie->byte_stream);
    if (movie->stream) IStream_Release(movie->stream);
    free(movie->buffer);
    if (movie->shutdown_mf) MFShutdown();
    if (movie->uninitialize_com) CoUninitialize();
    free(movie);
}
