import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

/** summary: chat models (answers and analysis); embedding: the search index; transcription and
 * voice: live sessions (Whisper, telling voices apart). */
export type BuiltinModelKind = 'summary' | 'embedding' | 'transcription' | 'voice';

export type BuiltinModelStatus =
  | { type: 'not_downloaded' }
  | { type: 'downloading'; progress: number }
  | { type: 'available' }
  /** A file of the wrong size: a download that stopped halfway (downloading again resumes it) */
  | { type: 'corrupted' };

/** A built-in model (llama.cpp on this computer), as the Rust core reports it. */
export interface BuiltinModel {
  name: string;
  display_name: string;
  status: BuiltinModelStatus;
  size_mb: number;
  description: string;
  kind: BuiltinModelKind;
}

export interface DownloadProgress {
  model: string;
  progress: number;
  downloaded_mb?: number;
  total_mb?: number;
  speed_mbps?: number;
  status: 'downloading' | 'completed' | 'error' | 'cancelled';
  error?: string;
}

/** This computer as the built-in models see it. */
export interface ComputerProfile {
  ramGb: number;
  /** A GPU the built-in models run on (Apple Silicon, or a CUDA/Vulkan build). */
  gpu: boolean;
  /** Too modest for the built-in models: an external AI is recommended. */
  basic: boolean;
}

export const builtinService = {
  computerProfile: () => invoke<ComputerProfile>('builtin_ai_computer_profile'),
  /** In the catalog's order, recommended first. */
  list: (kind: BuiltinModelKind) => invoke<BuiltinModel[]>('builtin_ai_list_models', { kind }),
  /** Resolves when the file is downloaded and checked. */
  download: (modelName: string) => invoke<void>('builtin_ai_download_model', { modelName }),
  cancel: (modelName: string) => invoke<void>('builtin_ai_cancel_download', { modelName }),
  remove: (modelName: string) => invoke<void>('builtin_ai_delete_model', { modelName }),
  /** The chat model for this computer's memory. */
  recommended: () => invoke<string>('builtin_ai_get_recommended_model'),
  onProgress: (handler: (progress: DownloadProgress) => void): Promise<UnlistenFn> =>
    listen<DownloadProgress>('builtin-ai-download-progress', event => handler(event.payload)),
};

export const isAvailable = (model: BuiltinModel) => model.status.type === 'available';

/** "1.2 GB" / "605 MB" */
export function formatSize(mb: number): string {
  return mb >= 1000 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`;
}
