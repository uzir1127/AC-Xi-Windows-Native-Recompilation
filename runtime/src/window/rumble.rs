// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
//! Event-driven two-motor feedback. Native hooks observe the original game's
//! successful weapon paths and player state; they never alter simulation data.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct Pulse { start: Instant, seconds: f32, low: f32, high: f32 }
impl Pulse {
    fn sample(self, now: Instant) -> (f32, f32) {
        let age = now.saturating_duration_since(self.start).as_secs_f32();
        let gain = (1.0 - (age - 0.02).max(0.0) / self.seconds).clamp(0.0, 1.0);
        (self.low * gain, self.high * gain)
    }
}
#[derive(Default)]
struct Mixer {
    enabled: bool,
    pulses: [Option<Pulse>; 4],
    flight: Option<(Instant, u32, f32)>,
}
fn unit(value: f32) -> f32 { if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 } }
impl Mixer {
    fn enable(&mut self, enabled: bool) {
        if !enabled { *self = Self::default(); }
        self.enabled = enabled;
    }
    fn event(&mut self, kind: u32, amount: f32, now: Instant) {
        if !self.enabled { return; }
        let (seconds, low, high) = match kind {
            1 => (0.075, 0.12, 0.38), // Each actual gun burst refreshes this texture.
            2 => (0.18, 0.55, 0.24),  // Successful missile launch, not a button press.
            3 if unit(amount) > 0.0 => {
                let strength = (0.35 + unit(amount) * 2.0).min(1.0);
                (0.32, 0.9 * strength, 0.65 * strength)
            }
            4 => { self.flight = None; (0.55, 1.0, 0.8) }
            _ => return,
        };
        // A weaker follow-up impact must not erase a stronger active impact.
        let old = self.pulses[(kind - 1) as usize].map(|p| p.sample(now)).unwrap_or_default();
        self.pulses[(kind - 1) as usize] = Some(Pulse { start: now, seconds, low: low.max(old.0), high: high.max(old.1) });
        trace(kind, amount);
    }
    fn frame(&mut self, flags: u32, burner: f32, now: Instant) {
        if self.enabled { self.flight = Some((now, flags, unit(burner))); }
    }
    fn sample(&self, now: Instant, strength: f32) -> (u16, u16) {
        if !self.enabled { return (0, 0); }
        let (mut low, mut high) = (0.0f32, 0.0f32);
        if let Some((stamp, flags, burner)) = self.flight {
            // No sustained engine/stall feedback if updates stop, or after death.
            if now.saturating_duration_since(stamp) < Duration::from_millis(150) && flags & 0xa0 == 0 {
                low = 0.16 * burner; high = 0.035 * burner;
                if flags & 8 != 0 {
                    static ORIGIN: OnceLock<Instant> = OnceLock::new();
                    let phase = now.saturating_duration_since(*ORIGIN.get_or_init(|| now)).as_secs_f32();
                    let buffet = 0.5 + 0.5 * (phase * 9.0 * std::f32::consts::TAU).sin();
                    low = low.max(0.12 + 0.10 * buffet); high = high.max(0.10 + 0.18 * buffet);
                }
            }
        }
        for pulse in self.pulses.iter().flatten() {
            let (l, h) = pulse.sample(now); low = low.max(l); high = high.max(h);
        }
        let scale = unit(strength) * u16::MAX as f32;
        ((unit(low) * scale).round() as u16, (unit(high) * scale).round() as u16)
    }
}
fn mixer() -> &'static Mutex<Mixer> {
    static MIXER: OnceLock<Mutex<Mixer>> = OnceLock::new();
    MIXER.get_or_init(|| Mutex::new(Mixer::default()))
}
fn trace(kind: u32, amount: f32) {
    static TRACE: OnceLock<bool> = OnceLock::new();
    if *TRACE.get_or_init(|| std::env::var_os("ACXI_TRACE_RUMBLE").is_some()) {
        log!("Rumble event {} amount {:.3}", kind, amount);
    }
}
#[no_mangle]
pub extern "C" fn acxi_rumble_event(kind: u32, amount_bits: u32) {
    mixer().lock().unwrap().event(kind, f32::from_bits(amount_bits), Instant::now());
}
#[no_mangle]
pub extern "C" fn acxi_rumble_frame(flags: u32, burner_bits: u32) {
    mixer().lock().unwrap().frame(flags, f32::from_bits(burner_bits), Instant::now());
}
pub fn output(enabled: bool, strength: f32, now: Instant) -> (u16, u16) {
    let mut mixer = mixer().lock().unwrap();
    mixer.enable(enabled && unit(strength) > 0.0);
    mixer.sample(now, strength)
}
pub fn cancel() { mixer().lock().unwrap().enable(false); }

