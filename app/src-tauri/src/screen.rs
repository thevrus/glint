//! Match Screen: the back light follows the colours on the main display.
//! A worker captures a tiny screenshot about ten times a second, averages seven vertical
//! strips, smooths them over time and writes only the zones that moved. Each zone is one
//! HID round trip, so the loop measures its writes and sends fewer when they run long.
use crate::device::{self, Shared};
use crate::store::{default_zones, parse_hex, to_hex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// The light faces the wall, so seen from the desk its zones may run the other way.
/// false: left of the screen drives zone 1. Flip if colours land on the wrong side.
const MIRROR: bool = false;
const ZONES: usize = 7;
const INTERVAL: Duration = Duration::from_millis(100);
/// Smoothing time constant: a hard cut settles in about three of these (~300 ms).
const TAU_MS: f32 = 100.0;
const SATURATION: f32 = 1.3;
/// Largest channel change (0..255) worth a HID write.
const THRESHOLD: u8 = 6;
const EMIT_GAP: Duration = Duration::from_millis(200);

pub type Rgb = [u8; 3];
type Res<T> = Result<T, String>;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static RUN: Mutex<Option<Running>> = Mutex::new(None);

struct Running {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
    saved: Vec<String>,
}

/// The user's own zone colours from before Match Screen started, while it is running.
pub fn saved_zones() -> Option<Vec<String>> {
    RUN.lock().unwrap().as_ref().map(|r| r.saved.clone())
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

#[tauri::command]
pub fn get_screen_sync() -> bool {
    active()
}

#[tauri::command]
pub async fn set_screen_sync(app: AppHandle, on: bool) -> Res<()> {
    if on {
        start(&app)
    } else {
        stop(&app, true);
        Ok(())
    }
}

fn start(app: &AppHandle) -> Res<()> {
    let mut run = RUN.lock().unwrap();
    if run.is_some() {
        return Ok(());
    }
    let sh = app.state::<Shared>();
    let (saved, has_back) = {
        let s = sh.snap.lock().unwrap();
        (s.zones.clone(), s.has_back)
    };
    if !has_back {
        return Err("This light has no back light.".into());
    }
    capture::allowed()?;
    let saved = if saved.len() == ZONES { saved } else { default_zones() };
    let stop = Arc::new(AtomicBool::new(false));
    let (a, s, z) = (app.clone(), stop.clone(), saved.clone());
    let thread = std::thread::Builder::new()
        .name("screen".into())
        .spawn(move || worker(a, s, z))
        .map_err(|e| e.to_string())?;
    *run = Some(Running { stop, thread, saved });
    ACTIVE.store(true, Ordering::Relaxed);
    let _ = app.emit("screen-sync", true);
    Ok(())
}

/// Stop the worker. `restore` puts back the zones from before Match Screen started;
/// skip it when something else (a preset) is about to set them anyway.
pub fn stop(app: &AppHandle, restore: bool) {
    // Held to the end so a second stop or a start cannot interleave with the restore.
    // The worker never takes RUN, so joining under it cannot deadlock.
    let mut run = RUN.lock().unwrap();
    let Some(r) = run.take() else { return };
    r.stop.store(true, Ordering::Relaxed);
    r.thread.thread().unpark();
    let _ = r.thread.join();
    // Snapshot first: nothing may read a screen frame as the user's own colours.
    app.state::<Shared>().snap.lock().unwrap().zones = r.saved.clone();
    if restore {
        let rgb: Vec<Rgb> = r.saved.iter().map(|h| parse_hex(h).unwrap_or([255; 3])).collect();
        let w = device::exec(app, move |l| {
            for (i, c) in rgb.iter().enumerate() {
                l.set_back_zone(i as u8 + 1, *c)?;
            }
            Ok(())
        });
        if let Err(e) = w {
            // The store still holds these zones, so a reconnect restores them.
            eprintln!("[screen] restore: {e}");
        }
    }
    // Cleared only now, so manual colour edits stay blocked until the restore has been written.
    ACTIVE.store(false, Ordering::Relaxed);
    device::publish(app);
    let _ = app.emit("screen-sync", false);
}

fn worker(app: AppHandle, stop: Arc<AtomicBool>, saved: Vec<String>) {
    let unknown = [None; ZONES];
    let mut sent: [Option<Rgb>; ZONES] = std::array::from_fn(|i| saved.get(i).and_then(|h| parse_hex(h)));
    let mut smooth: Option<[[f32; 3]; ZONES]> = None;
    let mut per_write = 40.0f32;
    let mut grab: Option<capture::Grabber> = None;
    let mut last = Instant::now();
    let mut last_emit = Instant::now() - EMIT_GAP;
    let mut emitted: Vec<String> = vec![];
    let mut was_connected = true;
    let mut failing = false;
    let quiet = |msg: String, failing: &mut bool| {
        if !*failing {
            eprintln!("[screen] {msg}");
            // once per failure streak, so the user learns why nothing is happening
            let _ = app.emit("notice", format!("Match Screen: {msg}"));
        }
        *failing = true;
    };
    while !stop.load(Ordering::Relaxed) {
        let t0 = Instant::now();
        if grab.is_none() {
            match capture::Grabber::new() {
                Ok(g) => grab = Some(g),
                Err(e) => {
                    quiet(e, &mut failing);
                    std::thread::park_timeout(Duration::from_secs(1));
                    continue;
                }
            }
        }
        let frame = match grab.as_ref().unwrap().grab() {
            Ok(f) => f,
            Err(e) => {
                quiet(e, &mut failing);
                grab = None;
                std::thread::park_timeout(Duration::from_millis(500));
                continue;
            }
        };
        let captured = t0.elapsed();
        let dt = t0.duration_since(last).as_secs_f32() * 1000.0;
        last = t0;
        let target = strips(&frame.px, frame.w, frame.h).map(|c| saturate(c, SATURATION));
        let s = step(smooth, target, dt, TAU_MS);
        smooth = Some(s);
        let zones = order(s.map(to_rgb), MIRROR);

        let connected = app.state::<Shared>().snap.lock().unwrap().connected;
        if connected && !was_connected {
            // A reconnect restores the stored zones on the light: resend everything.
            sent = unknown;
        }
        was_connected = connected;
        if connected {
            let n = budget(INTERVAL.as_secs_f32() * 1000.0, captured.as_secs_f32() * 1000.0, per_write);
            let picks = pick(&sent, &zones, THRESHOLD, n);
            if !picks.is_empty() {
                let list: Vec<(u8, Rgb)> = picks.iter().map(|&i| (i as u8 + 1, zones[i])).collect();
                let w0 = Instant::now();
                match device::exec(&app, move |l| list.iter().try_for_each(|&(z, c)| l.set_back_zone(z, c))) {
                    Ok(()) => {
                        let each = w0.elapsed().as_secs_f32() * 1000.0 / picks.len() as f32;
                        per_write = per_write * 0.7 + each * 0.3;
                        for i in picks {
                            sent[i] = Some(zones[i]);
                        }
                        failing = false;
                    }
                    Err(e) => {
                        sent = unknown;
                        quiet(e, &mut failing);
                    }
                }
            }
        }

        let hex: Vec<String> = zones.iter().map(|c| to_hex(*c)).collect();
        app.state::<Shared>().snap.lock().unwrap().zones = hex.clone();
        if hex != emitted && last_emit.elapsed() >= EMIT_GAP {
            device::publish(&app);
            last_emit = Instant::now();
            emitted = hex;
        }
        std::thread::park_timeout(INTERVAL.saturating_sub(t0.elapsed()));
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

/// Zones (0-based) to write this tick: moved more than `threshold`, largest change first,
/// at most `max`.
pub fn pick(sent: &[Option<Rgb>; ZONES], now: &[Rgb; ZONES], threshold: u8, max: usize) -> Vec<usize> {
    let mut d: Vec<(usize, u8)> = (0..ZONES).map(|i| (i, delta(sent[i], now[i]))).filter(|&(_, d)| d > threshold).collect();
    d.sort_by(|a, b| b.1.cmp(&a.1));
    d.into_iter().take(max).map(|(i, _)| i).collect()
}

/// How many zone writes fit in what is left of the tick after capturing (always at least one).
pub fn budget(interval_ms: f32, spent_ms: f32, per_write_ms: f32) -> usize {
    let left = (interval_ms - spent_ms) * 0.9;
    ((left / per_write_ms.max(1.0)).floor().max(1.0) as usize).min(ZONES)
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
        CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGMainDisplayID,
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

    /// Whether Screen Recording is already granted; never prompts.
    pub fn granted() -> bool {
        CGPreflightScreenCaptureAccess()
    }

    pub fn allowed() -> Res<()> {
        if CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() {
            Ok(())
        } else {
            Err("Allow Litra Beam in System Settings > Privacy & Security > Screen Recording, then quit and reopen Litra Beam.".into())
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
        assert_eq!(step(None, b, 100.0, TAU_MS), b);
        let mut s = a;
        for _ in 0..3 {
            s = step(Some(s), b, 100.0, TAU_MS);
        }
        assert!(s[0][0] > 240.0 && s[0][0] < 255.0, "{}", s[0][0]);
        // tick-rate independent: one 200 ms step equals two 100 ms steps
        let one = step(Some(a), b, 200.0, TAU_MS);
        let two = step(Some(step(Some(a), b, 100.0, TAU_MS)), b, 100.0, TAU_MS);
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
    fn pick_largest_changes_first_and_caps() {
        let sent = [Some([0, 0, 0]); ZONES];
        let mut now = [[0u8; 3]; ZONES];
        now[1] = [10, 0, 0];
        now[4] = [0, 90, 0];
        now[6] = [0, 0, 40];
        now[3] = [3, 0, 0]; // under the threshold
        assert_eq!(pick(&sent, &now, THRESHOLD, 7), vec![4, 6, 1]);
        assert_eq!(pick(&sent, &now, THRESHOLD, 2), vec![4, 6]);
        assert!(pick(&sent, &sent.map(|c| c.unwrap()), THRESHOLD, 7).is_empty());
        let mut unknown = sent;
        unknown[2] = None;
        assert_eq!(pick(&unknown, &now, THRESHOLD, 1), vec![2]);
    }

    #[test]
    fn budget_shrinks_with_slow_writes() {
        assert_eq!(budget(100.0, 10.0, 5.0), 7);
        assert_eq!(budget(100.0, 10.0, 30.0), 2);
        assert_eq!(budget(100.0, 10.0, 200.0), 1);
        assert_eq!(budget(100.0, 150.0, 10.0), 1);
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
