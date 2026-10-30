#!/usr/bin/env node
/**
 * Runs a command with NEXT_PUBLIC_MOCK=1, so the frontend answers Tauri commands
 * with the demo data in src/mocks instead of the Rust core. Cross-platform
 * stand-in for `NEXT_PUBLIC_MOCK=1 <command>`.
 *
 *   node scripts/mock.js next dev -p 3128
 */

const { spawnSync } = require('child_process');

const [command, ...args] = process.argv.slice(2);
if (!command) {
  console.error('Usage: node scripts/mock.js <command> [args...]');
  process.exit(1);
}

const result = spawnSync(command, args, {
  stdio: 'inherit',
  shell: true,
  env: { ...process.env, NEXT_PUBLIC_MOCK: '1' },
});
process.exit(result.status ?? 1);
