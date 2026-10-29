// ACXi port additions, 2026-10-29. SPDX-License-Identifier: MPL-2.0
//! Presentation cadence, independent of game timing and graphics state.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct FrameStats {
    last: Option<Instant>,
    intervals: VecDeque<(Instant, f64)>,
}
#[derive(Default, Debug)]
pub struct Reading { pub fps: f64, pub mean_ms: f64, pub p99_ms: f64 }
impl FrameStats {
    pub fn record(&mut self, now: Instant) {
        if let Some(last) = self.last {
            let delta = now.saturating_duration_since(last);
            // A stopped game/movie transition starts a fresh cadence sample.
            if delta > Duration::from_secs(1) { self.intervals.clear(); }
            else if !delta.is_zero() { self.intervals.push_back((now, delta.as_secs_f64())); }
        }
        self.last = Some(now);
        while self.intervals.front().is_some_and(|(at,_)| now.saturating_duration_since(*at) > Duration::from_secs(1)) || self.intervals.len() > 512 {
            self.intervals.pop_front();
        }
    }
    pub fn read(&self, now: Instant) -> Reading {
        if self.last.is_none_or(|last| now.saturating_duration_since(last) > Duration::from_secs(1)) { return Reading::default(); }
        let mut values: Vec<f64> = self.intervals.iter().filter(|(at,_)| now.saturating_duration_since(*at) <= Duration::from_secs(1)).map(|(_,dt)| *dt).collect();
        if values.is_empty() { return Reading::default(); }
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        values.sort_by(f64::total_cmp);
        let index = ((values.len() as f64 * 0.99).ceil() as usize).saturating_sub(1);
        Reading { fps: 1.0 / mean, mean_ms: mean * 1000.0, p99_ms: values[index] * 1000.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn distinct_game_and_display_cadence() {
        let start=Instant::now(); let mut game=FrameStats::default(); let mut display=FrameStats::default();
        for i in 0..=120 { let now=start+Duration::from_secs_f64(i as f64/60.0); display.record(now); if i%2==0 {game.record(now);} }
        let now=start+Duration::from_secs(2);
        assert!((game.read(now).fps-30.0).abs()<0.001);
        assert!((display.read(now).fps-60.0).abs()<0.001);
        assert!((display.read(now).mean_ms-1000.0/60.0).abs()<0.001);
    }
    #[test] fn stalled_frames_are_included_in_percentile() {
        let start=Instant::now(); let mut stats=FrameStats::default(); let mut now=start;stats.record(now);
        for _ in 0..30 {now+=Duration::from_millis(16);stats.record(now);}
        now+=Duration::from_millis(90);stats.record(now);
        let reading=stats.read(now);assert_eq!(reading.p99_ms,90.0);assert!(reading.fps<62.5);
    }
    #[test] fn inactivity_does_not_report_stale_fps() {
        let start=Instant::now();let mut stats=FrameStats::default();stats.record(start);stats.record(start+Duration::from_millis(16));
        assert_eq!(stats.read(start+Duration::from_secs(3)).fps,0.0);
        stats.record(start+Duration::from_secs(3));assert_eq!(stats.read(start+Duration::from_secs(3)).fps,0.0);
        stats.record(start+Duration::from_secs(3)+Duration::from_millis(20));assert_eq!(stats.read(start+Duration::from_secs(3)+Duration::from_millis(20)).fps,50.0);
    }
    #[test] fn duplicate_timestamps_and_empty_samples_are_safe() {
        let now=Instant::now();let mut stats=FrameStats::default();assert_eq!(stats.read(now).fps,0.0);
        stats.record(now);stats.record(now);assert_eq!(stats.read(now).fps,0.0);
    }
}
