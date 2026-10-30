/**
 * Outside the Tauri app there is no IPC at all: this fakes the Tauri internals
 * (events, current window, plugins) so the frontend runs in a regular browser.
 */

import { mockConvertFileSrc, mockIPC, mockWindows } from '@tauri-apps/api/mocks';

const DATA_DIR = '/Users/demo/Library/Application Support/com.voga.app';

const stores = new Map<number, Map<string, unknown>>();
let nextRid = 1;

function store(rid: number) {
  let entries = stores.get(rid);
  if (!entries) {
    entries = new Map();
    stores.set(rid, entries);
  }
  return entries;
}

// Event bus. Not mockIPC's `shouldMockEvents`: its unlisten reads `args.id` while
// @tauri-apps/api sends `eventId`, so removed listeners keep getting events.
const listeners = new Map<string, number[]>();

function plugin(cmd: string, args: Record<string, any>): unknown {
  switch (cmd) {
    case 'plugin:event|listen':
      listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]);
      return args.handler;
    case 'plugin:event|unlisten':
      listeners.set(args.event, (listeners.get(args.event) ?? []).filter(id => id !== args.eventId));
      return null;
    case 'plugin:event|emit':
    case 'plugin:event|emit_to':
      for (const id of listeners.get(args.event) ?? []) {
        (window as any).__TAURI_INTERNALS__.runCallback(id, { event: args.event, id, payload: args.payload });
      }
      return null;
    case 'plugin:app|version':
      return '0.1.0';
    case 'plugin:app|name':
      return 'Voga';
    case 'plugin:app|tauri_version':
      return '2.11.0';
    case 'plugin:path|resolve_directory':
      return DATA_DIR;
    case 'plugin:path|join':
      return (args.paths as string[]).join('/');
    case 'plugin:store|load':
    case 'plugin:store|get_store': {
      const rid = nextRid++;
      store(rid);
      return rid;
    }
    case 'plugin:store|get':
      return [store(args.rid).get(args.key) ?? null, store(args.rid).has(args.key)];
    case 'plugin:store|has':
      return store(args.rid).has(args.key);
    case 'plugin:store|set':
      store(args.rid).set(args.key, args.value);
      return null;
    case 'plugin:store|delete':
      return store(args.rid).delete(args.key);
    case 'plugin:store|keys':
      return [...store(args.rid).keys()];
    case 'plugin:store|values':
      return [...store(args.rid).values()];
    case 'plugin:store|entries':
      return [...store(args.rid).entries()];
    case 'plugin:store|length':
      return store(args.rid).size;
    case 'plugin:os|locale':
      return navigator.language;
    case 'plugin:notification|is_permission_granted':
      return false;
    default:
      // Window controls, updater check, process, notifications: nothing to do in a browser
      return null;
  }
}

export function installBrowserIpc() {
  mockWindows('main');
  mockConvertFileSrc('macos');
  mockIPC((cmd, args) => plugin(cmd, (args ?? {}) as Record<string, any>));
  (window as any).__TAURI_OS_PLUGIN_INTERNALS__ = {
    platform: 'macos',
    os_type: 'macos',
    family: 'unix',
    version: '15.0',
    arch: 'aarch64',
    exe_extension: '',
    eol: '\n',
  };
}
