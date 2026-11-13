#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adaptive;
mod device;
mod screen;
mod store;
mod tray;
mod updater;

use device::{run, Msg, Shared, Snapshot};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use store::{parse_hex, to_hex, Preset, Store};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewWindowBuilder};

static START: OnceLock<Instant> = OnceLock::new();
static SHOW_T0: Mutex<Option<Instant>> = Mutex::new(None);

type Res<T> = Result<T, String>;

pub fn show_window(app: &AppHandle) {
    *SHOW_T0.lock().unwrap() = Some(Instant::now());
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    if let Some(c) = app.config().app.windows.first().cloned() {
        // Hand the page its data before any script runs, so the first frame is already the
        // real state instead of defaults that pop once three IPC round trips come back.
        let boot = serde_json::json!({
            "state": *app.state::<Shared>().snap.lock().unwrap(),
            "presets": tray::all_presets(app),
            "adaptive": adaptive::current_view(app),
        });
        let script = format!("window.__BOOT__={boot};");
        if let Err(e) = WebviewWindowBuilder::from_config(app, &c).and_then(|b| b.initialization_script(script).build()) {
            eprintln!("could not open window: {e}");
        }
    }
}

fn notice(app: &AppHandle, msg: String) {
    eprintln!("[notice] {msg}");
    let _ = app.emit("notice", msg);
}

fn hex7(zones: &[String]) -> Vec<[u8; 3]> {
    (0..store::ZONES).map(|i| zones.get(i).and_then(|h| parse_hex(h)).unwrap_or([255, 255, 255])).collect()
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_state(app: AppHandle) -> Snapshot {
    app.state::<Shared>().snap.lock().unwrap().clone()
}

#[tauri::command]
async fn set_front_power(app: AppHandle, on: bool) -> Res<()> {
    run(&app, move |l| l.set_on(on), move |s| s.front_on = on)
}

#[tauri::command]
async fn set_front_brightness(app: AppHandle, lm: u16) -> Res<()> {
    adaptive::manual(&app);
    run(&app, move |l| l.set_brightness(lm), move |s| s.brightness = lm)
}

#[tauri::command]
async fn set_temperature(app: AppHandle, kelvin: u16) -> Res<()> {
    adaptive::manual(&app);
    run(&app, move |l| l.set_temperature(kelvin), move |s| s.temperature = kelvin)
}

#[tauri::command]
async fn set_back_power(app: AppHandle, on: bool) -> Res<()> {
    run(&app, move |l| l.set_back_on(on), move |s| s.back_on = on)
}

#[tauri::command]
async fn set_back_brightness(app: AppHandle, pct: u8) -> Res<()> {
    adaptive::manual(&app);
    run(&app, move |l| l.set_back_brightness(pct), move |s| s.back_brightness = pct)
}

fn not_syncing() -> Res<()> {
    if screen::active() {
        return Err("Turn off Match Screen to pick colours.".into());
    }
    Ok(())
}

#[tauri::command]
async fn set_back_zone(app: AppHandle, zone: u8, color: String) -> Res<()> {
    not_syncing()?;
    let rgb = parse_hex(&color).ok_or("Not a valid colour.")?;
    let hex = to_hex(rgb);
    adaptive::manual(&app);
    let mut zones = store::zones_or_default(app.state::<Shared>().snap.lock().unwrap().zones.clone());
    run(&app, move |l| l.set_back_zone(zone, rgb), |s| {
        if (1..=store::ZONES as u8).contains(&zone) {
            zones[zone as usize - 1] = hex;
        }
        s.zones = zones.clone();
        device::save_zones(&app, zones);
    })
}

#[tauri::command]
async fn set_back_all(app: AppHandle, color: String) -> Res<()> {
    not_syncing()?;
    let rgb = parse_hex(&color).ok_or("Not a valid colour.")?;
    let hex = to_hex(rgb);
    adaptive::manual(&app);
    run(&app, move |l| l.set_back_color(rgb), |s| {
        s.zones = vec![hex.clone(); store::ZONES];
        device::save_zones(&app, s.zones.clone());
    })
}

#[tauri::command]
fn list_presets(app: AppHandle) -> Vec<Preset> {
    tray::all_presets(&app)
}

fn presets_changed(app: &AppHandle) {
    let _ = app.emit("presets", tray::all_presets(app));
    tray::refresh(app);
}

pub fn apply_preset_by_id(app: &AppHandle, id: &str) -> Res<()> {
    let p = tray::all_presets(app).into_iter().find(|p| p.id == id).ok_or("Preset not found.")?;
    // The preset sets the zones itself, so no restore first.
    screen::stop(app, false);
    let has_back = app.state::<Shared>().snap.lock().unwrap().has_back;
    let zones = hex7(&p.zones);
    let (b, t, fo, bb, bo) = (p.brightness, p.temperature, p.front_on, p.back_brightness, p.back_on);
    adaptive::manual(app);
    run(
        app,
        move |l| {
            l.set_brightness(b)?;
            l.set_temperature(t)?;
            if has_back {
                l.set_back_brightness(bb)?;
                for (i, c) in zones.iter().enumerate() {
                    l.set_back_zone(i as u8 + 1, *c)?;
                }
                l.set_back_on(bo)?;
            }
            l.set_on(fo)
        },
        |s| {
            s.brightness = b;
            s.temperature = t;
            s.front_on = fo;
            if has_back {
                s.back_brightness = bb;
                s.back_on = bo;
                s.zones = p.zones.clone();
                device::save_zones(app, p.zones.clone());
            }
        },
    )
}

#[tauri::command]
async fn apply_preset(app: AppHandle, id: String) -> Res<()> {
    apply_preset_by_id(&app, &id)
}

#[tauri::command]
fn save_preset(app: AppHandle, name: String) -> Res<Preset> {
    let sh = app.state::<Shared>();
    let mut s = sh.snap.lock().unwrap().clone();
    // While Match Screen runs the snapshot holds a screen frame; a preset keeps the user's own colours.
    if let Some(z) = screen::saved_zones() {
        s.zones = z;
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the preset a name.".into());
    }
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let p = Preset {
        id: format!("u{ms}"),
        name: name.chars().take(32).collect(),
        builtin: false,
        icon: "slider".into(),
        front_on: s.front_on,
        brightness: s.brightness,
        temperature: s.temperature,
        back_on: s.back_on,
        back_brightness: s.back_brightness,
        zones: store::zones_or_default(s.zones),
    };
    {
        let mut st = sh.store.lock().unwrap();
        st.cfg.presets.push(p.clone());
        st.save();
    }
    presets_changed(&app);
    Ok(p)
}

#[tauri::command]
fn rename_preset(app: AppHandle, id: String, name: String) -> Res<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the preset a name.".into());
    }
    {
        let sh = app.state::<Shared>();
        let mut st = sh.store.lock().unwrap();
        let p = st.cfg.presets.iter_mut().find(|p| p.id == id).ok_or("Preset not found.")?;
        p.name = name.chars().take(32).collect();
        st.save();
    }
    presets_changed(&app);
    Ok(())
}

