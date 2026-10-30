/**
 * Stand-in for `@tauri-apps/api/core` in mock mode (NEXT_PUBLIC_MOCK=1, wired in
 * next.config.js). App commands are answered by ./handlers; `plugin:*` commands
 * (events, windows, store...) go to the real IPC inside the Tauri app, or to
 * ./browser when the page runs in a plain browser.
 */

import * as real from '@tauri-real/core';
import type { InvokeArgs, InvokeOptions } from '@tauri-real/core';
import { installBrowserIpc } from './browser';
import { fallback, handlers } from './handlers';

export * from '@tauri-real/core';

const LATENCY_MS = 40;
const warned = new Set<string>();

if (typeof window !== 'undefined') {
  if (!real.isTauri()) installBrowserIpc();
  console.info(
    '%c Voga mock mode %c dados de demonstração — nada é lido ou gravado no banco real',
    'background:#13343b;color:#f3ead8;border-radius:3px;padding:1px 4px',
    'color:inherit',
  );
}

async function mockInvoke(cmd: string, args: Record<string, any>): Promise<unknown> {
  await new Promise(resolve => setTimeout(resolve, LATENCY_MS));
  const handler = handlers[cmd];
  if (!handler) {
    if (!warned.has(cmd)) {
      warned.add(cmd);
      console.warn(`[mock] no handler for "${cmd}", returning a default`, args);
    }
    return fallback(cmd);
  }
  const result = await handler(args);
  // Same as crossing the IPC boundary: callers never share objects with the mock state
  return result === undefined ? null : structuredClone(result);
}

export async function invoke<T>(cmd: string, args: InvokeArgs = {}, options?: InvokeOptions): Promise<T> {
  if (cmd.startsWith('plugin:')) return real.invoke<T>(cmd, args, options);
  return (await mockInvoke(cmd, args as Record<string, any>)) as T;
}
