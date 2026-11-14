mod daemon;
mod island;
mod notify;
mod tray;
mod watcher;

use serde::Serialize;
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindowBuilder};

/// Where the front end should connect, read from the daemon's data directory.
#[derive(Serialize)]
struct DaemonInfo {
    url: String,
    token: String,
    home: String,
}

/// Reads `$MUSTER_HOME/daemon.json` and `token` on every call, so a restarted daemon
/// (new port) is picked up on reconnect. Never starts a daemon: that happens once at
/// launch, or when the user presses "Start daemon" (`start_daemon`).
/// Opens a web link in the default browser. Only http and https: terminal output is
/// untrusted, so no other scheme (and no file path) may reach `open`.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !is_web_url(&url) {
        return Err("only http and https links can be opened".into());
    }
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener).arg(&url).spawn().map(|_| ()).map_err(|e| format!("{opener}: {e}"))
}

fn is_web_url(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    rest.is_some_and(|r| !r.is_empty() && !r.starts_with('-')) && !url.chars().any(|c| c.is_control() || c == ' ')
}

#[tauri::command]
fn daemon_info() -> Result<DaemonInfo, String> {
    let home = daemon::muster_home()?;
    let dj = daemon::read_daemon_json(&home)?;
    Ok(DaemonInfo {
        url: format!("ws://127.0.0.1:{}/ws", dj.port),
        token: daemon::read_token(&home)?,
        home: home.display().to_string(),
    })
}

/// Explicit user action from the waiting screen.
#[tauri::command]
fn start_daemon() -> Result<String, String> {
    match daemon::ensure_running(true)? {
        daemon::Ensure::AlreadyRunning => Ok("already running".into()),
        daemon::Ensure::Starting => Ok("starting".into()),
        daemon::Ensure::Started(p) => Ok(format!("started {}", p.display())),
    }
}

fn notify_file<R: Runtime>(app: &AppHandle<R>) -> Option<std::path::PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("notifications.json"))
}

/// Whether macOS will show notifications: unavailable (not running from Muster.app),
/// unknown (asked on first need), requesting, granted or denied.
#[tauri::command]
fn notify_status() -> notify::Status {
    notify::status()
}

#[tauri::command]
fn get_notify(flag: tauri::State<tray::NotifyFlag>) -> bool {
    flag.0.load(std::sync::atomic::Ordering::Relaxed)
}

#[tauri::command]
fn set_notify<R: Runtime>(app: AppHandle<R>, flag: tauri::State<tray::NotifyFlag>, on: bool) -> Result<(), String> {
    flag.0.store(on, std::sync::atomic::Ordering::Relaxed);
    let path = notify_file(&app).ok_or("no config directory")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::json!({ "enabled": on }).to_string()).map_err(|e| e.to_string())
}

fn island_file<R: Runtime>(app: &AppHandle<R>) -> Option<std::path::PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("island.json"))
}

fn read_enabled(path: Option<std::path::PathBuf>) -> bool {
    path.and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["enabled"].as_bool())
        .unwrap_or(true)
}

/// Whether the strip around the notch is switched on, and whether a display has a notch.
#[derive(serde::Serialize)]
struct IslandInfo {
    enabled: bool,
    available: bool,
}

#[tauri::command]
async fn get_island<R: Runtime>(app: AppHandle<R>) -> Result<IslandInfo, String> {
    let enabled = app.state::<tray::IslandFlag>().0.load(std::sync::atomic::Ordering::Relaxed);
    // Screens may only be asked about on the main thread.
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(island::available());
    })
    .map_err(|e| e.to_string())?;
    let available = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap_or(false);
    Ok(IslandInfo { enabled, available })
}

#[tauri::command]
fn set_island<R: Runtime>(app: AppHandle<R>, on: bool) -> Result<(), String> {
    app.state::<tray::IslandFlag>().0.store(on, std::sync::atomic::Ordering::Relaxed);
    tray::refresh_island(&app);
    let path = island_file(&app).ok_or("no config directory")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::json!({ "enabled": on }).to_string()).map_err(|e| e.to_string())
}

/// A card the tray asked to open before the window's page was ready.
#[derive(Default)]
struct PendingOpen(Mutex<Option<String>>);

#[tauri::command]
fn take_pending_open(pending: tauri::State<PendingOpen>) -> Option<String> {
    pending.0.lock().unwrap().take()
}

