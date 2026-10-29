// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
use super::{Window,Event,FingerId,Coords};
use crate::desktop;
use sdl2::{event::Event as S,keyboard::Keycode as K,controller::{Axis,Button},mouse::MouseButton};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration,Instant};

/// The original in-game option owns controller pitch inversion. Only apply
/// the host keyboard/mouse preference before combining those input sources.
pub(super) fn flight_axes(config:desktop::Config,controller:Coords,keyboard:Coords,mouse:Coords)->Coords {
    let (mut x,mut y)=controller;
    let (kx,mut ky)=keyboard;
    let (mx,mut my)=mouse;
    if config.invert_pitch {ky=-ky;my=-my;}
    if x==0.0&&kx==0.0 {x=mx;}
    if y==0.0&&ky==0.0 {y=my;}
    ((x+kx).clamp(-1.0,1.0),(y+ky).clamp(-1.0,1.0))
}
pub(super) fn controller_axis(axis:i16,deadzone:f32)->f32 {
    let axis=(axis as f32/32767.0).clamp(-1.0,1.0);
    ((axis.abs().max(deadzone)-deadzone)/(1.0-deadzone)).copysign(axis)
}
fn look_keys(keys:&HashMap<K,(bool,Instant)>)->Coords {
    let active=|key|f32::from(u8::from(held(keys.get(&key))));
    (active(K::Kp6)-active(K::Kp4),active(K::Kp2)-active(K::Kp8))
}

/// SDL2's normal Windows path drops most virtual-key-only messages (notably
/// accessibility/remote keyboards that supply no hardware scan code). Feed
/// those messages through the same native bindings, leaving physical keys to
/// SDL's normal keyboard processing.
pub(super) unsafe fn install_keyboard_compat(window:*mut sdl2_sys::SDL_Window) {
    type Hook=unsafe extern "C" fn(*mut std::ffi::c_void,*mut std::ffi::c_void,u32,u64,i64);
    extern "C" {fn SDL_SetWindowsMessageHook(callback:Option<Hook>,userdata:*mut std::ffi::c_void);}
    unsafe extern "C" fn hook(userdata:*mut std::ffi::c_void,_hwnd:*mut std::ffi::c_void,msg:u32,vk:u64,lp:i64) {
        if !matches!(msg,0x100|0x101|0x104|0x105)||((lp>>16)&255)!=0{return;}
        let code=match vk {
            0x41..=0x5a=>(vk as i32)+32,
            0x70..=0x7b=>K::F1.into_i32()+(vk as i32-0x70),
            0x62=>K::Kp2.into_i32(),0x64=>K::Kp4.into_i32(),0x66=>K::Kp6.into_i32(),0x68=>K::Kp8.into_i32(),
            0x0d=>K::Return.into_i32(),0x1b=>K::Escape.into_i32(),0x20=>K::Space.into_i32(),0x09=>K::Tab.into_i32(),
            0x08=>K::Backspace.into_i32(),0x25=>K::Left.into_i32(),0x26=>K::Up.into_i32(),0x27=>K::Right.into_i32(),0x28=>K::Down.into_i32(),
            0x10|0xa0=>K::LShift.into_i32(),0xa1=>K::RShift.into_i32(),0x11|0xa2=>K::LCtrl.into_i32(),0xa3=>K::RCtrl.into_i32(),_=>return
        };
        let down=matches!(msg,0x100|0x104);
        let mut event:sdl2_sys::SDL_Event=std::mem::zeroed();
        event.key.type_=if down{sdl2_sys::SDL_EventType::SDL_KEYDOWN as u32}else{sdl2_sys::SDL_EventType::SDL_KEYUP as u32};
        event.key.windowID=sdl2_sys::SDL_GetWindowID(userdata.cast());
        event.key.timestamp=sdl2_sys::SDL_GetTicks();event.key.state=u8::from(down);
        event.key.repeat=u8::from(down&&(lp&(1<<30))!=0);
        event.key.keysym.sym=code;event.key.keysym.scancode=sdl2_sys::SDL_GetScancodeFromKey(code);
        event.key.keysym.mod_=sdl2_sys::SDL_GetModState() as u16;
        if lp&(1<<29)!=0 {event.key.keysym.mod_|=sdl2::keyboard::Mod::LALTMOD.bits();}
        sdl2_sys::SDL_PushEvent(&mut event);
    }
    SDL_SetWindowsMessageHook(Some(hook),window.cast());
}

