// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
//! F5 host telemetry. Counters are sampled off the renderer; no guest writes.
use crate::font::{Font, TextAlignment};
use crate::frame_stats::FrameStats;
use std::ffi::{c_char, c_void, CStr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[repr(C)]
struct Hardware {
    cpu_name: [c_char;128], gpu_name: [c_char;256],
    cpu_percent: f64, gpu_percent: f64, cpu_valid: bool, gpu_valid: bool,
    sampled_at: u64, ram_total: u64, ram_used: u64, process_bytes: u64, api_version: u32,
}
extern "C" {
    fn acxi_hardware_init(physical:*mut c_void,instance:*mut c_void,get_proc:*mut c_void);
    fn acxi_hardware_visible(visible:bool);
    fn acxi_hardware_read(out:*mut Hardware);
    fn acxi_hardware_shutdown();
}
pub const WIDTH: u32=480;
pub const HEIGHT: u32=132;
struct State {
    game:FrameStats, display:FrameStats, visible:bool, renderer:String,
    refreshed:Option<Instant>, pixels:Option<Arc<Vec<u8>>>, font:Option<Font>,
}
fn state()-> &'static Mutex<State> {
    static STATE:OnceLock<Mutex<State>>=OnceLock::new();
    STATE.get_or_init(||Mutex::new(State {game:FrameStats::default(),display:FrameStats::default(),visible:false,renderer:String::new(),refreshed:None,pixels:None,font:None}))
}
pub fn renderer(renderer:&str,physical:*mut c_void,instance:*mut c_void,get_proc:*mut c_void) {
    unsafe {acxi_hardware_init(physical,instance,get_proc);}
    let mut s=state().lock().unwrap();s.renderer=renderer.into();s.refreshed=None;
}
pub fn game_frame() {
    #[cfg(feature = "profile-angle")]
    touchHLE_gl_bindings::profile::game_frame();
    state().lock().unwrap().game.record(Instant::now());
}
pub fn display_frame() {state().lock().unwrap().display.record(Instant::now());}
pub fn shutdown() {unsafe {acxi_hardware_shutdown();}}

pub fn overlay()->Option<Arc<Vec<u8>>> {
    let visible=crate::desktop::config().performance_overlay;
    let now=Instant::now();let mut s=state().lock().unwrap();
    if s.visible!=visible {
        s.visible=visible;s.refreshed=None;s.pixels=None;
        unsafe {acxi_hardware_visible(visible);}
        log!("F5 performance overlay {}",if visible{"shown"}else{"hidden"});
    }
    if !visible {return None;}
    if s.refreshed.is_some_and(|t|now.duration_since(t)<Duration::from_millis(250)) {return s.pixels.clone();}
    let mut hardware:Hardware=unsafe {std::mem::zeroed()};
    unsafe {acxi_hardware_read(&mut hardware);}
    let name=|ptr:*const c_char|unsafe {CStr::from_ptr(ptr)}.to_string_lossy().trim().to_string();
    let cpu=name(hardware.cpu_name.as_ptr()).replace("(R)","").replace("(TM)","").replace(" CPU","");
    let gpu=name(hardware.gpu_name.as_ptr());
    // The API comes from the verified active renderer, never GL_VERSION's
    // GLES compatibility frontend. Its version comes from the Vulkan device.
    let api=if s.renderer.contains("Vulkan") {
        let v=hardware.api_version;
        if v!=0 {format!("Vulkan {}.{}.{}",(v>>22)&127,(v>>12)&1023,v&4095)}else{"Vulkan".into()}
    }else if s.renderer.contains("Direct3D11"){ "D3D11".into() }
    else if s.renderer.contains("Direct3D12"){ "D3D12".into() }
    else if s.renderer.contains("Metal"){ "Metal".into() }
    else {"Unknown API".into()};
    let usage=|valid:bool,value:f64|if valid&&value.is_finite(){format!("{:.0}%",value.clamp(0.0,100.0))}else{"N/A".into()};
    let game=s.game.read(now);let display=s.display.read(now);
    let rows=[api,
        format!("Game {:5.1} FPS  |  Display {:5.1} FPS",game.fps,display.fps),
        format!("Frame {:5.2} ms  |  p99 {:5.2} ms",display.mean_ms,display.p99_ms),
        format!("CPU {}  {}",cpu,usage(hardware.cpu_valid,hardware.cpu_percent)),
        format!("GPU {}  {}",gpu,usage(hardware.gpu_valid,hardware.gpu_percent)),
        if hardware.ram_total!=0 {format!("RAM {:.1}/{:.1} GiB  |  App {} MiB",hardware.ram_used as f64/1073741824.0,hardware.ram_total as f64/1073741824.0,hardware.process_bytes/1048576)}else{"RAM / App memory: N/A".into()}];
    if s.font.is_none(){s.font=Some(Font::mono_regular());}
    let font=s.font.as_ref().unwrap();
    let mut pixels=vec![0u8;(WIDTH*HEIGHT*4) as usize];
    for pixel in pixels.chunks_exact_mut(4){pixel.copy_from_slice(&[8,13,19,232]);}
    for x in 0..WIDTH as usize {pixels[x*4..x*4+4].copy_from_slice(&[67,225,205,255]);}
    for (row,text) in rows.iter().enumerate() {
        let color=if row==0 {[100,236,218]}else{[235,242,249]};
        // Leave room for long hardware brands; clipping cannot overlap rows.
        let text=if text.chars().count()>55 {format!("{}...",text.chars().take(52).collect::<String>())}else{text.clone()};
        font.draw(12.8,&text,(9.0,8.0+row as f32*19.0),None,TextAlignment::Left,|glyph|{
            let (ox,oy)=glyph.origin();let (w,h)=glyph.dimensions();
            for gy in 0..h {for gx in 0..w {
                let (x,y)=(ox.floor() as i32+gx,oy.floor() as i32+gy);
                if !(4..WIDTH as i32-4).contains(&x)||!(2..HEIGHT as i32-2).contains(&y){continue;}
                let a=glyph.pixel_at((gx,gy));let i=(y as usize*WIDTH as usize+x as usize)*4;
                for j in 0..3 {pixels[i+j]=(pixels[i+j] as f32*(1.0-a)+color[j] as f32*a) as u8;}
                pixels[i+3]=(232.0+(255.0-232.0)*a) as u8;
            }}
        });
    }
    let pixels=Arc::new(pixels);s.pixels=Some(pixels.clone());s.refreshed=Some(now);Some(pixels)
}
