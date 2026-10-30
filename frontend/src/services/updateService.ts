import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

/** Where an install is: the download (bytes so far / total, 0 when unknown), then the install. */
export type InstallStep = { phase: 'download'; received: number; total: number } | { phase: 'install' };

/**
 * App updates through the Tauri updater: tauri.conf.json points it at the latest.json of the
 * releases (scripts/generate-update-manifest-github.js), signed with ~/.voga/updater.key.
 */
export const updateService = {
  /** The newer version (current, version, date, body = release notes), or null when this is the latest. */
  find(): Promise<Update | null> {
    return check();
  },

  /** Downloads and installs `update`, then restarts Voga. */
  async install(update: Update, onStep: (step: InstallStep) => void): Promise<void> {
    let received = 0;
    let total = 0;
    await update.downloadAndInstall(event => {
      if (event.event === 'Started') total = event.data.contentLength ?? 0;
      if (event.event === 'Progress') received += event.data.chunkLength;
      onStep(event.event === 'Finished' ? { phase: 'install' } : { phase: 'download', received, total });
    });
    await relaunch();
  },
};
