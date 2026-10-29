/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Native MP4 movies: software decoding, OpenAL and Vulkan-backed UIKit.

use super::native_movie::{MovieInfo, Playback};
use crate::dyld::{ConstantExports, HostConstant};
use crate::frameworks::core_graphics::cg_affine_transform::CGAffineTransform;
use crate::frameworks::core_graphics::cg_image::{self, CGImageRelease};
use crate::frameworks::core_graphics::{CGPoint, CGRect, CGSize};
use crate::frameworks::foundation::{ns_string, ns_url, NSInteger, NSTimeInterval};
use crate::frameworks::uikit::ui_device::UIDeviceOrientation;
use crate::image::Image;
use crate::objc::{id, msg, msg_class, msg_super, nil, objc_classes, release, retain,
    ClassExports, HostObject, NSZonePtr};
use crate::window::DeviceOrientation;
use crate::Environment;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

struct Notification {
    name: &'static str,
    player: id,
    due: Instant,
    finish_reason: Option<i32>,
}
#[derive(Default)]
pub struct State {
    active_player: Option<id>,
    pending_notifications: VecDeque<Notification>,
    view_players: HashMap<id, id>,
    controller_players: HashMap<id, id>,
}
impl State {
    fn get(env: &mut Environment) -> &mut Self { &mut env.framework_state.media_player.movie_player }
}
type MPMovieScalingMode = NSInteger;
type MPMovieControlStyle = NSInteger;
type MPMoviePlaybackState = NSInteger;
const STOPPED: MPMoviePlaybackState = 0;
const PLAYING: MPMoviePlaybackState = 1;
const PAUSED: MPMoviePlaybackState = 2;
const FINISHED: i32 = 0;
const ERROR: i32 = 1;
const USER_EXITED: i32 = 2;

pub const MPMoviePlayerPlaybackDidFinishNotification: &str = "MPMoviePlayerPlaybackDidFinishNotification";
pub const MPMoviePlayerContentPreloadDidFinishNotification: &str = "MPMoviePlayerContentPreloadDidFinishNotification";
pub const MPMoviePlayerScalingModeDidChangeNotification: &str = "MPMoviePlayerScalingModeDidChangeNotification";
const MPMoviePlayerPlaybackDidFinishReasonUserInfoKey: &str = "MPMoviePlayerPlaybackDidFinishReasonUserInfoKey";
pub const CONSTANTS: ConstantExports = &[
    ("_MPMoviePlayerPlaybackDidFinishNotification", HostConstant::NSString(MPMoviePlayerPlaybackDidFinishNotification)),
    ("_MPMoviePlayerContentPreloadDidFinishNotification", HostConstant::NSString(MPMoviePlayerContentPreloadDidFinishNotification)),
    ("_MPMoviePlayerScalingModeDidChangeNotification", HostConstant::NSString(MPMoviePlayerScalingModeDidChangeNotification)),
    ("_MPMoviePlayerPlaybackDidFinishReasonUserInfoKey", HostConstant::NSString(MPMoviePlayerPlaybackDidFinishReasonUserInfoKey)),
];

struct MPMoviePlayerControllerHostObject {
    content_url: id,
    view: id,
    image_view: id,
    overlay_window: id,
    previous_key_window: id,
    playback: Option<Playback>,
    info: MovieInfo,
    playback_state: MPMoviePlaybackState,
    scaling_mode: MPMovieScalingMode,
    control_style: MPMovieControlStyle,
    fullscreen: bool,
    orientation: Option<DeviceOrientation>,
    previous_orientation: Option<DeviceOrientation>,
    initial_time: NSTimeInterval,
}
impl Default for MPMoviePlayerControllerHostObject {
    fn default() -> Self {
        Self { content_url: nil, view: nil, image_view: nil, overlay_window: nil, previous_key_window: nil,
            playback: None, info: MovieInfo::default(), playback_state: STOPPED, scaling_mode: 1,
            control_style: 0, fullscreen: true, orientation: None, previous_orientation: None, initial_time: 0.0 }
    }
}
impl HostObject for MPMoviePlayerControllerHostObject {}

