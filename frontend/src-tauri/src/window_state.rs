//! Remembers the main window's size and position between launches.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime};
use tauri_plugin_store::StoreExt;

const STORE_FILE: &str = "window_state.json";
const MAIN_KEY: &str = "main";
/// Smaller than this is not a usable window (e.g. saved while minimized).
const MIN_SIZE: (f64, f64) = (480.0, 360.0);

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    #[serde(default)]
    maximized: bool,
}

/// Puts the main window back where and how big the user left it, when that
/// still fits on a connected screen.
pub fn restore<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else { return };
    let Some(bounds) = app
        .store(STORE_FILE)
        .ok()
        .and_then(|store| store.get(MAIN_KEY))
        .and_then(|value| serde_json::from_value::<Bounds>(value).ok())
    else {
        return;
    };
    if bounds.width < MIN_SIZE.0 || bounds.height < MIN_SIZE.1 {
        return;
    }
    let visible = app.available_monitors().map(|monitors| {
        monitors.iter().any(|monitor| {
            let scale = monitor.scale_factor();
            let origin: LogicalPosition<f64> = monitor.position().to_logical(scale);
            let (w, h) = (monitor.size().width as f64 / scale, monitor.size().height as f64 / scale);
            // The title bar must be on this screen
            bounds.x + 40.0 < origin.x + w && bounds.x + bounds.width > origin.x + 40.0 && bounds.y >= origin.y && bounds.y < origin.y + h - 40.0
        })
    });
    let _ = window.set_size(LogicalSize::new(bounds.width, bounds.height));
    if visible.unwrap_or(false) {
        let _ = window.set_position(LogicalPosition::new(bounds.x, bounds.y));
    }
    if bounds.maximized {
        let _ = window.maximize();
    }
}

/// Saves the main window's bounds after it was resized or moved (the last of a burst).
pub fn remember<R: Runtime>(app: &AppHandle<R>) {
    static LATEST: AtomicU64 = AtomicU64::new(0);
    let call = LATEST.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if LATEST.load(Ordering::SeqCst) != call {
            return;
        }
        let Some(window) = app.get_webview_window("main") else { return };
        if window.is_minimized().unwrap_or(false) || !window.is_visible().unwrap_or(true) {
            return;
        }
        let maximized = window.is_maximized().unwrap_or(false);
        let (Ok(scale), Ok(position), Ok(size)) = (window.scale_factor(), window.outer_position(), window.inner_size()) else {
            return;
        };
        let position: LogicalPosition<f64> = position.to_logical(scale);
        let size: LogicalSize<f64> = size.to_logical(scale);
        let store = match app.store(STORE_FILE) {
            Ok(store) => store,
            Err(e) => return log::warn!("Window state: cannot open store: {}", e),
        };
        // A maximized window keeps the size it had before, to come back to
        let previous = store.get(MAIN_KEY).and_then(|v| serde_json::from_value::<Bounds>(v).ok());
        let bounds = match (maximized, previous) {
            (true, Some(previous)) => Bounds { maximized: true, ..previous },
            _ => Bounds { x: position.x, y: position.y, width: size.width, height: size.height, maximized },
        };
        if let Ok(value) = serde_json::to_value(bounds) {
            store.set(MAIN_KEY, value);
            let _ = store.save();
        }
    });
}
