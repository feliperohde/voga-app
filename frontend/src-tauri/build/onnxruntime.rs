//! Windows only: live sessions load onnxruntime.dll (speech detection, voices), which the bundle
//! ships from binaries/onnxruntime (tauri.windows.conf.json). This takes it from Microsoft's
//! pinned release once and checks every file against its SHA-256; a matching folder is left alone.

use sha2::{Digest, Sha256};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

const TARGET: &str = "x86_64-pc-windows-msvc";
const ZIP_URL: &str = "https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-win-x64-1.22.0.zip";
const ZIP_SHA256: &str = "174c616efc0271194488642a72f1a514e01487da4dfe84c49296d66e40ebe0da";

/// (path in the release's ZIP, name in binaries/onnxruntime, SHA-256)
const FILES: [(&str, &str, &str); 3] = [
    (
        "onnxruntime-win-x64-1.22.0/lib/onnxruntime.dll",
        "onnxruntime.dll",
        "579b636403983254346a5c1d80bd28f1519cd1e284cd204f8d4ff41f8d711559",
    ),
    (
        "onnxruntime-win-x64-1.22.0/lib/onnxruntime_providers_shared.dll",
        "onnxruntime_providers_shared.dll",
        "ba00ea1ef846c9b909c7854bc56c51051a20f9773b3e1153dda118d4b85d0b93",
    ),
    (
        "onnxruntime-win-x64-1.22.0/LICENSE",
        "onnxruntime-LICENSE.txt",
        "c250d6278f0b47a6439fb7592b08b58a55eb9f535aa49a1db63211c3f982b674",
    ),
];

pub fn stage() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let target = env::var("TARGET").unwrap_or_default();
    assert!(target == TARGET, "ONNX Runtime is bundled for {TARGET} only, not {target}");

    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("binaries")
        .join("onnxruntime");
    println!("cargo:rerun-if-changed={}", dir.display());
    if FILES.iter().all(|(_, name, sha)| matches(&dir.join(name), sha)) {
        return;
    }
    let zip = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("onnxruntime.zip");
    if let Err(error) = fill(&dir, &zip) {
        panic!("ONNX Runtime for the Windows bundle: {error}");
    }
    println!("cargo:warning=ONNX Runtime 1.22.0 put in {}", dir.display());
}

fn fill(dir: &Path, zip: &Path) -> Result<(), String> {
    if !matches(zip, ZIP_SHA256) {
        download(zip)?;
        if !matches(zip, ZIP_SHA256) {
            let _ = fs::remove_file(zip);
            return Err(format!("{ZIP_URL} does not have the expected SHA-256 {ZIP_SHA256}"));
        }
    }
    let file = fs::File::open(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", zip.display()))?;

    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Everything in the folder is bundled: refuse what is not ours rather than ship or delete it
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let name = entry.map_err(|e| e.to_string())?.file_name();
        if !FILES.iter().any(|(_, ours, _)| name.as_os_str() == *ours) {
            return Err(format!("{} also has {name:?}, which the app would ship: remove it", dir.display()));
        }
    }

    for (inner, name, sha) in FILES {
        let part = dir.join(format!("{name}.part"));
        let written = archive
            .by_name(inner)
            .map_err(|e| format!("{inner}: {e}"))
            .and_then(|mut source| {
                let mut out = fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
                io::copy(&mut source, &mut out).map_err(|e| format!("{inner}: {e}"))
            });
        if let Err(error) = written.and_then(|_| {
            if matches(&part, sha) { Ok(()) } else { Err(format!("{inner} does not have the expected SHA-256 {sha}")) }
        }) {
            let _ = fs::remove_file(&part);
            return Err(error);
        }
        fs::rename(&part, dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

fn download(to: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(ZIP_URL)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("{ZIP_URL}: {e}"))?;
    let mut out = fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
    io::copy(&mut response, &mut out).map_err(|e| format!("{ZIP_URL}: {e}"))?;
    Ok(())
}

/// The file is there and its SHA-256 is `sha`.
fn matches(path: &Path, sha: &str) -> bool {
    let Ok(mut file) = fs::File::open(path) else { return false };
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).is_ok() && format!("{:x}", hasher.finalize()) == sha
}