fn queue_notification(env: &mut Environment, player: id, name: &'static str, reason: Option<i32>) {
    if State::get(env).pending_notifications.iter().any(|n| n.player == player && n.name == name && n.finish_reason == reason) { return; }
    retain(env, player);
    State::get(env).pending_notifications.push_back(Notification { name, player, due: Instant::now(), finish_reason: reason });
}
fn prepare(env: &mut Environment, player: id) -> Result<(), String> {
    if env.objc.borrow::<MPMoviePlayerControllerHostObject>(player).playback.is_some() { return Ok(()); }
    let url = env.objc.borrow::<MPMoviePlayerControllerHostObject>(player).content_url;
    if url == nil { return Err("Movie has no content URL".into()); }
    let path = ns_url::to_rust_path(env, url);
    let bytes = env.fs.read(path.as_ref()).map_err(|error| format!("Reading movie {path:?}: {error:?}"))?;
    let playback = if let Some(hd) = crate::hd_assets::movie(&bytes) {
        match Playback::open(&hd) {
            Ok(p) => { log!("HD movie replacement loaded"); p }
            Err(e) => { log!("HD movie decode failed: {e}; using original"); Playback::open(&bytes)? }
        }
    } else { Playback::open(&bytes)? };
    let info = playback.info;
    log!("Native movie prepared: {}x{}, {}/{}, {:.3}s, {} Hz / {} channels (software decode, Vulkan display)",
        info.width, info.height, info.frame_rate_num, info.frame_rate_den, playback.duration(), info.audio_sample_rate, info.audio_channels);
    let object = env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(player);
    object.info = info;
    object.playback = Some(playback);
    queue_notification(env, player, MPMoviePlayerContentPreloadDidFinishNotification, None);
    Ok(())
}
fn ensure_view(env: &mut Environment, player: id) -> id {
    let existing = env.objc.borrow::<MPMoviePlayerControllerHostObject>(player).view;
    if existing != nil { return existing; }
    let screen: id = msg_class![env; UIScreen mainScreen];
    let bounds: CGRect = msg![env; screen bounds];
    let view: id = msg_class![env; _ACXiMovieTouchView alloc];
    let view: id = msg![env; view initWithFrame:bounds];
    let black: id = msg_class![env; UIColor blackColor];
    () = msg![env; view setBackgroundColor:black];
    () = msg![env; view setOpaque:true];
    let image_view: id = msg_class![env; UIImageView alloc];
    let image_view: id = msg![env; image_view initWithFrame:bounds];
    () = msg![env; image_view setUserInteractionEnabled:false];
    () = msg![env; view addSubview:image_view];
    let object = env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(player);
    object.view = view;
    object.image_view = image_view;
    State::get(env).view_players.insert(view, player);
    view
}
fn layout_movie(env: &mut Environment, player: id) {
    let object = env.objc.borrow::<MPMoviePlayerControllerHostObject>(player);
    let (view, image_view, info, mode, overlay) = (object.view, object.image_view, object.info, object.scaling_mode, object.overlay_window != nil);
    if view == nil || image_view == nil || info.width == 0 || info.height == 0 { return; }
    let bounds: CGRect = msg![env; view bounds];
    let rotation = if overlay {
        match env.window.as_ref().unwrap().current_rotation() {
            DeviceOrientation::LandscapeLeft => std::f32::consts::FRAC_PI_2,
            DeviceOrientation::LandscapeRight => -std::f32::consts::FRAC_PI_2,
            DeviceOrientation::PortraitUpsideDown => std::f32::consts::PI,
            DeviceOrientation::Portrait => 0.0,
        }
    } else { 0.0 };
    let landscape = rotation.abs() == std::f32::consts::FRAC_PI_2;
    let (available_w, available_h) = if landscape { (bounds.size.height, bounds.size.width) } else { (bounds.size.width, bounds.size.height) };
    let (width, height) = (info.width as f32, info.height as f32);
    let (sx, sy) = match mode {
        0 => (1.0, 1.0),
        2 => { let scale = (available_w / width).max(available_h / height); (scale, scale) }
        3 => (available_w / width, available_h / height),
        _ => { let scale = (available_w / width).min(available_h / height); (scale, scale) }
    };
    let native_bounds = CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width, height } };
    () = msg![env; image_view setBounds:native_bounds];
    let center = CGPoint { x: bounds.origin.x + bounds.size.width / 2.0, y: bounds.origin.y + bounds.size.height / 2.0 };
    () = msg![env; image_view setCenter:center];
    let (sin, cos) = rotation.sin_cos();
    let transform = CGAffineTransform { a: cos * sx, b: sin * sx, c: -sin * sy, d: cos * sy, tx: 0.0, ty: 0.0 };
    () = msg![env; image_view setTransform:transform];
}
fn show_movie(env: &mut Environment, player: id) {
    let view = ensure_view(env, player);
    let object = env.objc.borrow::<MPMoviePlayerControllerHostObject>(player);
    let (orientation, fullscreen) = (object.orientation, object.fullscreen);
    if let Some(orientation) = orientation {
        let previous = env.window.as_ref().unwrap().current_rotation();
        env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(player).previous_orientation = Some(previous);
        env.window.as_mut().unwrap().rotate_device(orientation);
    }
    let superview: id = msg![env; view superview];
    if superview == nil && fullscreen {
        let screen: id = msg_class![env; UIScreen mainScreen];
        let bounds: CGRect = msg![env; screen bounds];
        () = msg![env; view setFrame:bounds];
        let window: id = msg_class![env; UIWindow alloc];
        let window: id = msg![env; window initWithFrame:bounds];
        let previous = env.framework_state.uikit.ui_view.ui_window.key_window.unwrap_or(nil);
        retain(env, previous);
        let object = env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(player);
        object.overlay_window = window;
        object.previous_key_window = previous;
        () = msg![env; window addSubview:view];
        () = msg![env; window makeKeyAndVisible];
    }
    () = msg![env; view setHidden:false];
    layout_movie(env, player);
}
fn hide_movie(env: &mut Environment, player: id) {
    let object = env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(player);
    let window = std::mem::replace(&mut object.overlay_window, nil);
    let previous = std::mem::replace(&mut object.previous_key_window, nil);
    let orientation = object.previous_orientation.take();
    let view = object.view;
    if window != nil {
        () = msg![env; window setHidden:true];
        () = msg![env; view removeFromSuperview];
        release(env, window);
        if previous != nil { () = msg![env; previous makeKeyWindow]; }
    }
    release(env, previous);
    if let Some(orientation) = orientation { env.window.as_mut().unwrap().rotate_device(orientation); }
}
fn finish(env: &mut Environment, player: id, reason: i32) {
    let object = env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(player);
    let was_active = object.playback_state != STOPPED;
    object.playback_state = STOPPED;
    let playback = object.playback.take();
    if let Some(ref playback) = playback {
        log!("Native movie stop: reason {}, decoded {} video frames / {} audio bytes", reason,
            playback.decoded_video_frames, playback.decoded_audio_bytes);
    }
    drop(playback);
    hide_movie(env, player);
    if was_active { queue_notification(env, player, MPMoviePlayerPlaybackDidFinishNotification, Some(reason)); }
    if State::get(env).active_player == Some(player) {
        State::get(env).active_player = None;
        release(env, player);
    }
}

