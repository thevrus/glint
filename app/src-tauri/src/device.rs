//! All HID I/O lives on one dedicated thread that owns the HidApi and the Light.
//! Everything else sends it jobs over a channel and waits for the answer, so no UI or
//! command thread ever touches the device directly. The same thread polls for
//! plug/unplug, sleep/wake and out-of-range, and re-discovers with backoff.
use crate::adaptive;
use crate::store::{default_zones, parse_hex, Store};
use glint_core::{Error, HidApi, Light};
use serde::Serialize;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Serialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub connected: bool,
    pub model: String,
    pub has_back: bool,
    pub front_on: bool,
    pub brightness: u16,
    pub temperature: u16,
    pub back_on: bool,
    pub back_brightness: u8,
    pub zones: Vec<String>,
    /// A real failure other than "no light found" (for example a permission problem).
    pub error: Option<String>,
}

pub struct Dev {
    pub light: Option<Light>,
}

pub enum Msg {
    Job(Box<dyn FnOnce(&mut Dev) + Send>),
    Retry,
}

pub struct Shared {
    pub tx: Sender<Msg>,
    pub snap: Mutex<Snapshot>,
    pub store: Mutex<Store>,
    pub adaptive: Mutex<adaptive::Runtime>,
    pub adaptive_wake: Sender<()>,
    /// Asks the saver thread to write the config once changes go quiet.
    pub save_tx: Sender<()>,
}

fn is_link_error(e: &Error) -> bool {
    matches!(e, Error::Hid(_) | Error::Timeout)
}

/// Run `op` on the device thread. `upd` runs afterwards on the caller to update the cache.
pub fn run<T: Send + 'static>(
    app: &AppHandle,
    op: impl FnOnce(&Light) -> glint_core::Result<T> + Send + 'static,
    upd: impl FnOnce(&mut Snapshot),
) -> Result<T, String> {
    let v = exec(app, op)?;
    upd(&mut app.state::<Shared>().snap.lock().unwrap());
    publish(app);
    Ok(v)
}

/// Like `run`, but leaves the cache alone and publishes nothing on success (Match Screen
/// writes ten times a second and paces its own state events).
pub fn exec<T: Send + 'static>(
    app: &AppHandle,
    op: impl FnOnce(&Light) -> glint_core::Result<T> + Send + 'static,
) -> Result<T, String> {
    let sh = app.state::<Shared>();
    let (rtx, rrx) = mpsc::channel();
    let job = Msg::Job(Box::new(move |dev: &mut Dev| {
        let r = match dev.light.as_ref() {
            None => Err(None),
            Some(l) => op(l).map_err(Some),
        };
        if matches!(&r, Err(Some(e)) if is_link_error(e)) {
            dev.light = None;
        }
        let _ = rtx.send(r);
    }));
    sh.tx.send(job).map_err(|_| "Device thread stopped".to_string())?;
    match rrx.recv().map_err(|_| "Device thread stopped".to_string())? {
        Ok(v) => Ok(v),
        Err(None) => Err("Light not connected.".to_string()),
        Err(Some(e)) => {
            if is_link_error(&e) {
                mark_disconnected(app, None);
                let _ = sh.tx.send(Msg::Retry);
                Err("Lost contact with the light.".to_string())
            } else {
                Err(e.to_string())
            }
        }
    }
}

pub fn mark_disconnected(app: &AppHandle, error: Option<String>) {
    let sh = app.state::<Shared>();
    {
        let mut s = sh.snap.lock().unwrap();
        s.connected = false;
        s.error = error;
    }
    publish(app);
}

pub fn publish(app: &AppHandle) {
    let sh = app.state::<Shared>();
    let snap = sh.snap.lock().unwrap().clone();
    let _ = app.emit("state", &snap);
    crate::tray::refresh(app);
}

pub fn save_zones(app: &AppHandle, zones: Vec<String>) {
    let sh = app.state::<Shared>();
    // Called on every colour-drag tick: the disk write is debounced on the saver thread.
    sh.store.lock().unwrap().cfg.zones = Some(zones);
    let _ = sh.save_tx.send(());
}

fn read_full(l: &Light, s: &mut Snapshot) -> glint_core::Result<()> {
    s.front_on = l.is_on()?;
    s.brightness = l.brightness()?.clamp(30, 400);
    s.temperature = l.temperature()?.clamp(2700, 6500);
    if l.info.has_back {
        s.back_on = l.back_is_on()?;
        s.back_brightness = l.back_brightness()?.clamp(1, 100);
    }
    Ok(())
}

