/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Original MP4 decoding and timestamped audio/video playback.
//! Video is returned as CPU RGBA for the Vulkan-backed UIKit compositor.

use std::collections::VecDeque;
use std::ffi::{c_char, c_void, CStr};
use std::ptr::{null, null_mut, NonNull};
use std::time::Instant;
use touchHLE_openal_soft_wrapper as al;
use al::al_defines::*;
use al::alc_types::{ALCcontext, ALCdevice};

// OpenAL 1.1 constants from vendor/openal-soft/include/AL/al.h. The runtime's
// existing small binding exports only the subset used by older audio APIs.
const AL_TRUE: i32 = 1;
const AL_SOURCE_RELATIVE: i32 = 0x202;
const AL_ROLLOFF_FACTOR: i32 = 0x1021;
const AL_SAMPLE_OFFSET: i32 = 0x1025;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct MovieInfo {
    pub width: u32,
    pub height: u32,
    pub frame_rate_num: u32,
    pub frame_rate_den: u32,
    pub audio_sample_rate: u32,
    pub audio_channels: u32,
    pub audio_bits: u32,
    pub duration_100ns: i64,
}

#[repr(C)]
struct RawSample {
    kind: u32,
    pts_100ns: i64,
    duration_100ns: i64,
    data: *const u8,
    bytes: usize,
}

extern "C" {
    fn acxi_movie_open(bytes: *const c_void, size: usize, error: *mut c_char, error_size: usize) -> *mut c_void;
    fn acxi_movie_info(movie: *const c_void) -> *const MovieInfo;
    fn acxi_movie_read(movie: *mut c_void, sample: *mut RawSample) -> i32;
    fn acxi_movie_error(movie: *const c_void) -> *const c_char;
    fn acxi_movie_close(movie: *mut c_void);
}

pub(super) struct VideoFrame {
    pub pts: f64,
    pub pixels: Vec<u8>,
}

struct Decoder(NonNull<c_void>);
impl Decoder {
    fn open(bytes: &[u8]) -> Result<Self, String> {
        let mut error = [0i8; 256];
        let pointer = unsafe { acxi_movie_open(bytes.as_ptr().cast(), bytes.len(), error.as_mut_ptr(), error.len()) };
        NonNull::new(pointer).map(Self).ok_or_else(|| unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy().into_owned())
    }
    fn info(&self) -> MovieInfo { unsafe { *acxi_movie_info(self.0.as_ptr()) } }
    fn read(&mut self) -> Result<Option<(u32, f64, f64, Vec<u8>)>, String> {
        let mut sample: RawSample = unsafe { std::mem::zeroed() };
        match unsafe { acxi_movie_read(self.0.as_ptr(), &mut sample) } {
            0 => Ok(None),
            1 => {
                let bytes = unsafe { std::slice::from_raw_parts(sample.data, sample.bytes) }.to_vec();
                Ok(Some((sample.kind, sample.pts_100ns as f64 / 10_000_000.0,
                    sample.duration_100ns as f64 / 10_000_000.0, bytes)))
            }
            _ => Err(unsafe { CStr::from_ptr(acxi_movie_error(self.0.as_ptr())) }.to_string_lossy().into_owned()),
        }
    }
}
impl Drop for Decoder {
    fn drop(&mut self) { unsafe { acxi_movie_close(self.0.as_ptr()); } }
}

