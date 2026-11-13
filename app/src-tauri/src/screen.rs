//! Back-light modes that run on their own: Match Screen, where the back light follows the
//! colours on the main display, and the animated effects in `effects`.
//! For Match Screen a worker takes a tiny screenshot a few times a second, averages seven vertical strips,
//! and fades the light towards them slowly enough that it never pulls the eye. It is built
//! to cost nothing when there is nothing to do: no screenshots while the back light is off,
//! the light is away or the display sleeps, one a second while the picture stands still.
use crate::device::{self, Shared};
use crate::effects::{self, Effect};
use crate::store::{parse_hex, to_hex, zones_or_default, ZONES};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// The light faces the wall, so seen from the desk its zones may run the other way.
/// false: left of the screen drives zone 1. Flip if colours land on the wrong side.
const MIRROR: bool = false;
/// One fade step while the colours are moving.
const TICK: Duration = Duration::from_millis(80);
/// Between screenshots while the picture is changing, and once it has stood still.
const CAPTURE_GAP: Duration = Duration::from_millis(250);
const IDLE_GAP: Duration = Duration::from_secs(1);
const STILL_AFTER: Duration = Duration::from_secs(3);
/// A screenshot this far (0..255, any channel of any zone) from the last counts as movement.
const MOVED: f32 = 3.0;
/// Smoothing time constant: a hard cut settles in about three of these (~1.5 s).
const TAU_MS: f32 = 500.0;
const SATURATION: f32 = 1.3;
/// Largest channel change (0..255) worth a HID write.
const THRESHOLD: u8 = 2;
const EMIT_GAP: Duration = Duration::from_millis(200);

pub type Rgb = [u8; 3];
type Res<T> = Result<T, String>;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static RUN: Mutex<Option<Running>> = Mutex::new(None);

/// What drives the back light while the user's own colours are set aside.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Screen,
    Effect(Effect),
}

impl Mode {
    pub fn id(self) -> &'static str {
        match self {
            Mode::Screen => "screen",
            Mode::Effect(e) => e.id(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Screen => "Match Screen",
            Mode::Effect(e) => e.name(),
        }
    }

    pub fn parse(id: &str) -> Option<Mode> {
        if id == "screen" { Some(Mode::Screen) } else { Effect::parse(id).map(Mode::Effect) }
    }
}

struct Running {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
    saved: Vec<String>,
    mode: Mode,
}

