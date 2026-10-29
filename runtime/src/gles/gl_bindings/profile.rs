// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
//! Opt-in raw-API profiler. No pacing, vsync, resolution or guest state changes.
//! Cycles are reported as cycles/share, never converted into elapsed time.
use std::{cell::{Cell, RefCell}, path::PathBuf, time::{Instant, Duration}, io::Write};
use std::ffi::c_void;

#[repr(C)] #[derive(Default)] struct FileTime {lo:u32, hi:u32}
impl FileTime { fn value(&self)->u64 {(self.hi as u64)<<32|self.lo as u64} }
#[link(name="kernel32")]
extern "system" {
    fn GetCurrentThread()->*mut c_void;
    fn GetCurrentProcess()->*mut c_void;
    fn QueryThreadCycleTime(thread:*mut c_void,cycles:*mut u64)->i32;
    fn GetThreadTimes(thread:*mut c_void,c:*mut FileTime,e:*mut FileTime,k:*mut FileTime,u:*mut FileTime)->i32;
    fn GetProcessTimes(process:*mut c_void,c:*mut FileTime,e:*mut FileTime,k:*mut FileTime,u:*mut FileTime)->i32;
}
fn cycles()->u64 {
    let mut n=0; assert_ne!(unsafe {QueryThreadCycleTime(GetCurrentThread(),&mut n)},0); n
}
fn cpu_time(process:bool)->u64 {
    let (mut c,mut e,mut k,mut u)=(FileTime::default(),FileTime::default(),FileTime::default(),FileTime::default());
    let ok=unsafe {if process {GetProcessTimes(GetCurrentProcess(),&mut c,&mut e,&mut k,&mut u)}
                  else {GetThreadTimes(GetCurrentThread(),&mut c,&mut e,&mut k,&mut u)}};
    assert_ne!(ok,0);k.value()+u.value()
}
#[derive(Clone, Copy)] struct Counter { name:&'static str, calls:u64, samples:u64, ns:u64, cycles:u64, max_ns:u64 }
const EMPTY:Counter=Counter{name:"",calls:0,samples:0,ns:0,cycles:0,max_ns:0};
struct Run {
    label:String, target:u64, start:Instant, last_frame:Instant, cycle_start:u64,
    process_start:u64, thread_start:u64, game:u64,
    total:[Counter;512], current:[Counter;512], rows:Vec<[u64;10]>,
}
#[derive(Default)] struct State {
    folder:Option<PathBuf>, command:String, last_check:Option<Instant>, run:Option<Run>,
    calibration_ns:f64, calibration_cycles:f64,
}
thread_local! {
    static ACTIVE:Cell<bool>=const {Cell::new(false)};
    static STATE:RefCell<State>=RefCell::new(State::default());
    static RANDOM:Cell<u32>=const {Cell::new(0xa7f5d391)};
}
pub struct Scope(Option<(usize,Instant,u64,u64)>);
impl Scope {
    #[inline] pub fn enter(id:usize,name:&'static str)->Self {
        if !ACTIVE.with(Cell::get) {return Self(None)}
        STATE.with(|s| {let mut s=s.borrow_mut();if let Some(r)=s.run.as_mut() {
            for c in [&mut r.total[id],&mut r.current[id]] {c.name=name;c.calls+=1;}
        }});
        // Random selection avoids aliasing with repeated draw/state-call order.
        // Low-frequency potentially blocking calls are always timed.
        let rare=id==401||id==402||id==404||matches!(name,"glReadPixels"|"glFinish"|"glFlush"|"glTexImage2D"|"glTexSubImage2D");
        let selected=rare||RANDOM.with(|rng|{let mut x=rng.get();x^=x<<13;x^=x>>17;x^=x<<5;rng.set(x);x&127==0});
        if !selected {return Self(None)}
        // Cycle-query syscalls sit OUTSIDE the wall-clock interval. Their
        // baseline remains explicit for the independent CPU-cycle estimate.
        let before=cycles();
        Self(Some((id,Instant::now(),before,if rare {1}else{128})))
    }
}
impl Drop for Scope {
    #[inline] fn drop(&mut self) {
        if let Some((id,start,before,weight))=self.0 {
            let ns=start.elapsed().as_nanos() as u64;
            let elapsed_cycles=cycles().saturating_sub(before);
            STATE.with(|s| {let mut s=s.borrow_mut(); if let Some(r)=s.run.as_mut() {
                for c in [&mut r.total[id],&mut r.current[id]] {
                    c.samples+=1;c.ns+=ns*weight;c.cycles+=elapsed_cycles*weight;c.max_ns=c.max_ns.max(ns);
                }
            }});
        }
    }
}
pub fn game_frame() {STATE.with(|s| {if let Some(r)=s.borrow_mut().run.as_mut() {r.game+=1;}});}
fn sum(c:&[Counter])->(u64,u64,u64) {
    c.iter().fold((0,0,0),|(a,b,d),c|(a+c.calls,b+c.ns,d+c.cycles))
}
fn write_run(folder:&std::path::Path,r:Run,cycle_end:u64,process_end:u64,thread_end:u64,cal_ns:f64,cal_cycles:f64) {
    let seconds=r.start.elapsed().as_secs_f64();
    let path=folder.join(&r.label);
    let mut frames=std::fs::File::create(path.with_extension("frames.csv")).unwrap();
    writeln!(frames,"display_frame,game_frames,interval_ns,gl_calls,gl_wall_ns,gl_cycles,swap_wall_ns,swap_cycles,aot_wall_ns,aot_cycles").unwrap();
    for row in &r.rows {writeln!(frames,"{},{},{},{},{},{},{},{},{},{}",row[0],row[1],row[2],row[3],row[4],row[5],row[6],row[7],row[8],row[9]).unwrap();}
    let mut calls=std::fs::File::create(path.with_extension("calls.csv")).unwrap();
    writeln!(calls,"name,calls,wall_ns,thread_cycles,max_sampled_call_ns,samples").unwrap();
    for c in r.total.iter().filter(|c|c.calls>0) {writeln!(calls,"{},{},{},{},{},{}",c.name,c.calls,c.ns,c.cycles,c.max_ns,c.samples).unwrap();}
    let mut meta=std::fs::File::create(path.with_extension("meta.txt")).unwrap();
    writeln!(meta,"label={}\nseconds={}\ndisplay_frames={}\ngame_frames={}\nmain_thread_cycles={}\nprocess_cpu_ms={}\nmain_thread_cpu_ms={}\ncalibration_wall_timer_ns={}\ncalibration_timer_cycles={}\nsampling=1_in_128_random_except_blocking_calls\nclock_cycles_are_not_converted_to_time=true\nraw_api_wall_includes_driver_and_any_waits=true\noriginal_frame_limit_unchanged=true",r.label,seconds,r.rows.len(),r.game,cycle_end-r.cycle_start,(process_end-r.process_start) as f64/10000.0,(thread_end-r.thread_start) as f64/10000.0,cal_ns,cal_cycles).unwrap();
    eprintln!("[angle-profile] completed {}: {:.3}s, {} game frames, {} display frames",r.label,seconds,r.game,r.rows.len());
}
pub fn frame() {
    let now=Instant::now();
    STATE.with(|s| {
        let mut s=s.borrow_mut();
        if let Some(r)=s.run.as_mut() {
            let (n,ns,c)=sum(&r.current[..400]);
            r.rows.push([r.rows.len() as u64,r.game,now.duration_since(r.last_frame).as_nanos() as u64,n,ns,c,r.current[401].ns,r.current[401].cycles,r.current[400].ns,r.current[400].cycles]);
            r.current=[EMPTY;512];r.last_frame=now;
            if r.rows.len() as u64>=r.target {
                ACTIVE.with(|a|a.set(false));
                let ce=cycles();let pe=cpu_time(true);let te=cpu_time(false);
                let r=s.run.take().unwrap();
                write_run(s.folder.as_ref().unwrap(),r,ce,pe,te,s.calibration_ns,s.calibration_cycles);
            }
            return;
        }
        if s.last_check.is_some_and(|t|now.duration_since(t)<Duration::from_secs(1)) {return}
        s.last_check=Some(now);
        if s.folder.is_none() {
            s.folder=std::env::var_os("ACXI_PROFILE_DIR").map(PathBuf::from);
            let Some(ref folder)=s.folder else {return};
            std::fs::create_dir_all(folder).unwrap();
            // Empty measured region: cycle syscalls outside wall interval.
            let mut total_ns=0u64;let mut total_cycles=0u64;
            for _ in 0..100_000 {
                let a=cycles();let instant=Instant::now();
                let elapsed=instant.elapsed().as_nanos() as u64;let b=cycles();
                total_ns+=elapsed;total_cycles+=b-a;
            }
            s.calibration_ns=total_ns as f64/100_000.0;
            s.calibration_cycles=total_cycles as f64/100_000.0;
            eprintln!("[angle-profile] timer calibration {:.1} ns/scope, {:.1} cycles/pair",s.calibration_ns,s.calibration_cycles);
        }
        let Some(ref folder)=s.folder else {return};
        let Ok(command)=std::fs::read_to_string(folder.join("control.txt")) else {return};
        if command==s.command {return}
        s.command=command.clone();
        let words:Vec<_>=command.split_whitespace().collect();
        if words.len()!=2||!words[0].chars().all(|c|c.is_ascii_alphanumeric()||c=='_'||c=='-') {return}
        let Ok(target)=words[1].parse::<u64>() else {return};
        if !(60..=36000).contains(&target) {return}
        let now=Instant::now();
        s.run=Some(Run{label:words[0].into(),target,start:now,last_frame:now,cycle_start:cycles(),process_start:cpu_time(true),thread_start:cpu_time(false),game:0,total:[EMPTY;512],current:[EMPTY;512],rows:Vec::with_capacity(target as usize)});
        ACTIVE.with(|a|a.set(true));
        eprintln!("[angle-profile] started {} for {} display frames",words[0],target);
    });
}