fn window_open(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some()
}

fn backoff(misses: u32) -> Duration {
    Duration::from_secs(match misses {
        0..=3 => 2,
        4..=9 => 4,
        _ => 6,
    })
}

pub fn spawn(app: AppHandle, rx: Receiver<Msg>) {
    std::thread::Builder::new()
        .name("device".into())
        .spawn(move || {
            let mut api = loop {
                match HidApi::new() {
                    Ok(a) => break a,
                    Err(e) => {
                        mark_disconnected(&app, Some(e.to_string()));
                        std::thread::sleep(Duration::from_secs(2));
                    }
                }
            };
            let mut dev = Dev { light: None };
            let mut misses = 0u32;
            let mut polls = 0u64;
            let mut due = Instant::now();
            loop {
                match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                    Ok(Msg::Job(j)) => {
                        j(&mut dev);
                        // A job that just ran proves the link; keep the health poll out of
                        // the way of the writes queued behind it (a slider drag).
                        let min = Instant::now() + Duration::from_secs(1);
                        if dev.light.is_some() && due < min {
                            due = min;
                        }
                    }
                    Ok(Msg::Retry) => {
                        misses = 0;
                        due = Instant::now();
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        polls += 1;
                        let next = poll(&app, &mut api, &mut dev, &mut misses, polls);
                        due = Instant::now() + next;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("spawn device thread");
}

fn poll(app: &AppHandle, api: &mut HidApi, dev: &mut Dev, misses: &mut u32, polls: u64) -> Duration {
    let sh = app.state::<Shared>();
    if dev.light.is_none() {
        let _ = api.refresh_devices();
        match Light::open_first(api) {
            Ok(l) => {
                let mut s = sh.snap.lock().unwrap().clone();
                s.model = l.info.name.clone();
                s.has_back = l.info.has_back;
                s.error = None;
                let restore = sh.store.lock().unwrap().cfg.zones.clone();
                match read_full(&l, &mut s) {
                    Ok(()) => {
                        // Show the light as soon as its state is read. Restoring the zone
                        // colours is 14 more round trips; commands sent meanwhile queue up.
                        s.connected = true;
                        s.zones = restore.clone().unwrap_or_else(default_zones);
                        *sh.snap.lock().unwrap() = s;
                        publish(app);
                        if l.info.has_back {
                            for (i, hex) in restore.iter().flatten().enumerate().take(7) {
                                let Some(rgb) = parse_hex(hex) else { continue };
                                if let Err(e) = l.set_back_zone(i as u8 + 1, rgb) {
                                    *misses += 1;
                                    mark_disconnected(app, Some(e.to_string()));
                                    return backoff(*misses);
                                }
                            }
                        }
                        dev.light = Some(l);
                        *misses = 0;
                        adaptive::wake(app);
                        return backoff(0);
                    }
                    Err(e) => {
                        *misses += 1;
                        mark_disconnected(app, Some(e.to_string()));
                    }
                }
            }
            Err(Error::NotFound) => {
                *misses += 1;
                let changed = {
                    let s = sh.snap.lock().unwrap();
                    s.connected || s.error.is_some()
                };
                if changed {
                    mark_disconnected(app, None);
                }
            }
            Err(e) => {
                *misses += 1;
                mark_disconnected(app, Some(e.to_string()));
            }
        }
        return backoff(*misses);
    }

    // Connected: health check. Full read when the window is open, otherwise a cheap
    // is_on every poll and a full read now and then so the cache stays honest.
    let full = window_open(app) || polls.is_multiple_of(6);
    let mut s = sh.snap.lock().unwrap().clone();
    let r = if full {
        read_full(dev.light.as_ref().unwrap(), &mut s)
    } else {
        dev.light.as_ref().unwrap().is_on().map(|v| s.front_on = v)
    };
    match r {
        Ok(()) => {
            // Zones are never read back; keep the live ones (Match Screen updates them meanwhile).
            let changed = {
                let mut cur = sh.snap.lock().unwrap();
                s.zones = cur.zones.clone();
                let c = *cur != s;
                if c {
                    *cur = s;
                }
                c
            };
            if changed {
                publish(app);
            }
            *misses = 0;
            if window_open(app) { Duration::from_secs(2) } else { Duration::from_secs(5) }
        }
        Err(e) => {
            if is_link_error(&e) {
                dev.light = None;
                *misses = 0;
                mark_disconnected(app, None);
                Duration::from_secs(2)
            } else {
                Duration::from_secs(2)
            }
        }
    }
}
