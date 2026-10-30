//! Settings › AI and the first run: the built-in models, their downloads, and what this computer can run.

use serde::Serialize;
use tauri::{AppHandle, Runtime};

use super::catalog::{self, ModelKind};
use super::files::{self, Status};

/// A built-in model as the UI lists it (services/builtinService.ts BuiltinModel).
#[derive(Debug, Serialize)]
pub struct ModelView {
    name: &'static str,
    display_name: &'static str,
    status: Status,
    size_mb: u64,
    description: &'static str,
    kind: ModelKind,
}

fn known(name: &str) -> Result<&'static catalog::Model, String> {
    catalog::find(name).ok_or_else(|| format!("Unknown built-in model: {name}"))
}

/// `kind`: summary (chat, the default), embedding, transcription or voice. Recommended first.
#[tauri::command]
pub async fn builtin_ai_list_models(kind: Option<String>) -> Result<Vec<ModelView>, String> {
    Ok(catalog::of_kind(ModelKind::parse(kind.as_deref()))
        .map(|model| ModelView {
            name: model.name,
            display_name: model.label,
            status: files::status(model),
            size_mb: model.size_mb,
            description: model.description,
            kind: model.kind,
        })
        .collect())
}

/// Resolves when the file is downloaded and checked; fails with "CANCELLED" when cancelled.
#[tauri::command]
pub async fn builtin_ai_download_model<R: Runtime>(app: AppHandle<R>, model_name: String) -> Result<(), String> {
    files::download(&app, known(&model_name)?).await
}

#[tauri::command]
pub async fn builtin_ai_cancel_download(model_name: String) -> Result<(), String> {
    files::cancel(&model_name);
    Ok(())
}

#[tauri::command]
pub async fn builtin_ai_delete_model(model_name: String) -> Result<(), String> {
    if crate::live::session::model_in_use(&model_name) {
        return Err("A live session is using this model: stop it first".to_string());
    }
    files::delete(known(&model_name)?).await.map_err(|e| e.to_string())
}

/// The chat model for this computer's memory.
#[tauri::command]
pub async fn builtin_ai_get_recommended_model() -> Result<String, String> {
    Ok(catalog::recommended_chat(ram_gb()).to_string())
}

/// This computer as the built-in models see it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerProfile {
    pub ram_gb: u64,
    /// A GPU the built-in models and Whisper run on (Apple Silicon, or a CUDA/Vulkan build).
    pub gpu: bool,
    /// Too modest for the built-in models to run well: an external AI is recommended.
    pub basic: bool,
}

/// Little memory, or no GPU without plenty of it: the built-in models would be slow here.
pub(crate) fn is_basic(ram_gb: u64, gpu: bool) -> bool {
    ram_gb < 8 || (!gpu && ram_gb < 16)
}

#[tauri::command]
pub async fn builtin_ai_computer_profile() -> Result<ComputerProfile, String> {
    let ram_gb = ram_gb();
    let gpu = cfg!(any(all(target_os = "macos", target_arch = "aarch64"), feature = "whisper-cuda", feature = "whisper-vulkan"));
    Ok(ComputerProfile { ram_gb, gpu, basic: is_basic(ram_gb, gpu) })
}

fn ram_gb() -> u64 {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    system.total_memory() / (1024 * 1024 * 1024)
}

#[cfg(test)]
mod tests {
    #[test]
    fn modest_computers_are_told_apart() {
        assert!(super::is_basic(4, true), "4 GB is too little even with a GPU");
        assert!(super::is_basic(8, false), "8 GB without a GPU");
        assert!(!super::is_basic(8, true), "Apple Silicon with 8 GB runs the small models");
        assert!(!super::is_basic(32, false), "plenty of memory makes up for the processor");
    }
}