#[tauri::command]
fn delete_preset(app: AppHandle, id: String) -> Res<()> {
    {
        let sh = app.state::<Shared>();
        let mut st = sh.store.lock().unwrap();
        st.cfg.presets.retain(|p| p.id != id);
        st.save();
    }
    presets_changed(&app);
    Ok(())
}

#[tauri::command]
fn retry_connect(app: AppHandle) {
    let _ = app.state::<Shared>().tx.send(Msg::Retry);
}

#[tauri::command]
fn get_adaptive(app: AppHandle) -> adaptive::View {
    adaptive::current_view(&app)
}

#[tauri::command]
fn set_adaptive(app: AppHandle, settings: adaptive::Adaptive) {
    adaptive::set_settings(&app, settings);
}

#[tauri::command]
fn adaptive_resume(app: AppHandle) {
    adaptive::resume(&app);
}

#[tauri::command]
fn window_ready(connected: bool) {
    let since_show = SHOW_T0.lock().unwrap().map(|t| t.elapsed().as_millis());
    let since_start = START.get().map(|t| t.elapsed().as_millis());
    eprintln!("[timing] window ready: {:?} ms after show request, {:?} ms after process start, connected={connected}", since_show, since_start);
}

// ---------------------------------------------------------------- tray actions

pub fn tray_action(app: &AppHandle, id: &str) {
    let sh = app.state::<Shared>();
    let snap = sh.snap.lock().unwrap().clone();
    let r = match id {
        "front" => run(app, move |l| l.set_on(!snap.front_on), move |s| s.front_on = !snap.front_on),
        "back" => run(app, move |l| l.set_back_on(!snap.back_on), move |s| s.back_on = !snap.back_on),
        "adaptive" => {
            let mut a = sh.store.lock().unwrap().cfg.adaptive.clone();
            a.enabled = !a.enabled;
            adaptive::set_settings(app, a);
            Ok(())
        }
        _ => match id.strip_prefix("preset:") {
            Some(pid) => apply_preset_by_id(app, pid),
            None => Ok(()),
        },
    };
    if let Err(e) = r {
        notice(app, e);
    }
}

fn main() {
    START.get_or_init(Instant::now);
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_state,
            set_front_power,
            set_front_brightness,
            set_temperature,
            set_back_power,
            set_back_brightness,
            set_back_zone,
            set_back_all,
            list_presets,
            apply_preset,
            save_preset,
            rename_preset,
            delete_preset,
            retry_connect,
            get_adaptive,
            set_adaptive,
            adaptive_resume,
            window_ready,
            screen::set_screen_sync,
            screen::get_screen_sync
        ])
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let mut store = Store::load(dir);
            store.cfg.adaptive.sanitize();
            let zones = store.cfg.zones.clone().unwrap_or_else(store::default_zones);
            let (tx, rx) = mpsc::channel();
            let (atx, arx) = mpsc::channel();
            let (stx, srx) = mpsc::channel::<()>();
            let saver = app.handle().clone();
            std::thread::spawn(move || {
                // Write once the requests have been quiet for 400 ms.
                while srx.recv().is_ok() {
                    while srx.recv_timeout(std::time::Duration::from_millis(400)).is_ok() {}
                    saver.state::<Shared>().store.lock().unwrap().save();
                }
            });
            app.manage(Shared {
                save_tx: stx,
                tx,
                snap: Mutex::new(Snapshot { zones, ..Default::default() }),
                store: Mutex::new(store),
                adaptive: Mutex::new(adaptive::Runtime::default()),
                adaptive_wake: atx,
            });
            let handle = app.handle().clone();
            tray::init(&handle)?;
            device::spawn(handle.clone(), rx);
            adaptive::spawn(handle.clone(), arx);
            updater::spawn(handle.clone());
            show_window(&handle);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| match event {
        // Closing the window destroys the webview; the tray process keeps running.
        RunEvent::ExitRequested { api, code, .. } if code.is_none() => api.prevent_exit(),
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_window(app),
        // Flush a zone colour still waiting on the debounced saver.
        RunEvent::Exit => {
            screen::stop(app, true);
            app.state::<Shared>().store.lock().unwrap().save()
        }
        _ => {}
    });
}
