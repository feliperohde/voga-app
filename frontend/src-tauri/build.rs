// The ONNX Runtime DLL live sessions load on Windows (speech detection, voices)
#[cfg(windows)]
#[path = "build/onnxruntime.rs"]
mod onnxruntime;

fn main() {
    println!("cargo:rerun-if-env-changed=VOGA_FRESH_INSTALL");
    #[cfg(windows)]
    onnxruntime::stage();
    // Live sessions hear the computer's sound through Core Audio's process taps (macOS 14.2+).
    // Linked weakly, so Voga still opens on older macOS, where it does not offer that sound.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,CoreAudio");
    }
    tauri_build::build()
}