pub const CLASSES: ClassExports = objc_classes! {
(env, this, _cmd);

@implementation MPMoviePlayerController: NSObject
+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::<MPMoviePlayerControllerHostObject>::default(), &mut env.mem)
}
- (id)initWithContentURL:(id)url {
    retain(env, url);
    env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).content_url = url;
    if let Err(error) = prepare(env, this) { log!("Native movie prepare failed: {}", error); }
    this
}
- (())dealloc {
    hide_movie(env, this);
    let object = env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this);
    let (url, view, image_view) = (object.content_url, object.view, object.image_view);
    drop(object.playback.take());
    State::get(env).view_players.remove(&view);
    release(env, image_view);
    release(env, view);
    release(env, url);
    env.objc.dealloc_object(this, &mut env.mem);
}
- (id)contentURL { env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).content_url }
- (())setContentURL:(id)url {
    finish(env, this, USER_EXITED);
    retain(env, url);
    let old = std::mem::replace(&mut env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).content_url, url);
    release(env, old);
    if let Err(error) = prepare(env, this) { log!("Native movie prepare failed: {}", error); }
}
- (id)backgroundColor { msg_class![env; UIColor blackColor] }
- (())setBackgroundColor:(id)color {
    let view = ensure_view(env, this);
    () = msg![env; view setBackgroundColor:color];
}
- (())setScalingMode:(MPMovieScalingMode)mode {
    env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).scaling_mode = mode;
    layout_movie(env, this);
    queue_notification(env, this, MPMoviePlayerScalingModeDidChangeNotification, None);
}
- (MPMovieScalingMode)scalingMode { env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).scaling_mode }
- (())setUseApplicationAudioSession:(bool)_use_session {}
- (())setControlStyle:(MPMovieControlStyle)style { env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).control_style = style; }
- (())setMovieControlMode:(NSInteger)style { env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).control_style = style; }
- (())setMovieSourceType:(NSInteger)_source_type {}
- (())prepareToPlay { if let Err(error) = prepare(env, this) { log!("Native movie prepare failed: {}", error); } }
- (())prepare { if let Err(error) = prepare(env, this) { log!("Native movie prepare failed: {}", error); } }
- (())setFullscreen:(bool)fullscreen { env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).fullscreen = fullscreen; }
- (())setFullscreen:(bool)fullscreen animated:(bool)_animated { env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).fullscreen = fullscreen; }
- (bool)isFullscreen { env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).fullscreen }
- (())setInitialPlaybackTime:(NSTimeInterval)time {
    env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).initial_time = time;
    if time != 0.0 { log!("Native movie nonzero initial playback time is not yet supported: {}", time); }
}
- (NSTimeInterval)initialPlaybackTime { env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).initial_time }
- (NSTimeInterval)duration { env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).info.duration_100ns as f64 / 10_000_000.0 }
- (NSTimeInterval)currentPlaybackTime {
    env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).playback.as_mut().map_or(0.0, |playback| playback.position())
}
- (id)view { ensure_view(env, this) }
- (MPMoviePlaybackState)playbackState { env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).playback_state }
- (())setOrientation:(UIDeviceOrientation)orientation animated:(bool)_animated {
    env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this).orientation = match orientation {
        1 => Some(DeviceOrientation::Portrait), 2 => Some(DeviceOrientation::PortraitUpsideDown),
        3 => Some(DeviceOrientation::LandscapeLeft), 4 => Some(DeviceOrientation::LandscapeRight), _ => None,
    };
}
- (())play {
    if env.objc.borrow::<MPMoviePlayerControllerHostObject>(this).playback_state == PLAYING { return; }
    if let Some(old) = State::get(env).active_player {
        if old != this { finish(env, old, USER_EXITED); }
    }
    if let Err(error) = prepare(env, this) {
        log!("Native movie playback failed: {}", error);
        queue_notification(env, this, MPMoviePlayerPlaybackDidFinishNotification, Some(ERROR));
        return;
    }
    if State::get(env).active_player != Some(this) {
        retain(env, this);
        State::get(env).active_player = Some(this);
        show_movie(env, this);
    }
    let object = env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this);
    object.playback.as_mut().unwrap().resume();
    object.playback_state = PLAYING;
    log!("Native movie playing with original timestamps and PCM soundtrack");
}
- (())pause {
    let object = env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(this);
    if object.playback_state == PLAYING {
        if let Some(playback) = object.playback.as_mut() { playback.pause(); }
        object.playback_state = PAUSED;
    }
}
- (())stop { finish(env, this, USER_EXITED); }
@end