pub(super) struct Input {
    pub focused:bool,
    pub(super) rumble:super::rumble::Output,
    keys:HashMap<K,(bool,Instant)>,
    pressed_keys:HashSet<K>,
    touches:HashMap<u8,Coords>,
    menu_tap:Option<(Coords,Instant)>,
    menu_drag:Option<(Coords,Coords,Instant)>,
    buttons:HashMap<(u32,Button),(bool,Instant)>,
    mouse_buttons:HashMap<MouseButton,(bool,Instant)>,
    mouse_captured:bool,
    mouse_delta:Coords,
    mouse_axes:Coords,
    last_update:Instant,
    was_flight:bool,
    modern:bool,
    menu_axis:(i32,i32),
}
impl Default for Input {
    fn default()->Self {Self {focused:true,rumble:Default::default(),keys:HashMap::new(),pressed_keys:HashSet::new(),touches:HashMap::new(),menu_tap:None,menu_drag:None,buttons:HashMap::new(),mouse_buttons:HashMap::new(),mouse_captured:false,mouse_delta:(0.0,0.0),mouse_axes:(0.0,0.0),last_update:Instant::now(),was_flight:false,modern:true,menu_axis:(0,0)}}
}
fn held(value:Option<&(bool,Instant)>)->bool {value.is_some_and(|&(down,until)|down||Instant::now()<until)}
fn set_held<T:std::hash::Hash+Eq>(map:&mut HashMap<T,(bool,Instant)>,key:T,down:bool) {
    if down {map.insert(key,(true,Instant::now()+Duration::from_millis(100)));}
    else if let Some(value)=map.get_mut(&key){value.0=false;}
}
// Original clModeAction::eBTN values, merged by the native workButton hook.
const MISSILE:u32=1; const GUN:u32=2; const TARGET:u32=4; const THROTTLE:u32=8;
const BRAKE:u32=16; const VIEW:u32=256; const LEVEL:u32=512;
const YAW_RIGHT:u32=4096; const YAW_LEFT:u32=8192;
const PAUSE:u32=0x40000; const RADAR:u32=0x80000;
fn key_action(k:K,modern:bool)->u32 {
    match k {
        K::Q=>YAW_LEFT,K::E=>YAW_RIGHT,K::LCtrl|K::RCtrl=>BRAKE,
        K::LShift|K::RShift=>THROTTLE,K::Return=>MISSILE,K::J=>GUN,
        K::Space=>if modern{THROTTLE}else{GUN},
        K::L=>if modern{MISSILE}else{LEVEL},K::G=>LEVEL,K::T=>TARGET,
        K::Tab=>if modern{RADAR}else{TARGET},K::M=>RADAR,K::V=>VIEW,
        K::Escape|K::F1=>PAUSE,_=>0,
    }
}
fn pad_action(b:Button)->u32 {
    match b {Button::A=>GUN,Button::B=>MISSILE,Button::Y=>TARGET,
        Button::LeftShoulder|Button::DPadLeft=>YAW_LEFT,
        Button::RightShoulder|Button::DPadRight=>YAW_RIGHT,
        Button::Start=>PAUSE,Button::Back=>RADAR,Button::RightStick=>VIEW,
        Button::LeftStick=>LEVEL,_=>0}
}
fn menu_key(k:K)->K {match k {K::W=>K::Up,K::S=>K::Down,K::A=>K::Left,K::D=>K::Right,K::Backspace=>K::Escape,K::F1=>K::Return,_=>k}}
fn mouse_axis(previous:f32,delta:f32,dt:f32,sensitivity:f32)->f32 {
    let dt=dt.clamp(0.001,0.1);
    let target=(delta*sensitivity/(dt*900.0)).clamp(-1.0,1.0);
    let value=previous+(target-previous)*(1.0-(-dt/0.04).exp());
    if value.abs()<0.001 {0.0}else{value}
}
impl Input {
    fn held(&self,k:K)->bool {held(self.keys.get(&k))}
    fn flight_buttons(&self,active:bool,modern:bool)->u32 {
        if !active||!self.focused{return 0;}
        let mut mask=0;
        for (&key,value) in &self.keys {if held(Some(value)){mask|=key_action(key,modern);}}
        for (&(_,button),value) in &self.buttons {if held(Some(value)){mask|=pad_action(button);}}
        if self.mouse_captured {for (&button,value) in &self.mouse_buttons {if held(Some(value)){mask|=match button{MouseButton::Left=>GUN,MouseButton::Right=>MISSILE,MouseButton::Middle=>TARGET,_=>0};}}}
        mask
    }
    pub fn mouse_axes(&self)->Coords {if self.mouse_captured{self.mouse_axes}else{(0.0,0.0)}}
    pub fn axes(&self)->(f32,f32) {
        if !desktop::in_flight()||!self.focused{return (0.0,0.0);}
        let x=i32::from(self.held(K::D)||self.held(K::Right))-i32::from(self.held(K::A)||self.held(K::Left));
        let y=i32::from(self.held(K::S)||self.held(K::Down))-i32::from(self.held(K::W)||self.held(K::Up));
        (x as f32,y as f32)
    }
}

