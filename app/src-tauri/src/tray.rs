//! Menu bar / system tray icon and menu. The tray process is the app: it stays alive
//! with no window, and the menu is rebuilt only when something it shows has changed.
use crate::device::{Shared, Snapshot};
use crate::store::{builtins, Preset};
use std::sync::Mutex;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

const ID: &str = "main";
static LAST_SIG: Mutex<String> = Mutex::new(String::new());

pub fn all_presets(app: &AppHandle) -> Vec<Preset> {
    let mut p = builtins();
    p.extend(app.state::<Shared>().store.lock().unwrap().cfg.presets.clone());
    p
}

fn menu(app: &AppHandle, s: &Snapshot, status: &str, adaptive_on: bool, presets: &[Preset], update: &(String, bool), auto_update: bool) -> tauri::Result<Menu<tauri::Wry>> {
    let mut items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = vec![];
    if !s.connected {
        items.push(Box::new(MenuItem::with_id(app, "info", "Light not connected", false, None::<&str>)?));
    } else {
        if adaptive_on && !status.is_empty() {
            items.push(Box::new(MenuItem::with_id(app, "info", status, false, None::<&str>)?));
        }
        items.push(Box::new(CheckMenuItem::with_id(app, "front", "Front light", true, s.front_on, None::<&str>)?));
        if s.has_back {
            items.push(Box::new(CheckMenuItem::with_id(app, "back", "Back light", true, s.back_on, None::<&str>)?));
        }
        items.push(Box::new(CheckMenuItem::with_id(app, "adaptive", "Adaptive", true, adaptive_on, None::<&str>)?));
        items.push(Box::new(PredefinedMenuItem::separator(app)?));
        let pitems: Vec<MenuItem<tauri::Wry>> = presets
            .iter()
            .map(|p| MenuItem::with_id(app, format!("preset:{}", p.id), &p.name, true, None::<&str>))
            .collect::<tauri::Result<_>>()?;
        let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = pitems.iter().map(|i| i as &dyn IsMenuItem<tauri::Wry>).collect();
        items.push(Box::new(Submenu::with_items(app, "Presets", true, &refs)?));
    }
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(app, "update", &update.0, update.1, None::<&str>)?));
    items.push(Box::new(CheckMenuItem::with_id(app, "update-auto", "Check for Updates Automatically", true, auto_update, None::<&str>)?));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(app, "quit", "Quit", true, Some("CmdOrCtrl+Q"))?));
    let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|i| i.as_ref()).collect();
    Menu::with_items(app, &refs)
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    let mut b = TrayIconBuilder::with_id(ID)
        .icon(icon)
        .tooltip("Glint")
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let id = event.id.as_ref().to_string();
            let app = app.clone();
            if id == "show" {
                crate::show_window(&app);
            } else if id == "quit" {
                app.exit(0);
            } else if id == "update" {
                crate::updater::clicked(&app);
            } else if id == "update-auto" {
                crate::updater::toggle_auto(&app);
            } else {
                std::thread::spawn(move || crate::tray_action(&app, &id));
            }
        });
    #[cfg(target_os = "macos")]
    {
        b = b.icon_as_template(true);
    }
    let m = menu(app, &Snapshot::default(), "", false, &builtins(), &crate::updater::menu_item(), crate::updater::auto(app))?;
    b = b.menu(&m);
    b.build(app)?;
    Ok(())
}

/// Rebuild the tray menu if anything it displays changed. Safe to call from any thread.
pub fn refresh(app: &AppHandle) {
    let sh = app.state::<Shared>();
    let s = sh.snap.lock().unwrap().clone();
    let (status, on) = {
        let rt = sh.adaptive.lock().unwrap();
        (rt.view.status.clone(), rt.view.settings.enabled)
    };
    let presets = all_presets(app);
    let update = crate::updater::menu_item();
    let auto_update = crate::updater::auto(app);
    let sig = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        s.connected,
        s.has_back,
        s.front_on,
        s.back_on,
        on,
        status,
        presets.iter().map(|p| format!("{}={}", p.id, p.name)).collect::<Vec<_>>().join(","),
        update.0,
        auto_update
    );
    {
        let mut last = LAST_SIG.lock().unwrap();
        if *last == sig {
            return;
        }
        *last = sig;
    }
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let (Some(tray), Ok(m)) = (app2.tray_by_id(ID), menu(&app2, &s, &status, on, &presets, &update, auto_update)) {
            let _ = tray.set_menu(Some(m));
        }
    });
}
