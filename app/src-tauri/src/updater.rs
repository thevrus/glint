//! Self-update from GitHub Releases, driven from the tray menu. This is the only network
//! request the app makes: one small JSON fetch after launch and then once a day, and the
//! download itself when the user picks "Install". Updates are verified against the public
//! key in tauri.conf.json before they replace the app.
use crate::device::Shared;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

#[derive(Clone, PartialEq)]
enum State {
    Idle,
    Checking,
    UpToDate,
    Available(String),
    Installing(String),
    Failed,
}

static STATE: Mutex<State> = Mutex::new(State::Idle);

fn set(app: &AppHandle, s: State) {
    *STATE.lock().unwrap() = s;
    crate::tray::refresh(app);
}

/// Label and enabled flag for the tray item.
pub fn menu_item() -> (String, bool) {
    match STATE.lock().unwrap().clone() {
        State::Idle => ("Check for Updates…".into(), true),
        State::Checking => ("Checking for Updates…".into(), false),
        State::UpToDate => ("Glint is Up to Date".into(), true),
        State::Available(v) => (format!("Install Glint {v} and Restart"), true),
        State::Installing(v) => (format!("Installing Glint {v}…"), false),
        State::Failed => ("Update Failed. Try Again".into(), true),
    }
}

pub fn auto(app: &AppHandle) -> bool {
    !app.state::<Shared>().store.lock().unwrap().cfg.skip_update_check
}

pub fn toggle_auto(app: &AppHandle) {
    {
        let sh = app.state::<Shared>();
        let mut st = sh.store.lock().unwrap();
        st.cfg.skip_update_check = !st.cfg.skip_update_check;
        st.save();
    }
    crate::tray::refresh(app);
}

/// Check, and with `install` also download, verify, replace the app and restart.
async fn run(app: &AppHandle, install: bool) -> tauri_plugin_updater::Result<State> {
    // The timeout covers the check only; without it a stalled connection would leave the
    // menu item on "Checking" for good.
    let updater = app.updater_builder().timeout(Duration::from_secs(30)).build()?;
    let Some(update) = updater.check().await? else {
        return Ok(State::UpToDate);
    };
    if !install {
        return Ok(State::Available(update.version));
    }
    update.download_and_install(|_, _| {}, || {}).await?;
    app.restart()
}

/// Claim the updater for one run: `Some(install)` if it was free, `None` if a check or an
/// install is already going. Only a click may start an install.
fn begin(app: &AppHandle, manual: bool) -> Option<bool> {
    let install = {
        let mut s = STATE.lock().unwrap();
        let (next, install) = match &*s {
            State::Checking | State::Installing(_) => return None,
            State::Available(v) if manual => (State::Installing(v.clone()), true),
            State::Available(_) => return None,
            _ => (State::Checking, false),
        };
        *s = next;
        install
    };
    crate::tray::refresh(app);
    Some(install)
}

/// `manual` checks report "up to date" and failures in the menu; background ones stay quiet.
async fn check(app: AppHandle, install: bool, manual: bool) {
    let next = match run(&app, install).await {
        Ok(State::UpToDate) if !manual => State::Idle,
        Ok(s) => s,
        Err(e) => {
            eprintln!("[update] {e}");
            if manual { State::Failed } else { State::Idle }
        }
    };
    set(&app, next);
}

/// The tray item was clicked.
pub fn clicked(app: &AppHandle) {
    if let Some(install) = begin(app, true) {
        tauri::async_runtime::spawn(check(app.clone(), install, true));
    }
}

pub fn spawn(app: AppHandle) {
    // A dev build must not offer to replace itself with a release.
    if cfg!(debug_assertions) {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(15));
        loop {
            if auto(&app) && begin(&app, false).is_some() {
                tauri::async_runtime::block_on(check(app.clone(), false, false));
            }
            std::thread::sleep(Duration::from_secs(24 * 60 * 60));
        }
    });
}