/// Cancel both delivered and not-yet-delivered input without dropping releases
/// from an earlier cancellation. Focus loss, F4 and unplug can occur together.
fn cancel_touches(queue: &mut VecDeque<Event>, delivered: &mut HashMap<FingerId, Coords>) {
    queue.retain(|e| !matches!(e, Event::TouchesDown(_) | Event::TouchesMove(_)));
    let points = std::mem::take(delivered);
    if !points.is_empty() {
        queue.push_back(Event::TouchesUp(points));
    }
}

impl Window {
    pub(super) fn release_native_touches(&mut self) {
        self.stop_controller_rumble();
        cancel_touches(&mut self.event_queue, &mut self.delivered_touches);
        self.desktop_input.keys.clear();self.desktop_input.touches.clear();self.desktop_input.menu_tap=None;self.desktop_input.menu_drag=None;
        self.desktop_input.buttons.clear();self.desktop_input.mouse_buttons.clear();
        self.desktop_input.mouse_delta=(0.0,0.0);self.desktop_input.mouse_axes=(0.0,0.0);
        self.desktop_input.menu_axis=(0,0);desktop::set_flight_buttons(0);desktop::set_look_axes(0.0,0.0,false);
        if self.desktop_input.mouse_captured {self._sdl_ctx.mouse().set_relative_mouse_mode(false);self.desktop_input.mouse_captured=false;}
        self.dpad_state=super::DpadState{left:false,right:false,up:false,down:false,active:false};
        self.stick_active=false;self.virtual_accelerometer_last=None;
        if let Some((x,y,_,_))=self.virtual_cursor_last{self.virtual_cursor_last=Some((x,y,false,false));}
    }
    fn native_touch_coords(&self,(x,y):Coords)->Coords {
        let (w,h)=super::size_for_orientation(self.device_family,self.device_orientation,std::num::NonZeroU32::new(1).unwrap());
        let [x,y]=self.rotation_matrix().inverse().unwrap().transform([x/w as f32-0.5,y/h as f32-0.5]);
        let (uw,uh)=self.size_unrotated_unscaled();((x+0.5)*uw as f32,(y+0.5)*uh as f32)
    }
    fn native_menu_key(&mut self,key:K) {
        if desktop::is_hangar()&&matches!(key,K::Left|K::Right) {
            let end=if key==K::Right{(80.0,140.0)}else{(400.0,140.0)};
            self.desktop_input.menu_drag=Some(((240.0,140.0),end,Instant::now()));return;
        }
        match key {
            K::Up=>{desktop::navigate(0,-1);}, K::Down=>{desktop::navigate(0,1);},
            K::Left=>{desktop::navigate(-1,0);},K::Right=>{desktop::navigate(1,0);},
            K::Return|K::Space|K::Escape=>{
                if let Some(point)=desktop::confirm(key==K::Escape) {
                    self.desktop_input.menu_tap=Some((point,Instant::now()+Duration::from_millis(100)));
                }
            },_=>{}
        }
    }
    pub(super) fn native_event(&mut self,event:&S)->bool {
        if self.desktop_input.focused {
            match *event {
                S::ControllerButtonDown{which,..}=>self.desktop_input.rumble.select(which),
                S::ControllerAxisMotion{which,value,..} if i32::from(value).abs()>8000=>self.desktop_input.rumble.select(which),
                _=>{}
            }
        }
        match *event {
            S::KeyDown{keycode:Some(k),keymod,repeat,..}|S::KeyUp{keycode:Some(k),keymod,repeat,..}=>{
                let down=matches!(event,S::KeyDown{..});
                // A scan-code-free arrow can be accepted by both SDL and our
                // Windows compatibility hook. Only one initial press may act;
                // retain SDL-marked OS repeats and clear on the first release.
                if down {
                    if !self.desktop_input.pressed_keys.insert(k) && !repeat { return true; }
                } else { self.desktop_input.pressed_keys.remove(&k); }
                // SDL's scan-code-free compatibility events do not get its
                // normal Windows close handling. Use our normal quit path.
                if k==K::F4 && keymod.intersects(sdl2::keyboard::Mod::LALTMOD|sdl2::keyboard::Mod::RALTMOD) {
                    if down&&!repeat {self.stop_controller_rumble();self.event_queue.push_back(Event::Quit);}
                    return true;
                }
                let was_open=desktop::menu_open();
                if desktop::settings_key(k,down,repeat){if !was_open&&desktop::menu_open(){self.release_native_touches();}return true;}
                if !self.desktop_input.focused {return true;}
                let mapped=matches!(k,K::W|K::A|K::S|K::D|K::Up|K::Down|K::Left|K::Right|K::Backspace|K::F1|K::Kp2|K::Kp4|K::Kp6|K::Kp8)||key_action(k,desktop::config().modern_controls)!=0;
                if !mapped{return false;}
                if !repeat {set_held(&mut self.desktop_input.keys,k,down);}
                if down&&!desktop::in_flight(){let k=menu_key(k);if !repeat||matches!(k,K::Up|K::Down|K::Left|K::Right){self.native_menu_key(k);}}
                true
            }
            S::ControllerButtonDown{which,button,..}|S::ControllerButtonUp{which,button,..}=>{
                let down=matches!(event,S::ControllerButtonDown{..});
                if !self.desktop_input.focused {return true;}
                set_held(&mut self.desktop_input.buttons,(which,button),down);
                let chord_partner=match button{Button::Start=>Some(Button::Back),Button::Back=>Some(Button::Start),_=>None};
                if down&&chord_partner.is_some_and(|other|self.desktop_input.buttons.get(&(which,other)).is_some_and(|s|s.0)) {
                    desktop::settings_key(K::F4,true,false);self.release_native_touches();return true;
                }
                let was_open=desktop::menu_open();
                if desktop::controller_menu(button,down){if !was_open&&desktop::menu_open(){self.release_native_touches();}return true;}
                if !desktop::in_flight(){
                    // Flight-only buttons must not click unrelated menu rows.
                    // Keep right-stick events available to the virtual cursor.
                    let key=match button {Button::DPadUp=>K::Up,Button::DPadDown=>K::Down,Button::DPadLeft=>K::Left,Button::DPadRight=>K::Right,Button::A|Button::Start=>K::Return,Button::B|Button::Back=>K::Escape,_=>return button!=Button::RightStick};
                    if down{self.native_menu_key(key);}return true;
                }true
            }
            S::ControllerAxisMotion{axis,value,..} if !desktop::in_flight()&&!desktop::menu_open()&&self.desktop_input.focused=>{
                let direction=if value>16000{1}else if value< -16000{-1}else{0};
                let old=match axis {Axis::LeftX=>&mut self.desktop_input.menu_axis.0,Axis::LeftY=>&mut self.desktop_input.menu_axis.1,_=>return false};
                let changed=*old!=direction;*old=direction;
                if changed&&direction!=0 {self.native_menu_key(match (axis,direction){(Axis::LeftX,-1)=>K::Left,(Axis::LeftX,_)=>K::Right,(_, -1)=>K::Up,_=>K::Down});}
                true
            }
            S::ControllerAxisMotion{..} if desktop::in_flight()=>true,
            S::MouseButtonDown{x,y,..} if desktop::menu_open()=>{
                let (vx,vy,vw,vh)=self.viewport();desktop::settings_click((x as f32-vx as f32)/vw as f32*960.0,(y as f32-vy as f32)/vh as f32*640.0);true
            }
            S::MouseButtonUp{..}|S::MouseMotion{..}|S::FingerDown{..}|S::FingerUp{..}|S::FingerMotion{..} if desktop::menu_open()=>true,
            S::MouseButtonDown{mouse_btn:MouseButton::Left,x,y,..} if desktop::in_flight()&&desktop::config().mouse_flight&&!self.desktop_input.mouse_captured=>{
                let (vx,vy,vw,vh)=self.viewport();
                let (x,y)=((x as f32-vx as f32)/vw as f32,(y as f32-vy as f32)/vh as f32);
                if !desktop::config().touch_buttons||((0.3..0.7).contains(&x)&&(0.3..0.7).contains(&y)) {
                    self._sdl_ctx.mouse().set_relative_mouse_mode(true);
                    self.desktop_input.mouse_captured=self._sdl_ctx.mouse().relative_mouse_mode();
                    self.desktop_input.mouse_delta=(0.0,0.0);self.desktop_input.mouse_axes=(0.0,0.0);return true;
                }false
            }
            S::MouseButtonDown{mouse_btn,..}|S::MouseButtonUp{mouse_btn,..} if self.desktop_input.mouse_captured=>{
                set_held(&mut self.desktop_input.mouse_buttons,mouse_btn,matches!(event,S::MouseButtonDown{..}));true
            }
            S::MouseMotion{xrel,yrel,..} if self.desktop_input.mouse_captured=>{
                self.desktop_input.mouse_delta.0+=xrel as f32;self.desktop_input.mouse_delta.1+=yrel as f32;true
            }
            S::MouseWheel{..} if self.desktop_input.mouse_captured=>true,
            S::ControllerAxisMotion{..} if desktop::menu_open()||!self.desktop_input.focused=>true,
            S::Window{win_event:sdl2::event::WindowEvent::FocusLost,..}=>{self.desktop_input.focused=false;self.desktop_input.pressed_keys.clear();self.release_native_touches();false},
            S::Window{win_event:sdl2::event::WindowEvent::FocusGained,..}=>{self.desktop_input.focused=true;false},
            _=>false
        }
    }
    pub(super) fn update_native_input(&mut self) {
        let config=desktop::config();
        // ANGLE owns swapchain recreation. Keep its EGLSurface alive while
        // changing the native window, including fullscreen transitions.
        if self.fullscreen != config.fullscreen {
            let mode = if config.fullscreen { sdl2::video::FullscreenType::Desktop }
                else { sdl2::video::FullscreenType::Off };
            self.vulkan.clone().resize_window(|| self.window.set_fullscreen(mode))
                .expect("Changing Vulkan window mode failed");
            self.fullscreen = config.fullscreen;
        }
        if !self.fullscreen {
            let (width, height) = desktop::SIZES[config.resolution];
            if self.window.size() != (width, height) {
                self.vulkan.clone().resize_window(|| self.window.set_size(width, height).map_err(|e| e.to_string()))
                    .expect("Changing Vulkan window size failed");
            }
        }
        let flying=desktop::in_flight()&&self.desktop_input.focused;
        if self.desktop_input.was_flight!=flying||self.desktop_input.modern!=config.modern_controls {
            self.release_native_touches();self.desktop_input.was_flight=flying;self.desktop_input.modern=config.modern_controls;
        }
        if !config.mouse_flight&&self.desktop_input.mouse_captured {self.release_native_touches();}
        let now=Instant::now();let dt=now.duration_since(self.desktop_input.last_update).as_secs_f32();self.desktop_input.last_update=now;
        let (dx,dy)=std::mem::replace(&mut self.desktop_input.mouse_delta,(0.0,0.0));
        self.desktop_input.mouse_axes=(mouse_axis(self.desktop_input.mouse_axes.0,dx,dt,config.mouse_sensitivity),mouse_axis(self.desktop_input.mouse_axes.1,dy,dt,config.mouse_sensitivity));
        let mut desired=HashMap::new();let mut mask=self.desktop_input.flight_buttons(flying,config.modern_controls);
        let (mut look_x,mut look_y)=look_keys(&self.desktop_input.keys);
        if flying {
            for controller in &self.controllers {
                if controller.axis(Axis::TriggerRight)>4096 {mask|=THROTTLE;}
                if controller.axis(Axis::TriggerLeft)>4096 {mask|=BRAKE;}
                look_x+=controller_axis(controller.axis(Axis::RightX),config.deadzone);
                look_y+=controller_axis(controller.axis(Axis::RightY),config.deadzone);
            }
        }
        desktop::set_look_axes(look_x,look_y,flying);
        desktop::set_flight_buttons(mask);
        let motors=super::rumble::output(flying,config.controller_vibration,now);
        self.desktop_input.rumble.update(&mut self.controllers,motors,now);
        if !desktop::menu_open() {
            if let Some((point,until))=self.desktop_input.menu_tap {if Instant::now()<until{desired.insert(7,point);}else{self.desktop_input.menu_tap=None;}}
            if let Some((start,end,since))=self.desktop_input.menu_drag {
                let elapsed=since.elapsed().as_secs_f32();
                if elapsed<0.26 {let t=(elapsed/0.18).min(1.0);desired.insert(8,(start.0+(end.0-start.0)*t,start.1+(end.1-start.1)*t));}
                else {self.desktop_input.menu_drag=None;}
            }
        }
        let old=std::mem::replace(&mut self.desktop_input.touches,desired.clone());
        for (&id,&point) in &old {if !desired.contains_key(&id){let p=self.native_touch_coords(point);self.event_queue.push_back(Event::TouchesUp(HashMap::from([(FingerId::Desktop(id),p)])));}}
        for (&id,&point) in &desired {
            let kind=if !old.contains_key(&id){0}else if old.get(&id)!=Some(&point){1}else{continue;};
            let p=self.native_touch_coords(point);let map=HashMap::from([(FingerId::Desktop(id),p)]);
            self.event_queue.push_back(if kind==0{Event::TouchesDown(map)}else{Event::TouchesMove(map)});
        }
    }
    pub(super) fn stop_controller_rumble(&mut self) {
        super::rumble::cancel();
        self.desktop_input.rumble.stop(&mut self.controllers);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_stick_deadzone_and_keyboard_do_not_generate_flight_buttons() {
        assert_eq!(controller_axis(i16::MIN,0.15),-1.0);
        assert_eq!(controller_axis(i16::MAX,0.15),1.0);
        assert_eq!(controller_axis(4000,0.15),0.0);
        let mut keys=HashMap::new();
        set_held(&mut keys,K::Kp6,true);set_held(&mut keys,K::Kp8,true);
        assert_eq!(look_keys(&keys),(1.0,-1.0));
        for k in [K::Kp2,K::Kp4,K::Kp6,K::Kp8] {assert_eq!(key_action(k,true),0);}
        keys.clear();assert_eq!(look_keys(&keys),(0.0,0.0));
    }

    #[test]
    fn host_inversion_never_reverses_controller_pitch() {
        let c=desktop::Config::default();
        assert_eq!(flight_axes(c,(0.4,0.6),(0.0,0.0),(0.0,0.0)),(0.4,0.6));
        assert_eq!(flight_axes(c,(0.0,0.5),(0.0,0.25),(0.0,1.0)),(0.0,0.75));
        let c=desktop::Config {invert_pitch:true,..Default::default()};
        assert_eq!(flight_axes(c,(0.4,0.6),(0.0,0.0),(0.0,0.0)),(0.4,0.6));
        assert_eq!(flight_axes(c,(0.0,0.0),(0.4,0.6),(0.0,0.0)),(0.4,-0.6));
        assert_eq!(flight_axes(c,(0.0,0.0),(0.0,0.0),(0.4,0.6)),(0.4,-0.6));
    }

    #[test]
    fn modern_layout_does_not_mix_legacy_space_and_tab_actions() {
        let mut input=Input::default();
        for key in [K::Space,K::Tab,K::L,K::Q,K::J]{set_held(&mut input.keys,key,true);}
        assert_eq!(input.flight_buttons(true,true),THROTTLE|RADAR|MISSILE|YAW_LEFT|GUN);
        assert_eq!(input.flight_buttons(true,false),GUN|TARGET|LEVEL|YAW_LEFT);
        assert_eq!(input.flight_buttons(false,true),0);
        input.focused=false;
        assert_eq!(input.flight_buttons(true,true),0);
    }

    #[test]
    fn releasing_one_controller_does_not_release_another_or_keyboard() {
        let mut input=Input::default();
        let expired=Instant::now()-Duration::from_secs(1);
        input.buttons.insert((1,Button::A),(true,expired));
        input.buttons.insert((2,Button::A),(true,expired));
        input.keys.insert(K::Space,(true,expired));
        set_held(&mut input.buttons,(1,Button::A),false);
        assert_eq!(input.flight_buttons(true,true),GUN|THROTTLE);
        set_held(&mut input.buttons,(2,Button::A),false);
        assert_eq!(input.flight_buttons(true,true),THROTTLE);
    }

    #[test]
    fn quick_tap_survives_until_original_thirty_hz_game_can_sample_it() {
        let mut input=Input::default();
        set_held(&mut input.keys,K::L,true);
        set_held(&mut input.keys,K::L,false);
        assert_eq!(input.flight_buttons(true,true),MISSILE);
        input.keys.get_mut(&K::L).unwrap().1=Instant::now()-Duration::from_secs(1);
        assert_eq!(input.flight_buttons(true,true),0);
    }

    #[test]
    fn mouse_clicks_only_fire_when_captured_and_can_combine_with_pad() {
        let mut input=Input::default();
        set_held(&mut input.mouse_buttons,MouseButton::Left,true);
        set_held(&mut input.mouse_buttons,MouseButton::Right,true);
        set_held(&mut input.buttons,(1,Button::RightShoulder),true);
        assert_eq!(input.flight_buttons(true,true),YAW_RIGHT);
        input.mouse_captured=true;
        assert_eq!(input.flight_buttons(true,true),YAW_RIGHT|GUN|MISSILE);
        assert_eq!(input.flight_buttons(false,true),0);
    }

    #[test]
    fn mouse_velocity_is_frame_rate_independent_and_returns_to_neutral() {
        let sample=|hz:u32|{let mut value=0.0;for _ in 0..hz{value=mouse_axis(value,300.0/hz as f32,1.0/hz as f32,1.0);}value};
        assert!((sample(30)-sample(60)).abs()<0.001);
        assert!((sample(60)-sample(120)).abs()<0.001);
        let mut value=sample(60);
        for _ in 0..60 {value=mouse_axis(value,0.0,1.0/60.0,1.0);}
        assert_eq!(value,0.0);
        assert!((0.0..=1.0).contains(&mouse_axis(0.0,10000.0,0.0,3.0)));
        assert!((-1.0..=0.0).contains(&mouse_axis(0.0,-10000.0,1.0,3.0)));
    }

    #[test]
    fn disconnect_releases_every_delivered_input_source() {
        let fingers = [FingerId::Desktop(2), FingerId::ButtonToTouch(crate::options::Button::A),
            FingerId::DpadToTouch, FingerId::VirtualCursor, FingerId::Mouse];
        let mut delivered: HashMap<_, _> = fingers.into_iter().map(|f| (f, (10.0, 20.0))).collect();
        let expected = delivered.clone();
        let mut queue = VecDeque::new();
        cancel_touches(&mut queue, &mut delivered);
        assert!(delivered.is_empty());
        match queue.pop_front().unwrap() {
            Event::TouchesUp(points) => assert_eq!(points, expected),
            other => panic!("Expected releases, got {other:?}"),
        }
        assert!(queue.is_empty());
    }

    #[test]
    fn repeated_focus_menu_disconnect_keeps_release_and_quit() {
        let finger = FingerId::ButtonToTouch(crate::options::Button::B);
        let mut delivered = HashMap::from([(finger, (3.0, 4.0))]);
        let mut queue = VecDeque::from([
            Event::TouchesDown(HashMap::from([(FingerId::Desktop(0), (1.0, 2.0))])),
            Event::TouchesMove(delivered.clone()), Event::Quit,
        ]);
        cancel_touches(&mut queue, &mut delivered);
        cancel_touches(&mut queue, &mut delivered);
        cancel_touches(&mut queue, &mut delivered);
        assert!(matches!(queue.pop_front(), Some(Event::Quit)));
        assert!(matches!(queue.pop_front(), Some(Event::TouchesUp(points)) if points.contains_key(&finger)));
        assert!(queue.is_empty());
    }

    #[test]
    fn cancellation_preserves_a_release_already_waiting_for_uikit() {
        let mut queue = VecDeque::from([Event::TouchesUp(HashMap::from([
            (FingerId::Desktop(3), (440.0, 260.0)),
        ]))]);
        cancel_touches(&mut queue, &mut HashMap::new());
        assert_eq!(queue.len(), 1);
        assert!(matches!(queue.front(), Some(Event::TouchesUp(_))));
    }
}