/// The user's own zone colours from before a mode started, while one is running.
pub fn saved_zones() -> Option<Vec<String>> {
    RUN.lock().unwrap().as_ref().map(|r| r.saved.clone())
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// The running mode, if any.
pub fn mode() -> Option<Mode> {
    RUN.lock().unwrap().as_ref().map(|r| r.mode)
}

#[tauri::command]
pub fn get_back_mode() -> String {
    mode().map_or("off", Mode::id).to_string()
}

/// "off", "screen" or the id of an effect.
#[tauri::command]
pub async fn set_back_mode(app: AppHandle, mode: String) -> Res<()> {
    if mode == "off" {
        stop(&app, true);
        return Ok(());
    }
    start(&app, Mode::parse(&mode).ok_or("Unknown back light mode.")?)
}

fn halt(r: &Running) {
    r.stop.store(true, Ordering::Relaxed);
    r.thread.thread().unpark();
}

fn start(app: &AppHandle, mode: Mode) -> Res<()> {
    let mut run = RUN.lock().unwrap();
    if run.as_ref().is_some_and(|r| r.mode == mode) {
        return Ok(());
    }
    let sh = app.state::<Shared>();
    let (zones, has_back) = {
        let s = sh.snap.lock().unwrap();
        (s.zones.clone(), s.has_back)
    };
    if !has_back {
        return Err("This light has no back light.".into());
    }
    if mode == Mode::Screen {
        capture::allowed()?;
    }
    // Switching from another mode: the snapshot holds one of its frames, so the colours to
    // come back to are the ones that mode saved.
    let saved = match run.take() {
        Some(r) => {
            halt(&r);
            let _ = r.thread.join();
            r.saved
        }
        None => zones_or_default(zones),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let (a, s, z) = (app.clone(), stop.clone(), saved.clone());
    let thread = match std::thread::Builder::new().name("screen".into()).spawn(move || worker(a, s, z, mode)) {
        Ok(t) => t,
        Err(e) => {
            // nothing is running any more: hand the colours back rather than strand a frame
            sh.snap.lock().unwrap().zones = saved;
            ACTIVE.store(false, Ordering::Relaxed);
            device::publish(app);
            let _ = app.emit("back-mode", "off");
            return Err(e.to_string());
        }
    };
    *run = Some(Running { stop, thread, saved, mode });
    ACTIVE.store(true, Ordering::Relaxed);
    let _ = app.emit("back-mode", mode.id());
    Ok(())
}

/// Stop the worker. `restore` puts back the zones from before the mode started;
/// skip it when something else (a preset) is about to set them anyway.
pub fn stop(app: &AppHandle, restore: bool) {
    // Held to the end so a second stop or a start cannot interleave with the restore.
    // The worker never takes RUN, so joining under it cannot deadlock.
    let mut run = RUN.lock().unwrap();
    let Some(r) = run.take() else { return };
    halt(&r);
    let _ = r.thread.join();
    // Snapshot first: nothing may read an animation frame as the user's own colours.
    app.state::<Shared>().snap.lock().unwrap().zones = r.saved.clone();
    if restore {
        let rgb: Vec<Rgb> = r.saved.iter().map(|h| parse_hex(h).unwrap_or([255; 3])).collect();
        let w = device::exec(app, move |l| {
            let list: Vec<(u8, Rgb)> = rgb.iter().enumerate().map(|(i, c)| (i as u8 + 1, *c)).collect();
            l.set_back_zones(&list)
        });
        if let Err(e) = w {
            // The store still holds these zones, so a reconnect restores them.
            eprintln!("[screen] restore: {e}");
        }
    }
    // Cleared only now, so manual colour edits stay blocked until the restore has been written.
    ACTIVE.store(false, Ordering::Relaxed);
    device::publish(app);
    let _ = app.emit("back-mode", "off");
}

/// Sleep unless a stop is already pending. `stop()` unparks once, and a channel wait inside
/// the loop can swallow that wake-up, so every sleep checks the flag first.
fn nap(stop: &AtomicBool, d: Duration) {
    if !stop.load(Ordering::Relaxed) {
        std::thread::park_timeout(d);
    }
}

/// The time one fade step may account for. After a sleep between screenshots the real gap
/// is long; stepping by all of it would turn the first frame of a fade into a jump.
pub fn fade_dt(elapsed_ms: f32) -> f32 {
    elapsed_ms.clamp(0.0, TICK.as_secs_f32() * 1500.0)
}

/// What has reached the light and the window, and whether writes are currently failing.
struct Out {
    sent: [Option<Rgb>; ZONES],
    last_emit: Instant,
    emitted: Vec<String>,
    failing: bool,
}

impl Out {
    /// Report a failure once per streak, so the user learns why nothing is happening.
    fn fail(&mut self, app: &AppHandle, mode: Mode, msg: String) {
        if !self.failing {
            eprintln!("[screen] {msg}");
            let _ = app.emit("notice", format!("{}: {msg}", mode.name()));
        }
        self.failing = true;
    }

    /// Write `zones` as one frame if they moved enough, and keep the window's picture current.
    fn show(&mut self, app: &AppHandle, mode: Mode, zones: [Rgb; ZONES]) {
        let frame = changed(&self.sent, &zones, THRESHOLD);
        if !frame.is_empty() {
            let list: Vec<(u8, Rgb)> = frame.iter().map(|&i| (i as u8 + 1, zones[i])).collect();
            match device::exec(app, move |l| l.set_back_zones(&list)) {
                Ok(()) => {
                    for i in frame {
                        self.sent[i] = Some(zones[i]);
                    }
                    self.failing = false;
                }
                Err(e) => {
                    self.sent = [None; ZONES];
                    self.fail(app, mode, e);
                }
            }
        }
        let hex: Vec<String> = zones.iter().map(|c| to_hex(*c)).collect();
        app.state::<Shared>().snap.lock().unwrap().zones = hex.clone();
        if hex != self.emitted && self.last_emit.elapsed() >= EMIT_GAP {
            device::publish(app);
            self.last_emit = Instant::now();
            self.emitted = hex;
        }
    }
}

fn worker(app: AppHandle, stop: Arc<AtomicBool>, saved: Vec<String>, mode: Mode) {
    let base: [Rgb; ZONES] = std::array::from_fn(|i| saved.get(i).and_then(|h| parse_hex(h)).unwrap_or([255; 3]));
    let mut out = Out { sent: [None; ZONES], last_emit: Instant::now() - EMIT_GAP, emitted: vec![], failing: false };
    let started = Instant::now();
    let mut target: Option<[[f32; 3]; ZONES]> = None;
    let mut smooth: Option<[[f32; 3]; ZONES]> = None;
    let mut grab: Option<capture::Grabber> = None;
    let mut last = Instant::now();
    let mut last_capture = Instant::now();
    let mut last_change = Instant::now();
    let mut was_idle = false;
    while !stop.load(Ordering::Relaxed) {
        let t0 = Instant::now();
        // Nothing to light: no screenshots and no writes until that changes.
        let (connected, back_on) = {
            let sh = app.state::<Shared>();
            let s = sh.snap.lock().unwrap();
            (s.connected, s.back_on)
        };
        let dark = mode == Mode::Screen && capture::asleep();
        if !connected || !back_on || dark {
            was_idle = true;
            // a back light that is merely switched off should pick up promptly when it comes on
            nap(&stop, if connected && !dark { CAPTURE_GAP } else { IDLE_GAP });
            continue;
        }
        if was_idle {
            // A reconnect restores the stored zones on the light, and the picture has moved on:
            // resend everything and start from the screen as it is now.
            out.sent = [None; ZONES];
            target = None;
            smooth = None;
            was_idle = false;
        }

        let effect = match mode {
            Mode::Effect(e) => e,
            Mode::Screen => {
                let gap = if last_change.elapsed() < STILL_AFTER { CAPTURE_GAP } else { IDLE_GAP };
                if target.is_none() || last_capture.elapsed() >= gap {
                    if grab.is_none() {
                        match capture::Grabber::new() {
                            Ok(g) => grab = Some(g),
                            Err(e) => {
                                out.fail(&app, mode, e);
                                nap(&stop, Duration::from_secs(1));
                                continue;
                            }
                        }
                    }
                    let frame = match grab.as_ref().unwrap().grab() {
                        Ok(f) => f,
                        Err(e) => {
                            out.fail(&app, mode, e);
                            grab = None;
                            nap(&stop, Duration::from_millis(500));
                            continue;
                        }
                    };
                    let next = blend(strips(&frame.px, frame.w, frame.h).map(|c| saturate(c, SATURATION)));
                    if target.is_none_or(|t| distance(&t, &next) > MOVED) {
                        last_change = Instant::now();
                    }
                    target = Some(next);
                    last_capture = Instant::now();
                }
                let Some(goal) = target else { continue };
                let dt = fade_dt(t0.duration_since(last).as_secs_f32() * 1000.0);
                last = t0;
                let s = step(smooth, goal, dt, TAU_MS);
                smooth = Some(s);
                out.show(&app, mode, order(s.map(to_rgb), MIRROR));
                // Mid-fade the loop runs at the fade rate; once the light has arrived it sleeps
                // until the next screenshot is due.
                let wait = if distance(&s, &goal) >= 1.0 { TICK.saturating_sub(t0.elapsed()) } else { gap.saturating_sub(last_capture.elapsed()) };
                nap(&stop, wait.max(Duration::from_millis(10)));
                continue;
            }
        };
        out.show(&app, mode, effects::frame(effect, started.elapsed().as_secs_f32(), &base));
        nap(&stop, TICK.saturating_sub(t0.elapsed()).max(Duration::from_millis(10)));
    }
}

// ---------------------------------------------------------------- colour maths

/// Chroma-weighted average of each of 7 equal vertical strips of an RGBX frame (4 bytes a pixel, rows packed).
/// All-black rows at the top and bottom (letterbox bars) are left out.
pub fn strips(px: &[u8], w: usize, h: usize) -> [[f32; 3]; ZONES] {
    let at = |x: usize, y: usize| &px[(y * w + x) * 4..(y * w + x) * 4 + 3];
    let lit = |y: usize| (0..w).any(|x| at(x, y).iter().any(|&v| v > 16));
    let top = (0..h).find(|&y| lit(y));
    let Some(top) = top else { return [[0.0; 3]; ZONES] };
    let bottom = (0..h).rev().find(|&y| lit(y)).unwrap_or(top);
    let mut sum = [[0.0f32; 3]; ZONES];
    let mut n = [0.0f32; ZONES];
    for y in top..=bottom {
        for x in 0..w {
            let s = x * ZONES / w;
            let p = at(x, y);
            // Colourful pixels count for more: a plain average of a desktop is mostly window
            // white and grey, which would leave the light looking white whatever is on screen.
            let (hi, lo) = (p.iter().max().copied().unwrap_or(0) as f32, p.iter().min().copied().unwrap_or(0) as f32);
            let chroma = (hi - lo) / 255.0;
            let wgt = 0.05 + chroma * chroma * 4.0;
            for c in 0..3 {
                sum[s][c] += p[c] as f32 * wgt;
            }
            n[s] += wgt;
        }
    }
    std::array::from_fn(|s| sum[s].map(|v| if n[s] > 0.0 { v / n[s] } else { 0.0 }))
}

/// Push a colour away from its own grey by `k` (1.0 leaves it unchanged).
pub fn saturate(c: [f32; 3], k: f32) -> [f32; 3] {
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    c.map(|v| (l + (v - l) * k).clamp(0.0, 255.0))
}

/// Exponential smoothing that is independent of the tick rate.
pub fn step(prev: Option<[[f32; 3]; ZONES]>, target: [[f32; 3]; ZONES], dt_ms: f32, tau_ms: f32) -> [[f32; 3]; ZONES] {
    let Some(prev) = prev else { return target };
    let a = 1.0 - (-dt_ms.max(0.0) / tau_ms).exp();
    std::array::from_fn(|i| std::array::from_fn(|c| prev[i][c] + (target[i][c] - prev[i][c]) * a))
}

pub fn to_rgb(c: [f32; 3]) -> Rgb {
    c.map(|v| v.round().clamp(0.0, 255.0) as u8)
}

/// Strip i to zone i (index 0 is zone 1), or reversed when the light is mirrored.
pub fn order(strips: [Rgb; ZONES], mirror: bool) -> [Rgb; ZONES] {
    std::array::from_fn(|i| strips[if mirror { ZONES - 1 - i } else { i }])
}

/// Largest channel difference; an unknown (never sent) zone counts as maximal.
pub fn delta(sent: Option<Rgb>, now: Rgb) -> u8 {
    sent.map_or(u8::MAX, |s| (0..3).map(|c| s[c].abs_diff(now[c])).max().unwrap_or(0))
}

/// Zones (0-based) to write as one frame: none until some zone has moved more than
/// `threshold`, then every zone that differs at all, so the strip changes together.
pub fn changed(sent: &[Option<Rgb>; ZONES], now: &[Rgb; ZONES], threshold: u8) -> Vec<usize> {
    if (0..ZONES).all(|i| delta(sent[i], now[i]) <= threshold) {
        return vec![];
    }
    (0..ZONES).filter(|&i| delta(sent[i], now[i]) > 0).collect()
}

/// Let each zone borrow a quarter from either neighbour, so something small moving across a
/// strip boundary does not flip two zones back and forth.
pub fn blend(s: [[f32; 3]; ZONES]) -> [[f32; 3]; ZONES] {
    std::array::from_fn(|i| {
        let (l, r) = (s[i.saturating_sub(1)], s[(i + 1).min(ZONES - 1)]);
        std::array::from_fn(|c| 0.25 * l[c] + 0.5 * s[i][c] + 0.25 * r[c])
    })
}

/// Largest channel difference between two sets of zones.
pub fn distance(a: &[[f32; 3]; ZONES], b: &[[f32; 3]; ZONES]) -> f32 {
    (0..ZONES).flat_map(|i| (0..3).map(move |c| (a[i][c] - b[i][c]).abs())).fold(0.0, f32::max)
}

// ---------------------------------------------------------------- capture

pub struct Frame {
    pub px: Vec<u8>,
    pub w: usize,
    pub h: usize,
}

#[cfg(target_os = "macos")]
mod capture {
    //! ScreenCaptureKit one-shot screenshots (SCScreenshotManager, macOS 14+), scaled down
    //! by the system to a 64 px wide image and redrawn into an RGBX buffer we own.
    use super::{Frame, Res};
    use block2::RcBlock;
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::AllocAnyThread;
    use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGColorSpace, CGContext, CGDisplayIsAsleep, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGMainDisplayID,
        CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess,
    };
    use objc2_foundation::{NSArray, NSError};
    use objc2_screen_capture_kit::{SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow};
    use std::ptr::NonNull;
    use std::sync::mpsc;
    use std::time::Duration;

    const W: usize = 64;
    const WAIT: Duration = Duration::from_secs(1);

    /// Moves a value out of a ScreenCaptureKit completion handler to the waiting worker.
    /// Safe here: the handler hands it over and never touches it again.
    struct Handoff<T>(T);
    unsafe impl<T> Send for Handoff<T> {}

    /// The main display is asleep, so there is nothing to follow.
    pub fn asleep() -> bool {
        CGDisplayIsAsleep(CGMainDisplayID())
    }

    /// Whether Screen Recording is already granted; never prompts.
    pub fn granted() -> bool {
        CGPreflightScreenCaptureAccess()
    }

    pub fn allowed() -> Res<()> {
        if granted() || CGRequestScreenCaptureAccess() {
            Ok(())
        } else {
            Err("Allow Glint in System Settings > Privacy & Security > Screen Recording, then quit and reopen Glint.".into())
        }
    }

    pub struct Grabber {
        filter: Retained<SCContentFilter>,
        config: Retained<SCStreamConfiguration>,
        h: usize,
    }

    impl Grabber {
        pub fn new() -> Res<Grabber> {
            autoreleasepool(|_| {
                let (tx, rx) = mpsc::channel();
                let main = CGMainDisplayID();
                let done = RcBlock::new(move |content: *mut SCShareableContent, _: *mut NSError| {
                    let r = unsafe { content.as_ref() }.and_then(|c| {
                        let all = unsafe { c.displays() };
                        let d = all.iter().find(|d| unsafe { d.displayID() } == main).or_else(|| all.firstObject())?;
                        let (dw, dh) = unsafe { (d.width(), d.height()) };
                        let none = NSArray::<SCWindow>::new();
                        let f = unsafe { SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &d, &none) };
                        Some(Handoff((f, dw.max(1) as usize, dh.max(1) as usize)))
                    });
                    let _ = tx.send(r);
                });
                unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&done) };
                let Handoff((filter, dw, dh)) = rx.recv_timeout(WAIT).ok().flatten().ok_or("No display to capture.")?;
                let h = (W * dh / dw).clamp(8, W);
                let config = unsafe { SCStreamConfiguration::new() };
                unsafe {
                    config.setWidth(W);
                    config.setHeight(h);
                    config.setShowsCursor(false);
                }
                Ok(Grabber { filter, config, h })
            })
        }

        pub fn grab(&self) -> Res<Frame> {
            autoreleasepool(|_| {
                let (tx, rx) = mpsc::channel();
                let done = RcBlock::new(move |img: *mut CGImage, _: *mut NSError| {
                    let _ = tx.send(NonNull::new(img).map(|p| Handoff(unsafe { CFRetained::retain(p) })));
                });
                unsafe {
                    SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&self.filter, &self.config, Some(&done))
                };
                let Handoff(img) = rx.recv_timeout(WAIT).ok().flatten().ok_or("Screen capture failed.")?;
                pixels(&img, W, self.h).map(|px| Frame { px, w: W, h: self.h }).ok_or_else(|| "Could not read the screen image.".into())
            })
        }
    }

    /// Draw `img` into a w x h RGBX buffer, whatever its own pixel format and size.
    fn pixels(img: &CGImage, w: usize, h: usize) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; w * h * 4];
        let space = CGColorSpace::new_device_rgb()?;
        let info = CGImageAlphaInfo::NoneSkipLast.0 | CGImageByteOrderInfo::Order32Big.0;
        let ctx = unsafe { CGBitmapContextCreate(buf.as_mut_ptr().cast(), w, h, 8, w * 4, Some(&space), info) }?;
        CGContext::draw_image(Some(&ctx), CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(w as f64, h as f64)), Some(img));
        drop(ctx);
        Some(buf)
    }
}