@implementation _ACXiMovieTouchView: UIView
- (())touchesBegan:(id)_touches withEvent:(id)_event {
    if let Some(player) = State::get(env).view_players.get(&this).copied() { finish(env, player, USER_EXITED); }
}
@end

@implementation MPMoviePlayerViewController: UIViewController
- (id)initWithContentURL:(id)url {
    let this: id = msg_super![env; this initWithNibName:nil bundle:nil];
    let player: id = msg_class![env; MPMoviePlayerController alloc];
    let player: id = msg![env; player initWithContentURL:url];
    () = msg![env; player setFullscreen:false];
    let view: id = msg![env; player view];
    () = msg![env; this setView:view];
    State::get(env).controller_players.insert(this, player);
    this
}
- (id)moviePlayer { State::get(env).controller_players.get(&this).copied().unwrap_or(nil) }
- (())viewDidAppear:(bool)_animated {
    let player = State::get(env).controller_players.get(&this).copied().unwrap_or(nil);
    if player != nil { () = msg![env; player play]; }
}
- (())viewWillDisappear:(bool)_animated {
    let player = State::get(env).controller_players.get(&this).copied().unwrap_or(nil);
    if player != nil { () = msg![env; player stop]; }
}
- (bool)shouldAutorotateToInterfaceOrientation:(NSInteger)_orientation { true }
- (())dealloc {
    let player = State::get(env).controller_players.remove(&this).unwrap_or(nil);
    if player != nil { () = msg![env; player stop]; release(env, player); }
    msg_super![env; this dealloc]
}
@end
};

