//! Menu-bar item: count and list of cards that need you, Show / Quit, and session
//! totals fetched when the pointer reaches the icon (just before the menu opens).

use crate::watcher::{CardLite, Endpoint, Hooks, State, Watcher};
use crate::{daemon, island, notify};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

/// Whether to post macOS notifications; shared with the get/set commands.
#[derive(Clone)]
pub struct NotifyFlag(pub Arc<AtomicBool>);

/// Whether to show the strip around the notch; shared with the get/set commands.
#[derive(Clone)]
pub struct IslandFlag(pub Arc<AtomicBool>);

/// The cards the strip last showed.
#[derive(Default)]
pub struct IslandRows(Mutex<Vec<island::Row>>);

/// Redraws the strip from the last known cards (after the setting changed).
pub fn refresh_island<R: Runtime>(app: &AppHandle<R>) {
    let rows = app.state::<IslandRows>().0.lock().unwrap().clone();
    let on = app.state::<IslandFlag>().0.load(Ordering::Relaxed);
    let _ = app.run_on_main_thread(move || island::update(rows, on));
}

/// Which finished cards the strip keeps showing.
#[derive(Default)]
pub struct IslandFinished(Mutex<island::Finished>);

/// A card was clicked in the strip: a finished one stops showing.
pub fn island_dismiss<R: Runtime>(app: &AppHandle<R>, id: &str) {
    app.state::<IslandFinished>().0.lock().unwrap().dismiss(id);
    app.state::<IslandRows>().0.lock().unwrap().retain(|r| !(r.id == id && r.kind == island::Kind::Done));
    refresh_island(app);
}

/// The card a click on the strip itself opens: one that waits or has finished.
pub fn island_default<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let state = app.state::<IslandRows>();
    let rows = state.0.lock().unwrap();
    rows.first().filter(|r| r.kind != island::Kind::Working).map(|r| r.id.clone())
}

struct TrayHooks<R: Runtime> {
    app: AppHandle<R>,
    tray: Mutex<Option<TrayIcon<R>>>,
}

impl<R: Runtime> Hooks for TrayHooks<R> {
    fn changed(&self, st: &State) {
        let rows = self.app.state::<IslandFinished>().0.lock().unwrap().rows(
            st.cards.iter().map(|(id, c)| (id.as_str(), c.title.as_str(), c.status.as_str(), c.unread, c.status_since)),
            st.loaded,
        );
        *self.app.state::<IslandRows>().0.lock().unwrap() = rows;
        refresh_island(&self.app);
        let Some(tray) = self.tray.lock().unwrap().clone() else { return };
        let n = st.cards.values().filter(|c| c.status == "needs_you").count();
        let _ = tray.set_title(if n > 0 { Some(n.to_string()) } else { None::<String> });
        let _ = tray.set_tooltip(Some(if n > 0 { format!("Muster: {n} need you") } else { "Muster".into() }));
        if let Ok(menu) = build_menu(&self.app, st) {
            let _ = tray.set_menu(Some(menu));
        }
    }

    fn news(&self, card_id: &str, c: &CardLite, project: &str) {
        if c.status == "done" {
            self.app.state::<IslandFinished>().0.lock().unwrap().finished(card_id, c.status_since);
        }
        if !self.app.state::<NotifyFlag>().0.load(Ordering::Relaxed) {
            return;
        }
        if self.app.get_webview_window("main").is_some_and(|w| w.is_focused().unwrap_or(false)) {
            return;
        }
        // The card is the heading; what it did, or asks, is the body.
        let state = if c.status == "needs_you" { "Needs you" } else { "Finished" };
        let subtitle = if project.is_empty() { state.to_string() } else { format!("{state} · {project}") };
        notify::post(c.title.clone(), subtitle, c.detail.clone().unwrap_or_default(), card_id.into());
    }
}

pub fn setup<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let hooks = Arc::new(TrayHooks { app: app.clone(), tray: Mutex::new(None) });
    let watcher = Arc::new(Watcher::new(hooks.clone()));
    let want = watcher.want_stats.clone();
    let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    let tray = TrayIconBuilder::with_id("muster")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Muster")
        .menu(&build_menu(app, &State::default())?)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, ev| crate::on_tray_menu(app, ev.id().as_ref()))
        .on_tray_icon_event(move |_, ev| {
            // Hovering the icon precedes opening its menu: fetch fresh totals then.
            if matches!(ev, TrayIconEvent::Enter { .. } | TrayIconEvent::Click { .. }) {
                want.store(true, Ordering::SeqCst);
            }
        })
        .build(app)?;
    *hooks.tray.lock().unwrap() = Some(tray);

    std::thread::Builder::new()
        .name("muster-tray".into())
        .spawn(move || {
            watcher.run(|| {
                let home = daemon::muster_home()?;
                let dj = daemon::read_daemon_json(&home)?;
                Ok(Endpoint { url: format!("ws://127.0.0.1:{}/ws", dj.port), token: daemon::read_token(&home)? })
            })
        })
        .expect("spawn tray thread");
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn short(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", s / 60),
        3600..=86399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}

fn fmt_bytes(n: u64) -> String {
    const G: f64 = 1024.0 * 1024.0 * 1024.0;
    if n as f64 >= G {
        format!("{:.1} GB", n as f64 / G)
    } else {
        format!("{} MB", n / (1024 * 1024))
    }
}

fn build_menu<R: Runtime>(app: &AppHandle<R>, st: &State) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    let mut needs: Vec<(&String, &CardLite)> = st.cards.iter().filter(|(_, c)| c.status == "needs_you").collect();
    needs.sort_by_key(|(_, c)| c.status_since);
    let header = if !st.connected {
        "Daemon not connected".to_string()
    } else if needs.is_empty() {
        "Nothing needs you".to_string()
    } else {
        format!("{} need{} you", needs.len(), if needs.len() == 1 { "s" } else { "" })
    };
    menu.append(&MenuItem::with_id(app, "header", header, false, None::<&str>)?)?;
    let now = now_ms();
    for (id, c) in needs.iter().take(20) {
        let project = st.projects.get(&c.project_id).map(String::as_str).unwrap_or("");
        let mut title: String = c.title.chars().take(48).collect();
        if c.title.chars().count() > 48 {
            title.push('…');
        }
        let label = format!("{title}  —  {project} · waiting {}", short(now - c.status_since));
        menu.append(&MenuItem::with_id(app, format!("card:{id}"), label, true, None::<&str>)?)?;
    }
    if st.connected {
        if let Some((mem, cpu)) = st.total {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
            let label = format!("Sessions: {} · {:.0}% CPU", fmt_bytes(mem), cpu);
            menu.append(&MenuItem::with_id(app, "total", label, false, None::<&str>)?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "show", "Show Muster", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit Muster", true, Some("CmdOrCtrl+Q"))?)?;
    Ok(menu)
}
