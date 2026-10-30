//! Menu bar icon: Voga keeps collecting with its window closed.

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime};

pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "show", "Abrir o Voga", true, None::<&str>)?,
            &MenuItem::with_id(app, "settings", "Ajustes", true, None::<&str>)?,
            // components/UpdateProvider.tsx
            &MenuItem::with_id(app, "updates", "Procurar atualizações", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?,
        ],
    )?;
    let Some(icon) = app.default_window_icon().cloned() else {
        return Ok(log::warn!("No app icon for the menu bar"));
    };
    TrayIconBuilder::with_id("voga").icon(icon).tooltip("Voga").menu(&menu).on_menu_event(chosen).build(app)?;
    Ok(())
}

fn chosen<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let script = match event.id.as_ref() {
        "quit" => return app.exit(0),
        "settings" => Some("window.location.assign('/settings')"),
        "updates" => Some("window.dispatchEvent(new Event('voga:check-updates'))"),
        _ => None,
    };
    show_window(app);
    if let (Some(script), Some(window)) = (script, app.get_webview_window("main")) {
        let _ = window.eval(script);
    }
}

/// Brings the main window back, minimized or hidden.
pub fn show_window<R: Runtime>(app: &AppHandle<R>) {
    match app.get_webview_window("main") {
        Some(window) => {
            let _ = window.show().and_then(|_| window.unminimize()).and_then(|_| window.set_focus());
        }
        None => log::warn!("The main window is gone"),
    }
}
