//! QA builds made with `--fresh` (`pnpm tauri:build --fresh`) open as a first installation:
//! the first run of each such build erases what the app keeps on this computer (database,
//! trial, keys, settings, webview storage). Later runs of the same build keep their data.

use std::{fs, io::ErrorKind};

pub fn wipe_once(identifier: &str) {
    let Some(stamp) = option_env!("VOGA_FRESH_INSTALL") else { return };
    let Some(data) = dirs::data_dir().map(|d| d.join(identifier)) else { return };
    let marker = data.join(".fresh-install");
    if fs::read_to_string(&marker).is_ok_and(|s| s == stamp) {
        return;
    }

    // The same folders Tauri's app_*_dir() resolve to
    let mut targets: Vec<_> = [dirs::config_dir(), dirs::data_local_dir(), dirs::cache_dir()]
        .into_iter()
        .flatten()
        .map(|d| d.join(identifier))
        .chain([data.clone()])
        .collect();
    #[cfg(target_os = "macos")]
    if let Some(library) = dirs::home_dir().map(|h| h.join("Library")) {
        // localStorage and cookies of the webview
        targets.push(library.join("WebKit").join(identifier));
        targets.push(library.join("HTTPStorages").join(identifier));
    }

    for dir in &targets {
        match fs::remove_dir_all(dir) {
            Ok(()) => log::warn!("Fresh install: removed {}", dir.display()),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => log::error!("Fresh install: couldn't remove {}: {}", dir.display(), e),
        }
    }
    if let Err(e) = fs::create_dir_all(&data).and_then(|_| fs::write(&marker, stamp)) {
        log::error!("Fresh install: couldn't write {}: {}", marker.display(), e);
    }
}