/// Shows the main window, recreating it (and its webview) if it was closed.
fn show_main<R: Runtime>(app: &AppHandle<R>) -> bool {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return true;
    }
    if let Some(cfg) = app.config().app.windows.iter().find(|w| w.label == "main") {
        if let Ok(b) = WebviewWindowBuilder::from_config(app, cfg) {
            if let Ok(w) = b.build() {
                let _ = w.set_focus();
            }
        }
    }
    false
}

pub(crate) fn on_tray_menu<R: Runtime>(app: &AppHandle<R>, id: &str) {
    #[cfg(debug_assertions)]
    eprintln!("muster: tray menu {id}");
    match id {
        "show" => {
            show_main(app);
        }
        // A click on the notch strip: a waiting or finished card, else the window.
        "island" => match tray::longest_waiting(app) {
            Some(card) => on_tray_menu(app, &format!("card:{card}")),
            None => {
                show_main(app);
            }
        },
        "quit" => app.exit(0),
        _ => {
            if let Some(card) = id.strip_prefix("card:") {
                if show_main(app) {
                    let _ = app.emit_to("main", "open-card", card.to_string());
                } else {
                    *app.state::<PendingOpen>().0.lock().unwrap() = Some(card.to_string());
                }
            }
        }
    }
}

/// The standard macOS menus, minus "Close Window": Cmd+W is left to the page, which
/// uses it to close the card modal. The Edit menu is what makes Cmd+C / Cmd+V reach
/// the webview (and so the terminal).
fn app_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let sep = || PredefinedMenuItem::separator(app);
    let muster = Submenu::with_items(
        app,
        "Muster",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &sep()?,
            &PredefinedMenuItem::services(app, None)?,
            &sep()?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &sep()?,
            // A plain item rather than the predefined one, so quitting goes through
            // app.exit() and is not mistaken for "last window closed".
            &MenuItem::with_id(app, "quit", "Quit Muster", true, Some("CmdOrCtrl+Q"))?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &sep()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(app, "View", true, &[&PredefinedMenuItem::fullscreen(app, None)?])?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[&PredefinedMenuItem::minimize(app, None)?, &PredefinedMenuItem::maximize(app, None)?],
    )?;
    Menu::with_items(app, &[&muster, &edit, &view, &window])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Start the daemon right away so it is up by the time the page asks.
    std::thread::spawn(|| {
        if let Err(e) = daemon::ensure_running(false) {
            eprintln!("muster: {e}");
        }
    });
    let app = tauri::Builder::default()
        .menu(app_menu)
        .on_menu_event(|app, ev| {
            if ev.id().as_ref() == "quit" {
                app.exit(0);
            }
        })
        .plugin(tauri_plugin_dialog::init())
        .manage(PendingOpen::default())
        .invoke_handler(tauri::generate_handler![
            daemon_info,
            open_url,
            start_daemon,
            take_pending_open,
            get_notify,
            set_notify,
            notify_status,
            get_island,
            set_island
        ])
        .setup(|app| {
            let enabled = read_enabled(notify_file(app.handle()));
            app.manage(tray::NotifyFlag(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(enabled))));
            let island_on = read_enabled(island_file(app.handle()));
            app.manage(tray::IslandFlag(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(island_on))));
            app.manage(tray::IslandCounts::default());
            let handle = app.handle().clone();
            island::init(move || {
                let h = handle.clone();
                let _ = handle.run_on_main_thread(move || on_tray_menu(&h, "island"));
            });
            let handle = app.handle().clone();
            notify::init(move |card| {
                let h = handle.clone();
                let _ = handle.run_on_main_thread(move || on_tray_menu(&h, &format!("card:{card}")));
            });
            tray::setup(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Muster");
    app.run(|app, event| match event {
        #[cfg(debug_assertions)]
        tauri::RunEvent::ExitRequested { code: Some(c), .. } => eprintln!("muster: exit requested ({c})"),
        // Closing the last window keeps the app (and the menu-bar item) running; the
        // window's webview is destroyed, so only the shell and its tray remain.
        // An explicit quit (menu, tray, Cmd+Q) carries an exit code and goes through.
        tauri::RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { has_visible_windows: false, .. } => {
            show_main(app);
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::is_web_url;

    #[test]
    fn only_web_links_open() {
        assert!(is_web_url("https://example.com/a?b=1#c"));
        assert!(is_web_url("http://localhost:3000"));
        for bad in [
            "file:///etc/passwd",
            "/Applications/Calculator.app",
            "-a Calculator",
            "https://",
            "http://-x",
            "https://a b",
            "ssh://h",
            "",
        ] {
            assert!(!is_web_url(bad), "{bad}");
        }
    }
}
