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

fn state() -> State {
    STATE.lock().unwrap().clone()
}

fn set(app: &AppHandle, s: State) {
    *STATE.lock().unwrap() = s;
    crate::tray::refresh(app);
}

/// Label and enabled flag for the tray item.
pub fn menu_item() -> (String, bool) {
    match state() {
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
    let Some(update) = app.updater()?.check().await? else {
        return Ok(State::UpToDate);
    };
    if !install {
        return Ok(State::Available(update.version));
    }
    set(app, State::Installing(update.version.clone()));
    update.download_and_install(|_, _| {}, || {}).await?;
    app.restart()
}

/// `manual` checks report "up to date" and failures in the menu; background ones stay quiet.
async fn check(app: AppHandle, install: bool, manual: bool) {
    if !install {
        set(&app, State::Checking);
    }
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
    let install = match state() {
        State::Checking | State::Installing(_) => return,
        State::Available(_) => true,
        _ => false,
    };
    tauri::async_runtime::spawn(check(app.clone(), install, true));
}

pub fn spawn(app: AppHandle) {
    // A dev build must not offer to replace itself with a release.
    if cfg!(debug_assertions) {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(15));
        loop {
            if auto(&app) && matches!(state(), State::Idle | State::UpToDate | State::Failed) {
                tauri::async_runtime::block_on(check(app.clone(), false, false));
            }
            std::thread::sleep(Duration::from_secs(24 * 60 * 60));
        }
    });
}
