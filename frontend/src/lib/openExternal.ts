import { openUrl } from '@tauri-apps/plugin-opener';

/** Opens a web address in the default browser (a new tab when running outside the app). */
export function openExternal(url: string) {
  openUrl(url).catch(() => window.open(url, '_blank', 'noopener'));
}