pub(super) trait Pad {
    fn id(&self) -> u32;
    fn rumble(&mut self, low: u16, high: u16, ms: u32) -> Result<(), String>;
}
impl Pad for sdl2::controller::GameController {
    fn id(&self) -> u32 { self.instance_id() }
    fn rumble(&mut self, low: u16, high: u16, ms: u32) -> Result<(), String> {
        self.set_rumble(low, high, ms).map_err(|e| e.to_string())
    }
}
#[derive(Default)]
pub(super) struct Output {
    preferred: Option<u32>,
    target: Option<u32>,
    last: Option<(Instant, (u16, u16))>,
    retry_after: HashMap<u32, Instant>,
}
impl Output {
    pub fn select(&mut self, id: u32) { self.preferred = Some(id); }
    pub fn stop<P: Pad>(&mut self, pads: &mut [P]) {
        if let Some(id) = self.target {
            if let Some(pad) = pads.iter_mut().find(|p| p.id() == id) { let _ = pad.rumble(0, 0, 0); }
        }
        self.target = None; self.last = None;
    }
    pub fn forget(&mut self, id: u32) {
        self.retry_after.remove(&id);
        if self.preferred == Some(id) { self.preferred = None; }
    }
    pub fn update<P: Pad>(&mut self, pads: &mut [P], motors: (u16, u16), now: Instant) {
        let target = self.preferred.filter(|id| pads.iter().any(|p| p.id() == *id))
            .or_else(|| pads.first().map(Pad::id));
        if self.target != target { self.stop(pads); self.target = target; }
        let Some(pad) = pads.iter_mut().find(|p| Some(p.id()) == target) else { return; };
        if self.retry_after.get(&pad.id()).is_some_and(|&deadline| now < deadline) && motors != (0, 0) { return; }
        // At most 25 updates/sec; a stop is immediate. Each nonzero command has
        // a short watchdog duration, so a stalled process cannot leave it running.
        if let Some((stamp, previous)) = self.last {
            if motors == (0, 0) && previous == (0, 0) { return; }
            if motors != (0, 0) && now.saturating_duration_since(stamp) < Duration::from_millis(40) { return; }
        } else if motors == (0, 0) { return; }
        if let Err(error) = pad.rumble(motors.0, motors.1, if motors == (0, 0) { 0 } else { 120 }) {
            if !self.retry_after.contains_key(&pad.id()) {log!("Controller {} vibration unavailable, will retry: {}", pad.id(), error);}
            self.retry_after.insert(pad.id(), now + Duration::from_secs(1));
            self.last = None;
            return;
        }
        self.retry_after.remove(&pad.id());
        self.last = Some((now, motors));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_envelopes_mix_and_expire_without_sticking() {
        let now = Instant::now(); let mut m = Mixer::default(); m.enable(true);
        assert_eq!(m.sample(now, 1.0), (0, 0));
        m.event(3, 0.5, now); let impact = m.sample(now, 1.0);
        m.event(1, 1.0, now); assert_eq!(m.sample(now, 1.0), impact);
        m.event(3, 0.01, now); assert_eq!(m.sample(now, 1.0), impact);
        assert_eq!(m.sample(now + Duration::from_secs(1), 1.0), (0, 0));
        m.event(4, 1.0, now); assert_eq!(m.sample(now, 1.0).0, u16::MAX);
    }
    #[test]
    fn menus_focus_loss_off_and_resume_drop_old_events() {
        let now = Instant::now(); let mut m = Mixer::default();
        m.event(2, 1.0, now); m.enable(true); assert_eq!(m.sample(now, 1.0), (0, 0));
        m.event(2, 1.0, now); assert!(m.sample(now, 1.0).0 > 0);
        m.enable(false); m.event(4, 1.0, now); m.enable(true);
        assert_eq!(m.sample(now, 1.0), (0, 0));
    }
    #[test]
    fn flight_feedback_expires_and_dead_state_stops_engine() {
        let now = Instant::now(); let mut m = Mixer::default(); m.enable(true);
        m.frame(1, 1.0, now); assert!(m.sample(now, 1.0).0 > 0);
        assert_eq!(m.sample(now + Duration::from_millis(151), 1.0), (0, 0));
        m.frame(0x20, 1.0, now); assert_eq!(m.sample(now, 1.0), (0, 0));
        m.frame(8, 0.0, now); assert!(m.sample(now, 1.0).1 > 0);
        m.frame(0, f32::NAN, now); assert_eq!(m.sample(now, 1.0), (0, 0));
    }
    #[test]
    fn strength_and_invalid_events_are_bounded() {
        let now = Instant::now(); let mut m = Mixer::default(); m.enable(true);
        for (kind, value) in [(0, 1.0), (5, 1.0), (3, -1.0), (3, f32::NAN)] { m.event(kind, value, now); }
        assert_eq!(m.sample(now, 1.0), (0, 0));
        m.event(2, 1.0, now);
        assert_eq!(m.sample(now, 0.0), (0, 0)); assert_eq!(m.sample(now, f32::NAN), (0, 0));
        assert!(m.sample(now, 0.5).0 < m.sample(now, 1.0).0);
        assert_eq!(m.sample(now, 9.0), m.sample(now, 1.0));
    }
    struct FakePad { id: u32, fail: bool, calls: Vec<(u16, u16, u32)> }
    impl Pad for FakePad {
        fn id(&self) -> u32 { self.id }
        fn rumble(&mut self, l:u16,h:u16,ms:u32)->Result<(),String> {self.calls.push((l,h,ms));if self.fail{Err("unsupported".into())}else{Ok(())}}
    }
    #[test]
    fn delivery_targets_active_pad_throttles_and_stops_immediately() {
        let now=Instant::now();let mut out=Output::default();
        let mut pads=[FakePad{id:1,fail:false,calls:vec![]},FakePad{id:2,fail:false,calls:vec![]}];
        out.update(&mut pads,(10,20),now);out.update(&mut pads,(30,40),now+Duration::from_millis(1));
        assert_eq!(pads[0].calls,[(10,20,120)]);
        out.update(&mut pads,(0,0),now+Duration::from_millis(2));assert_eq!(pads[0].calls.last(),Some(&(0,0,0)));
        out.select(2);out.update(&mut pads,(30,40),now+Duration::from_millis(3));
        assert_eq!(pads[1].calls,[(30,40,120)]);
        out.update(&mut pads,(30,40),now+Duration::from_millis(44));assert_eq!(pads[1].calls.len(),2);
        out.stop(&mut pads);assert_eq!(pads[1].calls.last(),Some(&(0,0,0)));
    }
    #[test]
    fn unsupported_and_reconnected_controllers_are_handled() {
        let now=Instant::now();let mut out=Output::default();
        let mut pads=[FakePad{id:1,fail:true,calls:vec![]}];
        for i in 0..5 {out.update(&mut pads,(10,20),now+Duration::from_millis(i*100));}
        assert_eq!(pads[0].calls.len(),1);
        out.stop(&mut pads);out.forget(1);pads[0].fail=false;
        out.update(&mut pads,(10,20),now+Duration::from_secs(1));assert_eq!(pads[0].calls.last(),Some(&(10,20,120)));
        out.update::<FakePad>(&mut [],(10,20),now);assert_eq!(out.target,None);
    }
    #[test]
    fn transient_failure_recovers_without_menu_or_reconnect() {
        let now=Instant::now();let mut out=Output::default();
        let mut pads=[FakePad{id:1,fail:true,calls:vec![]}];
        out.update(&mut pads,(100,200),now);
        pads[0].fail=false;
        out.update(&mut pads,(100,200),now+Duration::from_millis(999));
        assert_eq!(pads[0].calls.len(),1);
        out.update(&mut pads,(100,200),now+Duration::from_secs(1));
        assert_eq!(pads[0].calls.len(),2);
        out.update(&mut pads,(0,0),now+Duration::from_millis(1001));
        assert_eq!(pads[0].calls.last(),Some(&(0,0,0)));
    }
    #[test]
    fn real_sdl_cold_start_feedback_stop_and_recovery() {
        #[derive(Default)]
        struct Motors { calls:usize, low:u16, high:u16, fail:bool }
        unsafe extern "C" fn callback(data:*mut std::ffi::c_void,low:u16,high:u16)->i32 {
            let m=&mut *data.cast::<Motors>();m.calls+=1;
            if m.fail{return -1;}m.low=low;m.high=high;0
        }
        let sdl=sdl2::init().unwrap();let controllers=sdl.game_controller().unwrap();
        let mut motors=Box::<Motors>::default();
        let mut desc:sdl2_sys::SDL_VirtualJoystickDesc=unsafe{std::mem::zeroed()};
        desc.version=1;desc.type_=1;desc.naxes=6;desc.nbuttons=16;
        desc.name=b"ACXi rumble regression\0".as_ptr().cast();
        desc.userdata=(&mut *motors as *mut Motors).cast();desc.Rumble=Some(callback);
        let index=unsafe{sdl2_sys::SDL_JoystickAttachVirtualEx(&desc)};
        assert!(index>=0,"{}",sdl2::get_error());
        let mut pads=[controllers.open(index as u32).unwrap()];
        let mut out=Output::default();let mut mixer=Mixer::default();let now=Instant::now();
        // Fresh gameplay enables feedback directly, without an F4 test pulse.
        mixer.enable(true);mixer.event(2,1.0,now);
        out.update(&mut pads,mixer.sample(now,0.65),now);
        assert!(motors.calls>0&&motors.low>0&&motors.high>0);
        out.stop(&mut pads);assert_eq!((motors.low,motors.high),(0,0));
        motors.fail=true;out.update(&mut pads,(1234,5678),now);
        let calls=motors.calls;motors.fail=false;
        out.update(&mut pads,(1234,5678),now+Duration::from_millis(999));assert_eq!(motors.calls,calls);
        out.update(&mut pads,(1234,5678),now+Duration::from_secs(1));
        assert_eq!((motors.low,motors.high),(1234,5678));
        out.stop(&mut pads);assert_eq!((motors.low,motors.high),(0,0));
        drop(pads);assert_eq!(unsafe{sdl2_sys::SDL_JoystickDetachVirtual(index)},0);
    }
}
