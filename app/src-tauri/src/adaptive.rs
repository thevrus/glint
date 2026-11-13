//! Adaptive mode: match the light to the time of day.
//! Pure maths (NOAA solar position, day curve) first, then the runtime that drives the device.
use crate::device::{self, Shared, Snapshot};
use crate::store::to_hex;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

// ---------------------------------------------------------------- settings

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Sun,
    Fixed,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum PauseMode {
    NextTransition,
    OneHour,
    Tomorrow,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Adaptive {
    pub enabled: bool,
    pub mode: Mode,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// Minutes after local midnight, used in Fixed mode and as the polar fallback.
    pub sunrise_min: u32,
    pub sunset_min: u32,
    pub day_temp: u16,
    pub night_temp: u16,
    pub day_brightness: u16,
    pub night_brightness: u16,
    pub ramp_min: u32,
    pub include_back: bool,
    pub back_day: u8,
    pub back_night: u8,
    pub turn_on: bool,
    pub pause_mode: PauseMode,
}

impl Default for Adaptive {
    fn default() -> Self {
        Adaptive {
            enabled: false,
            mode: Mode::Sun,
            lat: None,
            lon: None,
            sunrise_min: 7 * 60,
            sunset_min: 19 * 60,
            day_temp: 5600,
            night_temp: 2700,
            day_brightness: 220,
            night_brightness: 60,
            ramp_min: 60,
            include_back: false,
            back_day: 35,
            back_night: 12,
            turn_on: false,
            pause_mode: PauseMode::NextTransition,
        }
    }
}

impl Adaptive {
    pub fn sanitize(&mut self) {
        self.day_temp = self.day_temp.clamp(2700, 6500);
        self.night_temp = self.night_temp.clamp(2700, 6500);
        self.day_brightness = self.day_brightness.clamp(30, 400);
        self.night_brightness = self.night_brightness.clamp(30, 400);
        self.ramp_min = self.ramp_min.clamp(15, 180);
        self.sunrise_min = self.sunrise_min.min(1439);
        self.sunset_min = self.sunset_min.min(1439);
        self.back_day = self.back_day.clamp(1, 100);
        self.back_night = self.back_night.clamp(1, 100);
        self.lat = self.lat.map(|v| v.clamp(-90.0, 90.0));
        self.lon = self.lon.map(|v| v.clamp(-180.0, 180.0));
    }

    /// Sunrise and sunset in local minutes after midnight. `ss > sr` always (sunset may exceed 1440).
    /// Returns (sunrise, sunset, fell_back_to_fixed).
    pub fn day_bounds(&self, date: NaiveDate, offset_min: f64) -> (f64, f64, bool) {
        let fixed = || {
            let sr = self.sunrise_min as f64;
            let mut ss = self.sunset_min as f64;
            if ss <= sr {
                ss += 1440.0;
            }
            (sr, ss)
        };
        if self.mode == Mode::Sun {
            if let (Some(lat), Some(lon)) = (self.lat, self.lon) {
                if let Sun::Normal { rise, set } = sun_times(date, lat, lon, offset_min) {
                    let ss = if set <= rise { set + 1440.0 } else { set };
                    return (rise, ss, false);
                }
            }
            let (a, b) = fixed();
            return (a, b, true);
        }
        let (a, b) = fixed();
        (a, b, false)
    }
}

// ---------------------------------------------------------------- NOAA solar position

#[derive(Debug, PartialEq)]
pub enum Sun {
    Normal { rise: f64, set: f64 },
    PolarDay,
    PolarNight,
}

fn julian_day(date: NaiveDate) -> f64 {
    // JD at 0h UT. chrono counts 0001-01-01 as day 1; that date is JD 1721425.5.
    date.num_days_from_ce() as f64 + 1721424.5
}

/// Equation of time (minutes) and solar declination (radians) at Julian century `jc`.
fn sun_eq(jc: f64) -> (f64, f64) {
    let l0 = (280.46646 + jc * (36000.76983 + jc * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + jc * (35999.05029 - 0.0001537 * jc);
    let e = 0.016708634 - jc * (0.000042037 + 0.0000001267 * jc);
    let mr = m.to_radians();
    let c = mr.sin() * (1.914602 - jc * (0.004817 + 0.000014 * jc))
        + (2.0 * mr).sin() * (0.019993 - 0.000101 * jc)
        + (3.0 * mr).sin() * 0.000289;
    let true_long = l0 + c;
    let omega = (125.04 - 1934.136 * jc).to_radians();
    let app_long = (true_long - 0.00569 - 0.00478 * omega.sin()).to_radians();
    let mean_obliq = 23.0 + (26.0 + (21.448 - jc * (46.815 + jc * (0.00059 - jc * 0.001813))) / 60.0) / 60.0;
    let obliq = (mean_obliq + 0.00256 * omega.cos()).to_radians();
    let decl = (obliq.sin() * app_long.sin()).asin();
    let y = (obliq / 2.0).tan().powi(2);
    let l0r = l0.to_radians();
    let eq = y * (2.0 * l0r).sin() - 2.0 * e * mr.sin() + 4.0 * e * y * mr.sin() * (2.0 * l0r).cos()
        - 0.5 * y * y * (4.0 * l0r).sin()
        - 1.25 * e * e * (2.0 * mr).sin();
    (4.0 * eq.to_degrees(), decl)
}

/// Sunrise/sunset for a date at a place, in LOCAL minutes after midnight (0..1440 each).
/// `offset_min` is the UTC offset of the local zone on that date, in minutes.
pub fn sun_times(date: NaiveDate, lat: f64, lon: f64, offset_min: f64) -> Sun {
    let jd0 = julian_day(date);
    let latr = lat.to_radians();
    let ha_arg = |decl: f64| 90.833f64.to_radians().cos() / (latr.cos() * decl.cos()) - latr.tan() * decl.tan();
    let (_, decl0) = sun_eq((jd0 + 0.5 - 2451545.0) / 36525.0);
    let a0 = ha_arg(decl0);
    if a0 > 1.0 {
        return Sun::PolarNight;
    }
    if a0 < -1.0 {
        return Sun::PolarDay;
    }
    let event = |rising: bool| -> Option<f64> {
        let mut minutes = 720.0 - 4.0 * lon;
        for _ in 0..4 {
            let t = jd0 + minutes / 1440.0;
            let (eq, decl) = sun_eq((t - 2451545.0) / 36525.0);
            let a = ha_arg(decl);
            if !(-1.0..=1.0).contains(&a) {
                return None;
            }
            let ha = a.acos().to_degrees();
            minutes = if rising { 720.0 - 4.0 * (lon + ha) - eq } else { 720.0 - 4.0 * (lon - ha) - eq };
        }
        Some(minutes)
    };
    match (event(true), event(false)) {
        (Some(r), Some(s)) => Sun::Normal { rise: (r + offset_min).rem_euclid(1440.0), set: (s + offset_min).rem_euclid(1440.0) },
        _ => Sun::PolarDay,
    }
}

// ---------------------------------------------------------------- day curve

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Temperature level and brightness level (both 0 = night value, 1 = day value) at `t` minutes
/// after local midnight, for a day with sunrise `sr` and sunset `ss` (ss > sr, may exceed 1440).
/// Shape: night -> sunrise ramp (to ~60 % of the way to day) -> midday peak -> afternoon easing
/// warmer -> sunset ramp -> evening, which settles to the night value.
pub fn curve_at(t: f64, sr: f64, ss: f64, ramp: f64) -> (f64, f64) {
    let r = ramp.min((ss - sr) / 2.5).max(1.0);
    let noon = (sr + ss) / 2.0;
    // (time, temp level, brightness level)
    let base = [
        (sr - r / 2.0, 0.0, 0.0),
        (sr + r / 2.0, 0.6, 0.5),
        (noon, 1.0, 1.0),
        (ss - r / 2.0, 0.8, 0.85),
        (ss + r / 2.0, 0.1, 0.3),
        (ss + r * 1.5, 0.0, 0.0),
    ];
    let mut pts: Vec<(f64, f64, f64)> = Vec::with_capacity(18);
    for k in [-1.0, 0.0, 1.0] {
        for p in base {
            pts.push((p.0 + 1440.0 * k, p.1, p.2));
        }
    }
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    for w in pts.windows(2) {
        if t >= w[0].0 && t < w[1].0 {
            let s = smooth((t - w[0].0) / (w[1].0 - w[0].0));
            return (w[0].1 + (w[1].1 - w[0].1) * s, w[0].2 + (w[1].2 - w[0].2) * s);
        }
    }
    (0.0, 0.0)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub temp: u16,
    pub brightness: u16,
    pub back: u8,
}

pub fn target_at(c: &Adaptive, t: f64, sr: f64, ss: f64) -> Target {
    let (tg, bg) = curve_at(t, sr, ss, c.ramp_min as f64);
    Target {
        temp: lerp(c.night_temp as f64, c.day_temp as f64, tg).round().clamp(2700.0, 6500.0) as u16,
        brightness: lerp(c.night_brightness as f64, c.day_brightness as f64, bg).round().clamp(30.0, 400.0) as u16,
        back: lerp(c.back_night as f64, c.back_day as f64, bg).round().clamp(1.0, 100.0) as u8,
    }
}

pub fn kelvin_rgb(k: f64) -> [u8; 3] {
    let t = k / 100.0;
    let c = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    let r = if t <= 66.0 { 255.0 } else { 329.698727446 * (t - 60.0).powf(-0.1332047592) };
    let g = if t <= 66.0 { 99.4708025861 * t.ln() - 161.1195681661 } else { 288.1221695283 * (t - 60.0).powf(-0.0755148492) };
    let b = if t >= 66.0 { 255.0 } else if t <= 19.0 { 0.0 } else { 138.5177312231 * (t - 10.0).ln() - 305.0447927307 };
    [c(r), c(g), c(b)]
}

// ---------------------------------------------------------------- view model for the UI

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub settings: Adaptive,
    pub status: String,
    pub next: String,
    pub paused_text: Option<String>,
    pub target_temp: u16,
    pub target_brightness: u16,
    pub target_back: u8,
    pub now_min: f64,
    pub sunrise: f64,
    pub sunset: f64,
    pub fallback: bool,
    /// (minute, temp, brightness) every 15 min across the local day.
    pub curve: Vec<[f64; 3]>,
}

#[derive(Default)]
pub struct Runtime {
    pub paused_until: Option<i64>,
    pub last_min: Option<f64>,
    pub tint_temp: Option<u16>,
    pub view: View,
}

fn hhmm(min: f64) -> String {
    let m = min.rem_euclid(1440.0).round() as u32 % 1440;
    format!("{:02}:{:02}", m / 60, m % 60)
}

fn to_ts(dt: NaiveDateTime) -> i64 {
    Local
        .from_local_datetime(&dt)
        .earliest()
        .or_else(|| Local.from_local_datetime(&dt).latest())
        .map(|d| d.timestamp())
        .unwrap_or_else(|| dt.and_utc().timestamp())
}

fn at_minute(date: NaiveDate, min: f64) -> NaiveDateTime {
    date.and_hms_opt(0, 0, 0).unwrap() + Duration::seconds((min * 60.0).round() as i64)
}

/// Local "now". `LITRA_FAKE_TIME=HH:MM` (dev only) pins the clock to that time today.
fn now_local() -> (NaiveDateTime, f64) {
    let real = Local::now();
    let off = real.offset().local_minus_utc() as f64 / 60.0;
    if let Ok(s) = std::env::var("LITRA_FAKE_TIME") {
        if let Ok(t) = NaiveTime::parse_from_str(s.trim(), "%H:%M") {
            return (real.date_naive().and_time(t), off);
        }
    }
    (real.naive_local(), off)
}

fn offset_on(date: NaiveDate, fallback: f64) -> f64 {
    let noon = date.and_hms_opt(12, 0, 0).unwrap();
    Local
        .from_local_datetime(&noon)
        .earliest()
        .map(|d| d.offset().local_minus_utc() as f64 / 60.0)
        .unwrap_or(fallback)
}

/// Next sunrise/sunset strictly after `now_min` (minutes after today's midnight).
fn next_event(c: &Adaptive, date: NaiveDate, off: f64, now_min: f64) -> (f64, &'static str) {
    let mut ev: Vec<(f64, &'static str)> = vec![];
    for d in -1i64..=1 {
        let day = date + Duration::days(d);
        let (sr, ss, _) = c.day_bounds(day, offset_on(day, off));
        ev.push((sr + 1440.0 * d as f64, "sunrise"));
        ev.push((ss + 1440.0 * d as f64, "sunset"));
    }
    ev.sort_by(|a, b| a.0.total_cmp(&b.0));
    ev.into_iter().find(|e| e.0 > now_min + 0.5).unwrap_or((now_min + 720.0, "sunrise"))
}

pub fn build_view(c: &Adaptive, now: NaiveDateTime, off: f64, paused_until: Option<i64>) -> View {
    let date = now.date();
    let off = offset_on(date, off);
    let now_min = now.time().num_seconds_from_midnight() as f64 / 60.0;
    let (sr, ss, fallback) = c.day_bounds(date, off);
    let tg = target_at(c, now_min, sr, ss);
    let (nt, nname) = next_event(c, date, off, now_min);
    let next = format!("{} {}", nname, hhmm(nt));
    let paused_text = paused_until.and_then(|ts| {
        DateTime::from_timestamp(ts, 0).map(|d| {
            let l = d.with_timezone(&Local);
            let day = if l.date_naive() != date { "tomorrow " } else { "" };
            format!("{}{:02}:{:02}", day, l.hour(), l.minute())
        })
    });
    let status = if !c.enabled {
        String::new()
    } else if let Some(p) = &paused_text {
        format!("Adaptive paused until {p}")
    } else {
        format!("Adaptive: {} K, {} lm  next: {}", tg.temp, tg.brightness, next)
    };
    let curve = (0..=96)
        .map(|i| {
            let m = i as f64 * 15.0;
            let t = target_at(c, m.min(1439.9), sr, ss);
            [m, t.temp as f64, t.brightness as f64]
        })
        .collect();
    View {
        settings: c.clone(),
        status,
        next,
        paused_text,
        target_temp: tg.temp,
        target_brightness: tg.brightness,
        target_back: tg.back,
        now_min,
        sunrise: sr.rem_euclid(1440.0),
        sunset: ss.rem_euclid(1440.0),
        fallback,
        curve,
    }
}

fn pause_deadline(c: &Adaptive, now: NaiveDateTime, off: f64) -> i64 {
    let date = now.date();
    let now_min = now.time().num_seconds_from_midnight() as f64 / 60.0;
    match c.pause_mode {
        PauseMode::OneHour => to_ts(now) + 3600,
        PauseMode::Tomorrow => to_ts((date + Duration::days(1)).and_hms_opt(0, 0, 0).unwrap()),
        PauseMode::NextTransition => to_ts(at_minute(date, next_event(c, date, offset_on(date, off), now_min).0)),
    }
}

// ---------------------------------------------------------------- runtime

/// The view as of right now, without storing or emitting it.
pub fn current_view(app: &AppHandle) -> View {
    let sh = app.state::<Shared>();
    let (cfg, paused) = (sh.store.lock().unwrap().cfg.adaptive.clone(), sh.adaptive.lock().unwrap().paused_until);
    let (now, off) = now_local();
    build_view(&cfg, now, off, paused)
}

/// Store `view` as the current one and tell the window and the tray.
fn show(app: &AppHandle, view: View) {
    let _ = app.emit("adaptive", &view);
    app.state::<Shared>().adaptive.lock().unwrap().view = view;
    crate::tray::refresh(app);
}

pub fn refresh_view(app: &AppHandle) {
    show(app, current_view(app));
}

pub fn tick(app: &AppHandle) {
    let sh = app.state::<Shared>();
    let (now, off) = now_local();
    let now_ts = to_ts(now);
    let cfg = sh.store.lock().unwrap().cfg.adaptive.clone();
    let (mut paused, last_min) = {
        let rt = sh.adaptive.lock().unwrap();
        (rt.paused_until, rt.last_min)
    };
    if matches!(paused, Some(p) if now_ts >= p) {
        paused = None;
        sh.adaptive.lock().unwrap().paused_until = None;
    }
    let view = build_view(&cfg, now, off, paused);
    if cfg.enabled && paused.is_none() {
        apply(app, &cfg, &view, last_min);
    }
    sh.adaptive.lock().unwrap().last_min = Some(view.now_min);
    show(app, view);
}

fn step(cur: i32, tgt: i32, min_delta: i32, max_step: i32) -> Option<i32> {
    let d = tgt - cur;
    (d.abs() >= min_delta).then(|| cur + d.clamp(-max_step, max_step))
}

fn apply(app: &AppHandle, cfg: &Adaptive, v: &View, last_min: Option<f64>) {
    let sh = app.state::<Shared>();
    let snap: Snapshot = sh.snap.lock().unwrap().clone();
    if !snap.connected {
        return;
    }
    let mut log: Vec<String> = vec![];
    let fail = |e: String| {
        eprintln!("[adaptive] write failed: {e}");
        let _ = app.emit("notice", format!("Adaptive: {e}"));
    };
    if cfg.turn_on && !snap.front_on {
        if let Some(lm) = last_min {
            if lm < v.sunrise && v.sunrise <= v.now_min {
                match device::run(app, move |l| l.set_on(true), |s| s.front_on = true) {
                    Ok(_) => log.push("front on".into()),
                    Err(e) => fail(e),
                }
            }
        }
    }
    let mut temp_now = snap.temperature;
    if let Some(k) = step(snap.temperature as i32, v.target_temp as i32, 100, 200) {
        let k = k as u16;
        match device::run(app, move |l| l.set_temperature(k), |s| s.temperature = k) {
            Ok(_) => {
                temp_now = k;
                log.push(format!("temp {k} K"));
            }
            Err(e) => fail(e),
        }
    }
    if let Some(b) = step(snap.brightness as i32, v.target_brightness as i32, 3, 20) {
        let b = b as u16;
        match device::run(app, move |l| l.set_brightness(b), |s| s.brightness = b) {
            Ok(_) => log.push(format!("brightness {b} lm")),
            Err(e) => fail(e),
        }
    }
    if cfg.include_back && snap.has_back {
        if let Some(p) = step(snap.back_brightness as i32, v.target_back as i32, 2, 8) {
            let p = p as u8;
            match device::run(app, move |l| l.set_back_brightness(p), |s| s.back_brightness = p) {
                Ok(_) => log.push(format!("back {p} %")),
                Err(e) => fail(e),
            }
        }
        let tint = sh.adaptive.lock().unwrap().tint_temp;
        // Match Screen owns the zone colours while it runs.
        if !crate::screen::active() && tint.is_none_or(|t| (t as i32 - temp_now as i32).abs() >= 150) {
            let rgb = kelvin_rgb(temp_now as f64);
            let hex = to_hex(rgb);
            match device::run(app, move |l| l.set_back_color(rgb), |s| s.zones = vec![hex.clone(); crate::store::ZONES]) {
                Ok(_) => {
                    sh.adaptive.lock().unwrap().tint_temp = Some(temp_now);
                    device::save_zones(app, vec![hex.clone(); crate::store::ZONES]);
                    log.push(format!("back tint {hex}"));
                }
                Err(e) => fail(e),
            }
        }
    }
    eprintln!(
        "[adaptive] t={} target {} K {} lm {} % | device was {} K {} lm | {}",
        hhmm(v.now_min),
        v.target_temp,
        v.target_brightness,
        v.target_back,
        snap.temperature,
        snap.brightness,
        if log.is_empty() { "no write".to_string() } else { format!("wrote: {}", log.join(", ")) }
    );
}

/// The user changed something by hand (or applied a preset): pause adaptation.
pub fn manual(app: &AppHandle) {
    let sh = app.state::<Shared>();
    let cfg = sh.store.lock().unwrap().cfg.adaptive.clone();
    if !cfg.enabled {
        return;
    }
    let (now, off) = now_local();
    // This runs on every slider tick. Only the one-hour pause slides with each change; the
    // other deadlines are fixed points, so a pause still in force needs nothing.
    let live = sh.adaptive.lock().unwrap().paused_until.is_some_and(|p| p > to_ts(now));
    if live && cfg.pause_mode != PauseMode::OneHour {
        return;
    }
    let deadline = pause_deadline(&cfg, now, off);
    // The view shows the deadline to the minute: rebuild and re-emit only when that moves.
    let changed = {
        let mut rt = sh.adaptive.lock().unwrap();
        let changed = rt.paused_until.map(|p| p.div_euclid(60)) != Some(deadline.div_euclid(60));
        rt.paused_until = Some(deadline);
        changed
    };
    if changed {
        refresh_view(app);
    }
}

pub fn resume(app: &AppHandle) {
    app.state::<Shared>().adaptive.lock().unwrap().paused_until = None;
    wake(app);
    refresh_view(app);
}

pub fn set_settings(app: &AppHandle, mut s: Adaptive) {
    s.sanitize();
    let sh = app.state::<Shared>();
    {
        let mut st = sh.store.lock().unwrap();
        let was = st.cfg.adaptive.enabled;
        if !s.enabled || !was {
            sh.adaptive.lock().unwrap().paused_until = None;
        }
        st.cfg.adaptive = s;
        st.save();
    }
    sh.adaptive.lock().unwrap().tint_temp = None;
    refresh_view(app);
    wake(app);
}

pub fn wake(app: &AppHandle) {
    let _ = app.state::<Shared>().adaptive_wake.send(());
}

pub fn spawn(app: AppHandle, rx: std::sync::mpsc::Receiver<()>) {
    let secs = std::env::var("LITRA_TICK_SECS").ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(30);
    std::thread::spawn(move || loop {
        tick(&app);
        let _ = rx.recv_timeout(std::time::Duration::from_secs(secs));
        while rx.try_recv().is_ok() {}
    });
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        let diff = (a - b).rem_euclid(1440.0);
        diff.min(1440.0 - diff) <= tol
    }

    fn check(name: &str, date: NaiveDate, lat: f64, lon: f64, off: f64, rise: f64, set: f64) {
        match sun_times(date, lat, lon, off) {
            Sun::Normal { rise: r, set: s } => {
                assert!(close(r, rise, 3.0), "{name} sunrise got {} want {}", hhmm(r), hhmm(rise));
                assert!(close(s, set, 3.0), "{name} sunset got {} want {}", hhmm(s), hhmm(set));
            }
            other => panic!("{name}: {other:?}"),
        }
    }

    #[test]
    fn julian_day_epoch() {
        assert_eq!(julian_day(d(2000, 1, 1)), 2451544.5);
    }

    #[test]
    fn sun_known_cities() {
        // reference: timeanddate.com published times, local civil time
        check("Amsterdam 2024-06-21", d(2024, 6, 21), 52.3676, 4.9041, 120.0, 5.0 * 60.0 + 18.0, 22.0 * 60.0 + 6.0);
        check("Sydney 2024-06-21", d(2024, 6, 21), -33.8688, 151.2093, 600.0, 7.0 * 60.0, 16.0 * 60.0 + 54.0);
        check("Reykjavik 2024-06-21", d(2024, 6, 21), 64.1466, -21.9426, 0.0, 2.0 * 60.0 + 55.0, 4.0);
        check("New York 2024-12-21", d(2024, 12, 21), 40.7128, -74.0060, -300.0, 7.0 * 60.0 + 17.0, 16.0 * 60.0 + 32.0);
    }

    #[test]
    fn sun_polar() {
        assert_eq!(sun_times(d(2024, 6, 21), 69.6492, 18.9553, 120.0), Sun::PolarDay);
        assert_eq!(sun_times(d(2024, 12, 21), 69.6492, 18.9553, 60.0), Sun::PolarNight);
    }

    #[test]
    fn polar_falls_back_to_fixed() {
        let c = Adaptive { lat: Some(69.6492), lon: Some(18.9553), ..Default::default() };
        let (sr, ss, fb) = c.day_bounds(d(2024, 6, 21), 120.0);
        assert!(fb);
        assert_eq!((sr, ss), (420.0, 1140.0));
    }

    #[test]
    fn curve_bounds_and_ramps_monotonic() {
        let (sr, ss, r) = (420.0, 1140.0, 60.0);
        let mut prev = curve_at(sr - r / 2.0, sr, ss, r);
        // sunrise ramp up to midday: never decreasing
        let mut t = sr - r / 2.0;
        while t <= (sr + ss) / 2.0 {
            let c = curve_at(t, sr, ss, r);
            assert!(c.0 >= prev.0 - 1e-9 && c.1 >= prev.1 - 1e-9, "not monotonic rising at {t}");
            assert!((0.0..=1.0).contains(&c.0) && (0.0..=1.0).contains(&c.1));
            prev = c;
            t += 1.0;
        }
        // midday to evening: never increasing
        let mut t = (sr + ss) / 2.0;
        while t <= ss + r * 1.5 {
            let c = curve_at(t, sr, ss, r);
            assert!(c.0 <= prev.0 + 1e-9 && c.1 <= prev.1 + 1e-9, "not monotonic falling at {t}");
            prev = c;
            t += 1.0;
        }
        // night is exactly the night value
        assert_eq!(curve_at(180.0, sr, ss, r), (0.0, 0.0));
        assert_eq!(curve_at(1300.0, sr, ss, r), (0.0, 0.0));
        assert_eq!(curve_at(0.0, sr, ss, r), (0.0, 0.0));
        // midday is the day value
        assert_eq!(curve_at((sr + ss) / 2.0, sr, ss, r), (1.0, 1.0));
    }

    #[test]
    fn targets_reach_night_and_day_values() {
        let c = Adaptive { day_temp: 6500, night_temp: 2700, day_brightness: 400, night_brightness: 30, ..Default::default() };
        let night = target_at(&c, 180.0, 420.0, 1140.0);
        assert_eq!((night.temp, night.brightness, night.back), (2700, 30, c.back_night));
        let noon = target_at(&c, 780.0, 420.0, 1140.0);
        assert_eq!((noon.temp, noon.brightness, noon.back), (6500, 400, c.back_day));
    }

    #[test]
    fn default_curve_hits_spec_anchor_values() {
        let c = Adaptive::default();
        let sunrise_end = target_at(&c, 420.0 + 30.0, 420.0, 1140.0);
        assert!((4300..=4600).contains(&sunrise_end.temp), "{}", sunrise_end.temp);
        let sunset_end = target_at(&c, 1140.0 + 30.0, 420.0, 1140.0);
        assert!((2900..=3100).contains(&sunset_end.temp), "{}", sunset_end.temp);
    }

    #[test]
    fn midnight_wrap_is_continuous() {
        // sunset after midnight (Reykjavik style): sr=175, ss=1444
        let (sr, ss, r) = (175.0, 1444.0, 60.0);
        let a = curve_at(1439.99, sr, ss, r);
        let b = curve_at(0.0, sr, ss, r);
        assert!((a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01);
        // and with a normal day, 23:59 vs 00:00 is flat night
        let a = curve_at(1439.99, 420.0, 1140.0, 60.0);
        let b = curve_at(0.0, 420.0, 1140.0, 60.0);
        assert_eq!(a, b);
    }

    #[test]
    fn step_limits() {
        assert_eq!(step(4000, 4050, 100, 200), None);
        assert_eq!(step(4000, 4100, 100, 200), Some(4100));
        assert_eq!(step(4000, 6000, 100, 200), Some(4200));
        assert_eq!(step(4000, 2700, 100, 200), Some(3800));
    }

    #[test]
    fn kelvin_known_points() {
        assert_eq!(kelvin_rgb(6500.0)[0], 255);
        let warm = kelvin_rgb(2700.0);
        assert!(warm[0] == 255 && warm[1] > 150 && warm[1] < 190 && warm[2] > 70 && warm[2] < 120, "{warm:?}");
    }

    fn fixed(pause_mode: PauseMode) -> Adaptive {
        // Fixed mode: sunrise 07:00, sunset 19:00, no dependence on the machine's time zone.
        Adaptive { enabled: true, mode: Mode::Fixed, pause_mode, ..Default::default() }
    }

    fn at(h: u32, m: u32) -> NaiveDateTime {
        d(2024, 6, 21).and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn view_names_the_next_transition() {
        let c = fixed(PauseMode::NextTransition);
        for (h, m, want) in [(3, 0, "sunrise 07:00"), (12, 0, "sunset 19:00"), (19, 0, "sunrise 07:00"), (23, 59, "sunrise 07:00")] {
            assert_eq!(build_view(&c, at(h, m), 0.0, None).next, want, "at {h:02}:{m:02}");
        }
    }

    #[test]
    fn view_status_and_curve() {
        let c = fixed(PauseMode::NextTransition);
        // 13:00 is solar noon between 07:00 and 19:00: the day values exactly
        let v = build_view(&c, at(13, 0), 0.0, None);
        assert_eq!(v.status, format!("Adaptive: {} K, {} lm  next: sunset 19:00", c.day_temp, c.day_brightness));
        // the UI plots one point per 15 min, both ends of the day included
        assert_eq!(v.curve.len(), 97);
        let off = Adaptive { enabled: false, ..c };
        assert_eq!(build_view(&off, at(12, 0), 0.0, None).status, "");
    }

    #[test]
    fn pause_lasts_until_the_chosen_deadline() {
        use PauseMode::*;
        for (mode, h, m, want) in [
            (NextTransition, 12, 0, "19:00"),
            (NextTransition, 20, 0, "tomorrow 07:00"),
            (OneHour, 12, 0, "13:00"),
            (OneHour, 23, 30, "tomorrow 00:30"),
            (Tomorrow, 12, 0, "tomorrow 00:00"),
        ] {
            let c = fixed(mode);
            let v = build_view(&c, at(h, m), 0.0, Some(pause_deadline(&c, at(h, m), 0.0)));
            assert_eq!(v.paused_text.as_deref(), Some(want), "{mode:?} at {h:02}:{m:02}");
            assert_eq!(v.status, format!("Adaptive paused until {want}"));
        }
    }

    #[test]
    fn fixed_bounds_wrap_past_midnight() {
        let c = Adaptive { mode: Mode::Fixed, sunrise_min: 1200, sunset_min: 360, ..Default::default() };
        assert_eq!(c.day_bounds(d(2024, 6, 21), 0.0), (1200.0, 1800.0, false));
        // Sun mode with no location yet uses the fixed times and says so
        assert_eq!(Adaptive::default().day_bounds(d(2024, 6, 21), 0.0), (420.0, 1140.0, true));
    }

    #[test]
    fn sanitize_clamps_stored_settings_to_device_limits() {
        let mut c = Adaptive {
            day_temp: 9000,
            night_temp: 100,
            day_brightness: 999,
            night_brightness: 0,
            ramp_min: 0,
            sunrise_min: 5000,
            sunset_min: 1440,
            back_day: 200,
            back_night: 0,
            lat: Some(123.0),
            lon: Some(-999.0),
            ..Default::default()
        };
        c.sanitize();
        assert_eq!((c.day_temp, c.night_temp, c.day_brightness, c.night_brightness), (6500, 2700, 400, 30));
        assert_eq!((c.ramp_min, c.sunrise_min, c.back_day, c.back_night), (15, 1439, 100, 1));
        assert_eq!((c.lat, c.lon), (Some(90.0), Some(-180.0)));
        assert_eq!(c.sunset_min, 1439);
        let mut long = Adaptive { ramp_min: 999, ..Default::default() };
        long.sanitize();
        assert_eq!(long.ramp_min, 180);
    }
}