/// An isolated OpenAL context keeps the movie's source/buffers separate from
/// the app's AudioUnit/AudioQueue state. Every operation restores the previous
/// current context before returning to translated game code.
struct MovieAudio {
    device: *mut ALCdevice,
    context: *mut ALCcontext,
    source: u32,
    queued: VecDeque<(u32, u64)>,
    processed_samples: u64,
    rate: u32,
    channels: u32,
    started: bool,
    paused: bool,
}
struct RestoreAlContext(*mut ALCcontext);
impl Drop for RestoreAlContext {
    fn drop(&mut self) { unsafe { al::alcMakeContextCurrent(self.0); } }
}
impl MovieAudio {
    fn activate(&self) -> RestoreAlContext {
        let previous = unsafe { al::alcGetCurrentContext() };
        assert_ne!(unsafe { al::alcMakeContextCurrent(self.context) }, 0);
        RestoreAlContext(previous)
    }
    fn new(rate: u32, channels: u32) -> Result<Self, String> {
        if !matches!(channels, 1 | 2) || rate == 0 { return Err("Unsupported movie audio channel layout".into()); }
        let device = unsafe { al::alcOpenDevice(null()) };
        if device.is_null() { return Err("Could not open movie audio output".into()); }
        let context = unsafe { al::alcCreateContext(device, null()) };
        if context.is_null() { unsafe { al::alcCloseDevice(device); } return Err("Could not create movie audio context".into()); }
        let mut audio = Self { device, context, source: 0, queued: VecDeque::new(), processed_samples: 0,
            rate, channels, started: false, paused: false };
        {
            let _restore = audio.activate();
            unsafe {
                al::alGenSources(1, &mut audio.source);
                al::alSourcei(audio.source, AL_SOURCE_RELATIVE as _, AL_TRUE as _);
                al::alSourcef(audio.source, AL_ROLLOFF_FACTOR as _, 0.0);
                let error = al::alGetError();
                if error != AL_NO_ERROR as _ { return Err(format!("Creating movie audio source: OpenAL error {error:#x}")); }
            }
        }
        Ok(audio)
    }
    fn queue(&mut self, bytes: &[u8]) -> Result<(), String> {
        let _restore = self.activate();
        let mut buffer = 0;
        unsafe {
            al::alGenBuffers(1, &mut buffer);
            al::alBufferData(buffer, if self.channels == 1 { AL_FORMAT_MONO16 as _ } else { AL_FORMAT_STEREO16 as _ },
                bytes.as_ptr().cast(), bytes.len().try_into().map_err(|_| "Movie audio packet too large")?, self.rate as _);
            al::alSourceQueueBuffers(self.source, 1, &buffer);
            let error = al::alGetError();
            if error != AL_NO_ERROR as _ { al::alDeleteBuffers(1, &buffer); return Err(format!("Queuing movie PCM: OpenAL error {error:#x}")); }
            if self.started && !self.paused {
                let mut state = 0;
                al::alGetSourcei(self.source, AL_SOURCE_STATE as _, &mut state);
                if state == AL_STOPPED as _ { al::alSourcePlay(self.source); }
            }
        }
        self.queued.push_back((buffer, bytes.len() as u64 / (u64::from(self.channels) * 2)));
        Ok(())
    }
    fn resume(&mut self) {
        let _restore = self.activate();
        if !self.queued.is_empty() { unsafe { al::alSourcePlay(self.source); } }
        self.started = true;
        self.paused = false;
    }
    fn pause(&mut self) {
        let _restore = self.activate();
        unsafe { al::alSourcePause(self.source); }
        self.paused = true;
    }
    fn position(&mut self) -> (f64, bool) {
        let _restore = self.activate();
        unsafe {
            let mut processed = 0;
            al::alGetSourcei(self.source, AL_BUFFERS_PROCESSED as _, &mut processed);
            for _ in 0..processed {
                let mut buffer = 0;
                al::alSourceUnqueueBuffers(self.source, 1, &mut buffer);
                let (expected, frames) = self.queued.pop_front().expect("Movie audio queue accounting");
                assert_eq!(expected, buffer);
                self.processed_samples += frames;
                al::alDeleteBuffers(1, &buffer);
            }
            let (mut offset, mut state) = (0, 0);
            al::alGetSourcei(self.source, AL_SAMPLE_OFFSET as _, &mut offset);
            al::alGetSourcei(self.source, AL_SOURCE_STATE as _, &mut state);
            ((self.processed_samples + offset.max(0) as u64) as f64 / f64::from(self.rate),
             self.started && state == AL_STOPPED as _ && self.queued.is_empty())
        }
    }
}
impl Drop for MovieAudio {
    fn drop(&mut self) {
        let previous = unsafe { al::alcGetCurrentContext() };
        unsafe {
            al::alcMakeContextCurrent(self.context);
            if self.source != 0 { al::alSourceStop(self.source); al::alDeleteSources(1, &self.source); }
            for &(buffer, _) in &self.queued { al::alDeleteBuffers(1, &buffer); }
            al::alcMakeContextCurrent(if previous == self.context { null_mut() } else { previous });
            al::alcDestroyContext(self.context);
            al::alcCloseDevice(self.device);
        }
    }
}

