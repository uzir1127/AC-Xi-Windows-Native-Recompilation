// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
//! Windows port settings and native menu navigation. Guest physics stay intact.
use crate::font::{Font, TextAlignment};
use sdl2::keyboard::Keycode;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const SIZES: [(u32,u32);5] = [(480,320),(960,640),(1280,720),(1920,1080),(2560,1440)];
const SETTINGS_FILE: &str = "acxi-settings.ini";
const SETTINGS_ROWS: usize = 14;
const ROW_HEIGHT: usize = 25;
const ANISOTROPY_LEVELS: [u32;5] = [1,2,4,8,16];
// Store the pair together so readers never see a partially updated setting.
// This is separate from State: overlay() already holds the settings mutex.
static ACTIVE_GRAPHICS: AtomicU64 = AtomicU64::new((2u64 << 32) | 4);
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config { pub resolution: usize, pub render_scale: u32, pub anisotropy: u32, pub hd_assets: bool, pub fullscreen: bool, pub touch_buttons: bool, pub invert_pitch: bool, pub deadzone: f32, pub performance_overlay: bool, pub modern_controls: bool, pub mouse_flight: bool, pub mouse_sensitivity: f32, pub controller_vibration: f32, pub auto_save: bool }
impl Default for Config { fn default()->Self { Self { resolution:1,render_scale:2,anisotropy:4,hd_assets:true,fullscreen:false,touch_buttons:false,invert_pitch:false,deadzone:0.15,performance_overlay:false,modern_controls:true,mouse_flight:true,mouse_sensitivity:1.0,controller_vibration:0.65,auto_save:false } } }
impl Config {
    fn from_settings(text:&str)->Self {
        let mut config=Self::default();
        for line in text.lines() { if let Some((k,v))=line.split_once('=') { match k.trim() {
            "resolution"=>if let Ok(n)=v.trim().parse::<usize>() { if n<SIZES.len(){config.resolution=n;} },
            "render_scale"=>if let Ok(n)=v.trim().parse::<u32>() { if (1..=4).contains(&n){config.render_scale=n;} },
            "anisotropy"=>if let Ok(n)=v.trim().parse::<u32>() { if ANISOTROPY_LEVELS.contains(&n){config.anisotropy=n;} },
            "hd_assets"=>match v.trim(){"true"=>config.hd_assets=true,"false"=>config.hd_assets=false,_=>{}},
            "modern_controls"=>config.modern_controls=v.trim()!="false",
            "mouse_flight"=>config.mouse_flight=v.trim()!="false",
            "auto_save"=>config.auto_save=v.trim()=="true",
            "controller_vibration"=>if let Ok(n)=v.trim().parse::<f32>() {if n.is_finite(){config.controller_vibration=n.clamp(0.0,1.0);}},
            "mouse_sensitivity"=>if let Ok(n)=v.trim().parse::<f32>() {if n.is_finite(){config.mouse_sensitivity=n.clamp(0.25,3.0);}},
            "performance_overlay"=>config.performance_overlay=v.trim()=="true", "fullscreen"=>config.fullscreen=v.trim()=="true", "touch_buttons"=>config.touch_buttons=v.trim()=="true", "invert_pitch"=>config.invert_pitch=v.trim()=="true",
            "deadzone"=>if let Ok(n)=v.trim().parse::<f32>() { if n.is_finite(){config.deadzone=n.clamp(0.0,0.5);} }, _=>{} } } }
        config
    }
    fn settings_text(self)->String {
        format!("resolution={}\nrender_scale={}\nanisotropy={}\nhd_assets={}\nfullscreen={}\ntouch_buttons={}\ninvert_pitch={}\ndeadzone={}\nperformance_overlay={}\nmodern_controls={}\nmouse_flight={}\nmouse_sensitivity={}\ncontroller_vibration={}\nauto_save={}\n",self.resolution,self.render_scale,self.anisotropy,self.hd_assets,self.fullscreen,self.touch_buttons,self.invert_pitch,self.deadzone,self.performance_overlay,self.modern_controls,self.mouse_flight,self.mouse_sensitivity,self.controller_vibration,self.auto_save)
    }
}
struct State { config:Config, menu:bool, row:usize, raster:Option<Arc<Vec<u8>>>, table:Vec<[i32;5]>, table_id:u64, selected:usize, navigation:bool, hangar:bool, modal:bool, close_with_back:bool, table_seen:Option<Instant>, flight_seen:Option<Instant>, message:String }
fn state()-> &'static Mutex<State> {
    static STATE:OnceLock<Mutex<State>>=OnceLock::new();
    STATE.get_or_init(|| {
        let config=std::fs::read_to_string(SETTINGS_FILE).map(|text|Config::from_settings(&text)).unwrap_or_default();
        Mutex::new(State {config,menu:false,row:0,raster:None,table:Vec::new(),table_id:0,selected:0,navigation:false,hangar:false,modal:false,close_with_back:false,table_seen:None,flight_seen:None,message:String::new()})
    })
}
pub fn config()->Config { state().lock().unwrap().config }
/// Called at startup after applying file/command-line options. F4 edits only
/// the saved preference; render targets and filtering keep these active values.
pub fn set_active_graphics(render_scale:u32,anisotropy:u32) {
    ACTIVE_GRAPHICS.store(((render_scale as u64)<<32)|anisotropy as u64,Ordering::Relaxed);
    state().lock().unwrap().raster=None;
}
pub fn active_graphics()->(u32,u32) {
    let packed=ACTIVE_GRAPHICS.load(Ordering::Relaxed);
    ((packed>>32) as u32,packed as u32)
}
pub fn menu_open()->bool { state().lock().unwrap().menu }
fn flight(s:&State)->bool { s.flight_seen.is_some_and(|t|t.elapsed()<Duration::from_millis(350)) }
pub fn in_flight()->bool { let s=state().lock().unwrap(); !s.menu && flight(&s) && !s.table_seen.zip(s.flight_seen).is_some_and(|(menu,flight)|menu>flight) }
static FLIGHT_BUTTONS:AtomicU32=AtomicU32::new(0);
static LOOK_AXES:AtomicU64=AtomicU64::new(0);
pub fn set_look_axes(x:f32,y:f32,enabled:bool) {
    let axis=|v:f32|if v.is_finite(){(v.clamp(-1.0,1.0)*32767.0).round() as i16 as u16 as u64}else{0};
    LOOK_AXES.store(if enabled{axis(x)|(axis(y)<<16)|(1u64<<32)}else{0},Ordering::Relaxed);
}
#[no_mangle]
pub extern "C" fn acxi_native_look_axes()->u64 {LOOK_AXES.load(Ordering::Relaxed)}
pub fn set_flight_buttons(mask:u32) {FLIGHT_BUTTONS.store(mask,Ordering::Relaxed);}
#[no_mangle]
pub extern "C" fn acxi_native_flight_buttons()->u32 {FLIGHT_BUTTONS.load(Ordering::Relaxed)}
fn write_settings(path:&std::path::Path,config:Config)->std::io::Result<()> {
    use std::io::Write;
    // Replace in the same directory only after the complete file is flushed.
    // A failed write/rename leaves the previous settings file intact.
    let mut name=path.as_os_str().to_os_string();
    name.push(format!(".{}.tmp",std::process::id()));
    let temporary=std::path::PathBuf::from(name);
    let result=(|| {
        let mut file=std::fs::File::create(&temporary)?;
        file.write_all(config.settings_text().as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary,path)
    })();
    if result.is_err(){let _=std::fs::remove_file(&temporary);}
    result
}
fn save_settings(s:&mut State) {
    let c=s.config;
    s.message=match write_settings(std::path::Path::new(SETTINGS_FILE),c) {Ok(())=>if active_graphics()!=(c.render_scale,c.anisotropy)||crate::hd_assets::enabled()!=c.hd_assets{"Settings saved. Restart to apply graphics changes.".into()}else{"Settings saved.".into()},Err(e)=>{log!("Settings save failed: {}",e);format!("Save failed: {e}")}};
}
fn should_auto_save(before:Config,after:Config)->bool {after.auto_save || before.auto_save!=after.auto_save}
fn change(s:&mut State,delta:i32) {
    let before=s.config;
    if s.row!=12 {s.message.clear();}
    match s.row {
        0=>s.config.resolution=(s.config.resolution as i32+delta).rem_euclid(SIZES.len() as i32) as usize,
        1=>s.config.render_scale=(s.config.render_scale as i32-1+delta).rem_euclid(4) as u32+1,
        2=>{let i=ANISOTROPY_LEVELS.iter().position(|&n|n==s.config.anisotropy).unwrap_or(2);s.config.anisotropy=ANISOTROPY_LEVELS[(i as i32+delta).rem_euclid(ANISOTROPY_LEVELS.len() as i32) as usize];},
        3=>s.config.hd_assets=!s.config.hd_assets,
        4=>s.config.fullscreen=!s.config.fullscreen,
        5=>s.config.touch_buttons=!s.config.touch_buttons,
        6=>s.config.invert_pitch=!s.config.invert_pitch,
        7=>s.config.performance_overlay=!s.config.performance_overlay,
        8=>s.config.modern_controls=!s.config.modern_controls,
        9=>s.config.mouse_flight=!s.config.mouse_flight,
        10=>s.config.mouse_sensitivity=(s.config.mouse_sensitivity+delta as f32*0.25).clamp(0.25,3.0),
        11=>s.config.auto_save=!s.config.auto_save,
        12=>save_settings(s),
        13=>s.menu=false,_=>{}
    }
    if s.config!=before {
        if should_auto_save(before,s.config) {save_settings(s);}else{s.message="Unsaved changes. Choose Save settings.".into();}
    }
    s.raster=None;
}
pub fn settings_key(key:Keycode,down:bool,repeat:bool)->bool {
    let mut s=state().lock().unwrap();
    if key==Keycode::F5 {if down&&!repeat {s.config.performance_overlay=!s.config.performance_overlay;if s.config.auto_save {save_settings(&mut s);}else{s.message="Unsaved changes. Choose Save settings.".into();}s.raster=None;}return true;}
    if key==Keycode::F4 { if down&&!repeat {s.menu=!s.menu;if s.menu{s.row=0;}s.raster=None;} return true; }
    if !s.menu{return false;}
    if down { match key {
        Keycode::Escape=>s.menu=false,
        Keycode::Up=>s.row=(s.row+SETTINGS_ROWS-1)%SETTINGS_ROWS,Keycode::Down|Keycode::Tab=>s.row=(s.row+1)%SETTINGS_ROWS,
        Keycode::Left=>change(&mut s,-1),Keycode::Right|Keycode::Return|Keycode::Space=>change(&mut s,1),_=>{}
    } s.raster=None; }
    true
}
pub fn settings_click(x:f32,y:f32)->bool {
    let mut s=state().lock().unwrap();if !s.menu{return false;}
    if (76.0..884.0).contains(&x)&&(122.0..(122+SETTINGS_ROWS*ROW_HEIGHT) as f32).contains(&y) {s.row=((y-122.0)/ROW_HEIGHT as f32) as usize;change(&mut s,1);}
    true
}
pub fn controller_menu(button:sdl2::controller::Button,down:bool)->bool {
    use sdl2::controller::Button as B;
    if button==B::Guide { return settings_key(Keycode::F4,down,false); }
    if !menu_open(){return false;}
    let k=match button {B::DPadUp=>Keycode::Up,B::DPadDown=>Keycode::Down,B::DPadLeft=>Keycode::Left,B::DPadRight=>Keycode::Right,B::A=>Keycode::Return,B::B|B::Start=>Keycode::Escape,_=>return true};
    settings_key(k,down,false)
}
/// Called only at the statically identified original mobile-button draw entry.
#[no_mangle]
pub extern "C" fn acxi_draw_touch_buttons()->i32 {
    let mut s=state().lock().unwrap();s.flight_seen=Some(Instant::now());i32::from(s.config.touch_buttons)
}
#[no_mangle]
pub extern "C" fn acxi_draw_phone_hint()->i32 { i32::from(config().touch_buttons) }
/// Original menu rectangles are integer (mask,x,y,width,height) records.
/// The generated caller bounds-checks this range against guest address space.
#[no_mangle]
pub unsafe extern "C" fn acxi_observe_menu_table(ptr:*const u8,count:u32,flags:u32,owner:u32,phase:u32) {
    if (ptr.is_null()&&count>0)||count>128{return;}
    // Vtable addresses are from the inspected, identity-checked 1.1.3 image.
    let hangar=owner==0xfd270;
    let pause=owner==0xfd1c4;
    let modal=owner==0xfcf84&&(5..=14).contains(&phase)||pause&&matches!(phase,6|7|10);
    let mut items=Vec::new();let mut hash=owner as u64;
    for i in 0..count as usize {
        let mut r=[0i32;5];for (j,v) in r.iter_mut().enumerate(){*v=std::ptr::read_unaligned(ptr.add(i*20+j*4).cast());}
        // The failed-mission table contains its parent and dialog buttons
        // together. Navigate only the pair used by the original phase handler.
        if owner==0xfcf84 && if modal {r[0]&12==0} else {r[0]&3==0} {continue;}
        if pause && if modal {r[0]&0x180==0} else {r[0]&0x7f==0} {continue;}
        if r[0]==0||r[1]< -480||r[2]< -320||r[1]>=480||r[2]>=320||r[3]<=0||r[4]<=0||r[3]>960||r[4]>640 {continue;}
        let right=(r[1] as i64+r[3] as i64).min(480) as i32;
        let bottom=(r[2] as i64+r[4] as i64).min(320) as i32;
        r[1]=r[1].max(0);r[2]=r[2].max(0);r[3]=right-r[1];r[4]=bottom-r[2];
        if r[3]<=0||r[4]<=0{continue;}
        for v in r {hash=hash.wrapping_mul(31).wrapping_add(v as u64);}
        items.push(r);
    }
    let close_with_back=pause&&!modal&&flags&0x40000000!=0;
    let prefer_ok=flags&0x40000000!=0&&(hangar||close_with_back||items.iter().all(|r|r[3]*r[4]>=480*160));
    let ok_index=items.len();
    // These are the inspected original clMenuTask global button rectangles.
    // They are enabled by UpdateTouchButton after the local table is tested.
    if flags&0x40000000!=0{items.push([0x40000000,416,0,64,48]);hash=hash.wrapping_mul(31).wrapping_add(0x40000000);}
    if flags&0x80000000!=0{items.push([i32::MIN,0,0,64,48]);hash=hash.wrapping_mul(31).wrapping_add(0x80000000);}
    if items.is_empty(){return;}
    let mut s=state().lock().unwrap();s.table_seen=Some(Instant::now());s.hangar=hangar;s.modal=modal;s.close_with_back=close_with_back;
    let new_ok=flags&0x40000000!=0&&!s.table.iter().any(|r|r[0]==0x40000000);
    if hash!=s.table_id {s.table_id=hash;s.table=items;s.selected=if prefer_ok||new_ok{ok_index}else{0};s.navigation=false;s.raster=None;
        if std::env::var_os("ACXI_TRACE_INPUT").is_some(){log!("Native menu owner {:#x}, phase {}, {} active rectangles, default {}",owner,phase,s.table.len(),s.selected);}
    }
}
pub fn is_hangar()->bool {let s=state().lock().unwrap();s.hangar&&s.table_seen.is_some_and(|t|t.elapsed()<Duration::from_secs(1))}
pub fn navigate(dx:i32,dy:i32)->Option<(f32,f32)> {
    let mut s=state().lock().unwrap(); if s.table.is_empty()||s.table_seen.is_none_or(|t|t.elapsed()>Duration::from_secs(1)){return None;}
    let center=|r:[i32;5]|(r[1] as f32+r[3] as f32/2.0,r[2] as f32+r[4] as f32/2.0);
    let (x,y)=center(s.table[s.selected]);let mut best=None;let mut score=f32::INFINITY;
    for (i,&r) in s.table.iter().enumerate(){let (nx,ny)=center(r);let (a,b)=(nx-x,ny-y);let forward=a*dx as f32+b*dy as f32;
        if forward<=1.0 {continue;}let side=(a*dy as f32-b*dx as f32).abs();let candidate=forward+side*3.0;
        if candidate<score {best=Some(i);score=candidate;}
    }
    if let Some(i)=best{s.selected=i;}s.navigation=true;s.raster=None;Some(center(s.table[s.selected]))
}
pub fn confirm(back:bool)->Option<(f32,f32)> {
    let mut s=state().lock().unwrap();
    if back {
        if s.close_with_back&&s.table_seen.is_some_and(|t|t.elapsed()<Duration::from_secs(1)){return Some((448.0,24.0));}
        if s.table.iter().any(|r|r[0]==i32::MIN){return Some((24.0,20.0));}
        if s.modal&&s.table.len()>=2 {let r=s.table[1];return Some((r[1] as f32+r[3] as f32/2.0,r[2] as f32+r[4] as f32/2.0));}
        return None;
    }
    if !s.table.is_empty()&&s.table_seen.is_some_and(|t|t.elapsed()<Duration::from_secs(1)) {
        s.navigation=true;s.raster=None;let r=s.table[s.selected];return Some((r[1] as f32+r[3] as f32/2.0,r[2] as f32+r[4] as f32/2.0));
    }Some((240.0,150.0))
}
pub fn overlay()->Option<Arc<Vec<u8>>> {
    let mut s=state().lock().unwrap();
    if !s.menu&&(!s.navigation||flight(&s)||s.table_seen.is_none_or(|t|t.elapsed()>Duration::from_secs(1))){return None;}
    if let Some(p)=&s.raster{return Some(p.clone());}
    let mut p=vec![0u8;960*640*4];
    fn rect(p:&mut[u8],x:usize,y:usize,w:usize,h:usize,c:[u8;4]) {for yy in y..(y+h).min(640){for xx in x..(x+w).min(960){p[(yy*960+xx)*4..(yy*960+xx)*4+4].copy_from_slice(&c);}}}
    if s.menu {
        rect(&mut p,0,0,960,640,[5,10,18,224]);rect(&mut p,60,45,840,580,[16,25,37,255]);rect(&mut p,60,45,840,4,[50,205,205,255]);
        let regular=Font::sans_regular();let bold=Font::sans_bold();
        fn text(p:&mut[u8],f:&Font,size:f32,x:f32,y:f32,t:&str,c:[u8;3]) {f.draw(size,t,(x,y),None,TextAlignment::Left,|g|{let (ox,oy)=g.origin();let (w,h)=g.dimensions();for gy in 0..h{for gx in 0..w{let (xx,yy)=(ox.floor() as i32+gx,oy.floor() as i32+gy);if !(0..960).contains(&xx)||!(0..640).contains(&yy){continue;}let a=g.pixel_at((gx,gy));let idx=(yy as usize*960+xx as usize)*4;for j in 0..3{p[idx+j]=(p[idx+j] as f32*(1.0-a)+c[j] as f32*a) as u8;}p[idx+3]=255;}}});}
        text(&mut p,&bold,25.0,86.0,67.0,"ACXi  /  Settings",[235,245,250]);text(&mut p,&regular,15.0,650.0,77.0,"F4 to close  |  Vulkan",[100,219,219]);
        let c=s.config;let (w,h)=SIZES[c.resolution];let on=|v|if v{"On"}else{"Off"};
        let filtering=|n|if n==1{"Original".to_owned()}else{format!("{n}x AF")};
        let rows=[
            ("Window resolution",format!("{w} x {h}")),
            ("Internal resolution",format!("{}x  /  {} x {}",c.render_scale,480*c.render_scale,320*c.render_scale)),
            ("Texture filtering",filtering(c.anisotropy)),
            ("HD textures / text / movie",if c.hd_assets {"Enhanced"}else{"Original"}.into()),
            ("Fullscreen",on(c.fullscreen).into()),
            ("Mobile touch buttons",on(c.touch_buttons).into()),
            ("Keyboard / mouse invert pitch",on(c.invert_pitch).into()),
            ("Performance overlay (F5)",on(c.performance_overlay).into()),
            ("Keyboard layout",if c.modern_controls{"ACX / modern"}else{"Previous ACXi"}.into()),
            ("Mouse flight",on(c.mouse_flight).into()),
            ("Mouse sensitivity",format!("{:.2}x",c.mouse_sensitivity)),
            ("Auto-save settings",on(c.auto_save).into()),
            ("Save settings",String::new()),("Return to game",String::new())
        ];
        for (i,(label,value)) in rows.iter().enumerate(){let y=122+i*ROW_HEIGHT;if s.row==i{rect(&mut p,76,y,808,ROW_HEIGHT-1,[34,63,79,255]);rect(&mut p,76,y,3,ROW_HEIGHT-1,[65,226,212,255]);}text(&mut p,&regular,17.0,95.0,y as f32+4.0,label,[222,233,242]);text(&mut p,&regular,17.0,590.0,y as f32+4.0,value,[108,229,216]);}
        let (active_scale,active_af)=active_graphics();
        let pending=(active_scale,active_af)!=(c.render_scale,c.anisotropy)||crate::hd_assets::enabled()!=c.hd_assets;
        let graphics=if pending {
            format!("Graphics pending: restart to apply. Running: {active_scale}x; AF request: {} (GPU may limit AF).",filtering(active_af))
        } else {
            format!("Running: {active_scale}x; AF request: {}. {} Graphics changes need restart.",filtering(active_af),if c.auto_save{"Auto-save On."}else{"Use Save settings."})
        };
        text(&mut p,&regular,13.0,86.0,484.0,&graphics,if pending{[245,215,125]}else{[155,181,199]});
        text(&mut p,&regular,14.0,86.0,514.0,"Arrows / D-pad: select and change   Enter / A: confirm   Esc / B: close",[155,181,199]);
        let help=if c.modern_controls{"WASD: pitch/roll  Q/E: yaw  Space: throttle  Ctrl: brake  J: gun  L: missile"}else{"WASD: pitch/roll  Q/E: yaw  Shift: throttle  Ctrl: brake  Space: gun  Enter: missile"};
        text(&mut p,&regular,13.0,86.0,542.0,help,[155,181,199]);
        text(&mut p,&regular,13.0,86.0,564.0,"T: target  M: radar  V: view  Esc/Start: pause  View+Start: settings",[155,181,199]);
        text(&mut p,&regular,12.0,86.0,582.0,"Mouse: click flight view to capture; Esc/F4 releases. Left: gun; Right: missile; Middle: target.",[155,181,199]);
        if !s.message.is_empty(){text(&mut p,&regular,13.0,86.0,600.0,&s.message,[130,235,202]);}
    } else if let Some(r)=s.table.get(s.selected).filter(|r|r[3]*r[4]<480*120) {
        let (x,y,w,h)=(r[1].max(0) as usize*2,r[2].max(0) as usize*2,r[3] as usize*2,r[4] as usize*2);
        for (a,b,c,d) in [(x,y,w,3),(x,y+h.saturating_sub(3),w,3),(x,y,3,h),(x+w.saturating_sub(3),y,3,h)] {rect(&mut p,a,b,c,d,[245,225,93,255]);}
    }
    let p=Arc::new(p);s.raster=Some(p.clone());Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_settings_path(label:&str)->std::path::PathBuf {
        std::env::temp_dir().join(format!("acxi-settings-{}-{}-{label}.ini",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()))
    }
    #[test]
    fn mobile_buttons_default_off_but_saved_choice_is_respected() {
        assert!(!Config::default().touch_buttons);
        assert!(!Config::from_settings("").touch_buttons);
        assert!(Config::from_settings("touch_buttons=true").touch_buttons);
        assert!(!Config::from_settings("touch_buttons=false").touch_buttons);
    }
    #[test]
    fn manual_and_automatic_saving_are_separate_and_toggle_persists() {
        let original=Config::default();
        let manual=Config{touch_buttons:true,..original};
        assert!(!original.auto_save);
        assert!(!should_auto_save(original,manual));
        let automatic=Config{auto_save:true,..manual};
        assert!(should_auto_save(manual,automatic));
        assert!(should_auto_save(automatic,Config{touch_buttons:false,..automatic}));
        assert!(should_auto_save(automatic,manual));
    }
    #[test]
    fn settings_replace_existing_file_and_reload_all_fields() {
        let path=test_settings_path("roundtrip");
        write_settings(&path,Config::default()).unwrap();
        for on in [true,false] {
            let c=Config{resolution:4,render_scale:4,anisotropy:16,hd_assets:on,fullscreen:on,touch_buttons:on,invert_pitch:on,deadzone:0.35,performance_overlay:on,modern_controls:on,mouse_flight:on,mouse_sensitivity:2.75,controller_vibration:0.4,auto_save:on};
            write_settings(&path,c).unwrap();
            assert_eq!(Config::from_settings(&std::fs::read_to_string(&path).unwrap()),c);
        }
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    #[cfg(windows)]
    fn failed_settings_replace_preserves_the_previous_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let path=test_settings_path("failure");
        write_settings(&path,Config::default()).unwrap();
        let before=std::fs::read(&path).unwrap();
        // Deny delete-sharing to simulate an externally locked settings file.
        let locked=std::fs::OpenOptions::new().read(true).share_mode(1).open(&path).unwrap();
        let changed=Config{touch_buttons:true,..Config::default()};
        assert!(write_settings(&path,changed).is_err());
        assert_eq!(std::fs::read(&path).unwrap(),before);
        drop(locked);std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn vibration_strength_is_persistent_bounded_and_can_be_disabled() {
        assert_eq!(Config::from_settings("controller_vibration=0").controller_vibration,0.0);
        assert_eq!(Config::from_settings("controller_vibration=5").controller_vibration,1.0);
        assert_eq!(Config::from_settings("controller_vibration=-5").controller_vibration,0.0);
        for value in ["NaN","inf","invalid"] {
            assert_eq!(Config::from_settings(&format!("controller_vibration={value}")).controller_vibration,0.65);
        }
    }

    #[test]
    fn existing_settings_keep_controls_and_get_graphics_defaults() {
        let c=Config::from_settings("resolution=3\nfullscreen=true\ntouch_buttons=false\ninvert_pitch=true\ndeadzone=0.25\nperformance_overlay=true\n");
        assert_eq!((c.render_scale,c.anisotropy),(2,4));
        assert_eq!(c.resolution,3);
        assert!(c.fullscreen&&c.invert_pitch&&c.performance_overlay);
        assert!(!c.touch_buttons);
        assert_eq!(c.deadzone,0.25);
        assert!(c.modern_controls&&c.mouse_flight);
        assert_eq!(c.mouse_sensitivity,1.0);
    }

    #[test]
    fn invalid_graphics_values_and_nonfinite_deadzone_are_ignored() {
        for text in ["render_scale=0\nanisotropy=0\ndeadzone=NaN", "render_scale=5\nanisotropy=3\ndeadzone=inf", "render_scale=-1\nanisotropy=32", "render_scale=4294967296\nanisotropy=garbage"] {
            assert_eq!(Config::from_settings(text),Config::default());
        }
    }

    #[test]
    fn every_graphics_choice_round_trips_with_other_preferences() {
        for render_scale in 1..=4 {for anisotropy in ANISOTROPY_LEVELS {
            let c=Config {render_scale,anisotropy,hd_assets:false,resolution:4,fullscreen:true,touch_buttons:false,invert_pitch:true,deadzone:0.35,performance_overlay:true,modern_controls:false,mouse_flight:false,mouse_sensitivity:1.75,controller_vibration:0.35,auto_save:true};
            assert_eq!(Config::from_settings(&c.settings_text()),c);
        }}
    }

    #[test]
    fn mouse_sensitivity_rejects_nonfinite_values_and_bounds_extreme_values() {
        for value in ["NaN","inf","-inf","invalid"] {
            assert_eq!(Config::from_settings(&format!("mouse_sensitivity={value}")).mouse_sensitivity,1.0);
        }
        assert_eq!(Config::from_settings("mouse_sensitivity=-1").mouse_sensitivity,0.25);
        assert_eq!(Config::from_settings("mouse_sensitivity=999").mouse_sensitivity,3.0);
    }

    #[test]
    fn obsolete_controller_inversion_is_ignored_and_not_saved() {
        let c=Config::from_settings("controller_invert_pitch=true");
        assert_eq!(c,Config::default());
        assert!(!c.settings_text().contains("controller_invert_pitch"));
    }
}