#[cfg(not(target_os = "macos"))]
mod capture {
    use super::{Frame, Res};

    pub fn allowed() -> Res<()> {
        Err("Match Screen is only available on macOS.".into())
    }

    pub fn asleep() -> bool {
        true
    }

    pub struct Grabber;
    impl Grabber {
        pub fn new() -> Res<Grabber> {
            Err("unsupported".into())
        }
        pub fn grab(&self) -> Res<Frame> {
            Err("unsupported".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: usize, h: usize, f: impl Fn(usize, usize) -> Rgb) -> Vec<u8> {
        let mut v = vec![];
        for y in 0..h {
            for x in 0..w {
                let c = f(x, y);
                v.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        v
    }

    #[test]
    fn strips_split_left_to_right() {
        // 14 px wide: two columns a strip, strip i gets red = i * 30.
        let px = frame(14, 4, |x, _| [(x / 2 * 30) as u8, 0, 100]);
        let s = strips(&px, 14, 4);
        for (i, c) in s.iter().enumerate() {
            assert_eq!(to_rgb(*c), [(i * 30) as u8, 0, 100]);
        }
    }

    #[test]
    fn strips_skip_letterbox_rows() {
        let px = frame(7, 10, |_, y| if y < 2 || y >= 8 { [0, 0, 0] } else { [200, 100, 50] });
        assert!(strips(&px, 7, 10).iter().all(|c| to_rgb(*c) == [200, 100, 50]));
        let black = frame(7, 4, |_, _| [0, 0, 0]);
        assert!(strips(&black, 7, 4).iter().all(|c| to_rgb(*c) == [0, 0, 0]));
    }

    #[test]
    fn saturate_keeps_grey_and_boosts_colour() {
        assert_eq!(to_rgb(saturate([128.0; 3], 1.3)), [128; 3]);
        let c = saturate([200.0, 100.0, 100.0], 1.3);
        assert!(c[0] > 200.0 && c[1] < 100.0);
        assert!(saturate([250.0, 0.0, 0.0], 2.0).iter().all(|v| (0.0..=255.0).contains(v)));
    }

    #[test]
    fn smoothing_settles_in_about_three_tau() {
        let (a, b) = ([[0.0; 3]; ZONES], [[255.0; 3]; ZONES]);
        assert_eq!(step(None, b, 100.0, 100.0), b);
        let mut s = a;
        for _ in 0..3 {
            s = step(Some(s), b, 100.0, 100.0);
        }
        assert!(s[0][0] > 240.0 && s[0][0] < 255.0, "{}", s[0][0]);
        // tick-rate independent: one 200 ms step equals two 100 ms steps
        let one = step(Some(a), b, 200.0, TAU_MS);
        let two = step(Some(step(Some(a), b, 100.0, TAU_MS)), b, 100.0, TAU_MS);
        // one fade step never moves the light by more than a quarter of the way, however long
        // the loop slept before it
        for slept in [80.0, 250.0, 1000.0, 60_000.0] {
            assert!(step(Some(a), b, fade_dt(slept), TAU_MS)[0][0] < 255.0 / 4.0, "{slept}");
        }
        assert_eq!(fade_dt(40.0), 40.0);
        assert!((one[0][0] - two[0][0]).abs() < 0.01);
    }

    #[test]
    fn order_mirrors() {
        let s: [Rgb; ZONES] = std::array::from_fn(|i| [i as u8; 3]);
        assert_eq!(order(s, false)[0], [0; 3]);
        assert_eq!(order(s, true)[0], [6; 3]);
        assert_eq!(order(s, true)[6], [0; 3]);
    }

    #[test]
    fn changed_writes_whole_frame_or_nothing() {
        let sent = [Some([10, 10, 10]); ZONES];
        let mut now = [[10u8; 3]; ZONES];
        now[1] = [12, 10, 10]; // at the threshold: not worth a write yet
        assert!(changed(&sent, &now, THRESHOLD).is_empty());
        now[4] = [10, 40, 10];
        // once one zone has moved, every zone that differs goes in the same frame
        assert_eq!(changed(&sent, &now, THRESHOLD), vec![1, 4]);
        let mut unknown = sent;
        unknown[2] = None;
        assert_eq!(changed(&unknown, &[[10; 3]; ZONES], THRESHOLD), vec![2]);
    }

    #[test]
    fn blend_softens_edges_and_keeps_flat_colour() {
        let flat = [[90.0, 20.0, 200.0]; ZONES];
        assert_eq!(blend(flat), flat);
        let mut one = [[0.0; 3]; ZONES];
        one[3] = [200.0; 3];
        let b = blend(one);
        assert_eq!((b[2][0], b[3][0], b[4][0], b[1][0]), (50.0, 100.0, 50.0, 0.0));
        assert_eq!(distance(&one, &b), 100.0);
    }
}

/// Manual check of the real capture path: `cargo test capture_smoke -- --ignored --nocapture`.
#[cfg(all(test, target_os = "macos"))]
mod smoke {
    use super::*;

    #[test]
    #[ignore]
    fn capture_smoke() {
        println!("screen recording granted: {}", capture::granted());
        if !capture::granted() {
            return;
        }
        let g = capture::Grabber::new().expect("grabber");
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let f = g.grab().expect("grab");
            let z = strips(&f.px, f.w, f.h).map(|c| to_hex(to_rgb(saturate(c, SATURATION))));
            println!("{}x{} in {:?}: {}", f.w, f.h, t.elapsed(), z.join(" "));
        }
    }
}