pub(super) struct Playback {
    decoder: Decoder,
    pub info: MovieInfo,
    audio: Option<MovieAudio>,
    frames: VecDeque<VideoFrame>,
    video_decoded_to: f64,
    audio_decoded_to: f64,
    eof: bool,
    playing_since: Option<Instant>,
    paused_position: f64,
    audio_tail_since: Option<Instant>,
    pub decoded_video_frames: u64,
    pub decoded_audio_bytes: u64,
}
impl Playback {
    pub fn open(bytes: &[u8]) -> Result<Self, String> {
        let decoder = Decoder::open(bytes)?;
        let info = decoder.info();
        let audio = if info.audio_channels > 0 { Some(MovieAudio::new(info.audio_sample_rate, info.audio_channels)?) } else { None };
        let mut playback = Self { decoder, info, audio, frames: VecDeque::new(),
            video_decoded_to: 0.0, audio_decoded_to: 0.0, eof: false, playing_since: None,
            paused_position: 0.0, audio_tail_since: None, decoded_video_frames: 0, decoded_audio_bytes: 0 };
        playback.fill(0.35)?;
        Ok(playback)
    }
    fn fill(&mut self, horizon: f64) -> Result<(), String> {
        while !self.eof && (self.video_decoded_to < horizon || (self.audio.is_some() && self.audio_decoded_to < horizon)) {
            let Some((kind, pts, duration, bytes)) = self.decoder.read()? else { self.eof = true; break; };
            match kind {
                1 => {
                    self.video_decoded_to = pts + duration;
                    self.frames.push_back(VideoFrame { pts, pixels: bytes });
                    self.decoded_video_frames += 1;
                }
                2 => {
                    self.audio_decoded_to = pts + duration;
                    self.decoded_audio_bytes += bytes.len() as u64;
                    if let Some(audio) = self.audio.as_mut() { audio.queue(&bytes)?; }
                }
                _ => return Err("Unknown native movie sample kind".into()),
            }
        }
        Ok(())
    }
    pub fn resume(&mut self) {
        if self.playing_since.is_some() { return; }
        self.playing_since = Some(Instant::now());
        if let Some(audio) = self.audio.as_mut() { audio.resume(); }
        self.audio_tail_since = None;
    }
    pub fn pause(&mut self) {
        self.paused_position = self.position();
        self.playing_since = None;
        if let Some(audio) = self.audio.as_mut() { audio.pause(); }
    }
    pub fn position(&mut self) -> f64 {
        let Some(started) = self.playing_since else { return self.paused_position; };
        if let Some(audio) = self.audio.as_mut() {
            let (position, stopped) = audio.position();
            if self.eof && stopped {
                return position + self.audio_tail_since.get_or_insert_with(Instant::now).elapsed().as_secs_f64();
            }
            self.audio_tail_since = None;
            position
        } else { self.paused_position + started.elapsed().as_secs_f64() }
    }
    pub fn update(&mut self) -> Result<(Option<VideoFrame>, bool), String> {
        if self.playing_since.is_none() { return Ok((None, false)); }
        let position = self.position();
        self.fill(position + 0.35)?;
        let mut frame = None;
        while self.frames.front().is_some_and(|frame| frame.pts <= position + 0.001) {
            frame = self.frames.pop_front();
        }
        let duration = if self.info.duration_100ns > 0 { self.info.duration_100ns as f64 / 10_000_000.0 }
            else { self.video_decoded_to.max(self.audio_decoded_to) };
        Ok((frame, self.eof && position >= duration))
    }
    pub fn duration(&self) -> f64 { self.info.duration_100ns as f64 / 10_000_000.0 }
}