pub(super) fn handle_players(env: &mut Environment) {
    if let Some(player) = State::get(env).active_player {
        let playing = env.objc.borrow::<MPMoviePlayerControllerHostObject>(player).playback_state == PLAYING;
        if playing {
            let mut playback = env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(player).playback.take().unwrap();
            let result = playback.update();
            let info = playback.info;
            env.objc.borrow_mut::<MPMoviePlayerControllerHostObject>(player).playback = Some(playback);
            match result {
                Ok((frame, done)) => {
                    if let Some(frame) = frame {
                        let image = Image::from_pixel_vec(frame.pixels, (info.width, info.height));
                        let cg = cg_image::from_image(env, image);
                        let image: id = msg_class![env; UIImage alloc];
                        let image: id = msg![env; image initWithCGImage:cg];
                        CGImageRelease(env, cg);
                        let image_view = env.objc.borrow::<MPMoviePlayerControllerHostObject>(player).image_view;
                        () = msg![env; image_view setImage:image];
                        release(env, image);
                    }
                    if done { finish(env, player, FINISHED); }
                }
                Err(error) => { log!("Native movie decode/playback error: {}", error); finish(env, player, ERROR); }
            }
        }
    }
    let now = Instant::now();
    let mut ready = Vec::new();
    let pending = &mut State::get(env).pending_notifications;
    let mut index = 0;
    while index < pending.len() {
        if pending[index].due <= now { ready.push(pending.remove(index).unwrap()); } else { index += 1; }
    }
    for notification in ready {
        let name = ns_string::get_static_str(env, notification.name);
        let center: id = msg_class![env; NSNotificationCenter defaultCenter];
        let user_info = if let Some(reason) = notification.finish_reason {
            let value: id = msg_class![env; NSNumber numberWithInt:reason];
            let key = ns_string::get_static_str(env, MPMoviePlayerPlaybackDidFinishReasonUserInfoKey);
            msg_class![env; NSDictionary dictionaryWithObject:value forKey:key]
        } else { nil };
        () = msg![env; center postNotificationName:name object:(notification.player) userInfo:user_info];
        release(env, notification.player);
    }
}
