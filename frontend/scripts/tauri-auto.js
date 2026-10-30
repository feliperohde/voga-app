#!/usr/bin/env node
/**
 * Runs the Tauri CLI with this build's options.
 *
 *   node scripts/tauri-auto.js dev|build [options] [tauri arguments]
 *   node scripts/tauri-auto.js name [options]     # prints the app name for these options
 *
 * Build options, as flags so package.json scripts work on every platform:
 *   --mock                  demo build with sample data (src-tauri/tauri.mock.conf.json)
 *   --fresh                 QA build: its first run erases the app's data on that computer and opens
 *                           as a first installation (VOGA_FRESH_INSTALL, see src-tauri/src/fresh_install.rs)
 *   --llama=cuda|vulkan|cpu GPU of the built-in models and of the live transcription (Whisper)
 *                           (VOGA_LLAMA_FEATURE; default: metal on macOS, cpu elsewhere)
 * Other arguments go to tauri (e.g. --bundles app).
 *
 * dev and build first build the built-in models' sidecar (llama-helper, llama.cpp) and put it in
 * src-tauri/binaries, where the bundle takes it from.
 *
 * Release builds sign their updater artifacts with ~/.voga/updater.key (and its .password file)
 * unless TAURI_SIGNING_PRIVATE_KEY is already set.
 *
 * The demo gets its own app name, and so its own file: Voga, Voga-demo.
 */

const { execSync, spawnSync } = require('child_process');
const fs = require('fs');
const path = require('path');
const os = require('os');

const command = process.argv[2];
if (!command || !['dev', 'build', 'name'].includes(command)) {
  console.error('Usage: node tauri-auto.js dev|build|name [--mock] [--fresh] [--llama=cuda|vulkan|cpu]');
  process.exit(1);
}

const env = { ...process.env };
const OPTIONS = { '--llama=': 'VOGA_LLAMA_FEATURE' };
let mock = false;
const tauriArgs = [];
for (const arg of process.argv.slice(3)) {
  const option = Object.keys(OPTIONS).find(prefix => arg.startsWith(prefix));
  if (arg === '--mock') {
    mock = true;
  } else if (arg === '--fresh') {
    // A new value per build: each build erases the data once, on its first run
    env.VOGA_FRESH_INSTALL = String(Date.now());
    if (command !== 'name') console.log('🧹 Fresh install: the first run of this build erases the app data on that computer');
  } else if (option) {
    env[OPTIONS[option]] = arg.slice(option.length);
    if (command !== 'name') console.log(`🧩 ${OPTIONS[option]}=${env[OPTIONS[option]]}`);
  } else {
    tauriArgs.push(arg);
  }
}

/** The app name for this build: the standard one keeps "Voga", the demo says what it is. */
function buildName() {
  return mock ? 'Voga-demo' : 'Voga';
}

if (command === 'name') {
  console.log(buildName());
  process.exit(0);
}

/**
 * Builds llama-helper (always optimized: a debug llama.cpp is too slow to use) and copies it to
 * src-tauri/binaries/llama-helper-<target triple>, the name Tauri's externalBin expects.
 */
function buildSidecar() {
  const root = path.join(__dirname, '..', '..');
  const triple = execSync('rustc -vV', { encoding: 'utf8' }).match(/^host: (\S+)$/m)[1];
  const requested = env.VOGA_LLAMA_FEATURE ?? (process.platform === 'darwin' ? 'metal' : 'cpu');
  const feature = requested === 'cpu' ? '' : requested;
  const args = ['build', '-p', 'llama-helper', '--release', ...(feature ? ['--features', feature] : [])];
  console.log(`🦙 Building the built-in models' sidecar (${feature || 'cpu'})`);
  const built = spawnSync('cargo', args, { stdio: 'inherit', env, cwd: root });
  if (built.status !== 0) process.exit(built.status ?? 1);

  const exe = process.platform === 'win32' ? '.exe' : '';
  const targetDir = env.CARGO_TARGET_DIR || path.join(root, 'target');
  const binaries = path.join(__dirname, '..', 'src-tauri', 'binaries');
  fs.mkdirSync(binaries, { recursive: true });
  fs.copyFileSync(path.join(targetDir, 'release', `llama-helper${exe}`), path.join(binaries, `llama-helper-${triple}${exe}`));
}

buildSidecar();

if (command === 'build' && !env.TAURI_SIGNING_PRIVATE_KEY) {
  const key = path.join(os.homedir(), '.voga', 'updater.key');
  if (fs.existsSync(key)) {
    env.TAURI_SIGNING_PRIVATE_KEY = fs.readFileSync(key, 'utf8');
    const password = `${key}.password`;
    if (fs.existsSync(password)) env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = fs.readFileSync(password, 'utf8');
    console.log('✍️  Updater artifacts signed with ~/.voga/updater.key');
  } else {
    console.warn('⚠️  No ~/.voga/updater.key: the updater artifacts cannot be signed (tauri signer generate)');
  }
}

// Whisper runs in the app itself: the same GPU as the sidecar (Metal is always on for macOS)
if (['cuda', 'vulkan'].includes(env.VOGA_LLAMA_FEATURE)) tauriArgs.push('--features', `whisper-${env.VOGA_LLAMA_FEATURE}`);

// Configurations merge in order: the demo's, then this build's name
if (mock) tauriArgs.push('--config', 'src-tauri/tauri.mock.conf.json');
if (command === 'build') {
  const name = buildName();
  tauriArgs.push('--config', JSON.stringify({ productName: name }));
  console.log(`📦 App: ${name}`);
}

// Run the tauri CLI with node directly, without a shell, so the JSON argument arrives intact everywhere
const args = [path.join(__dirname, '..', 'node_modules', '@tauri-apps', 'cli', 'tauri.js'), command, ...tauriArgs];
const result = spawnSync(process.execPath, args, { stdio: 'inherit', env });

// Tauri empties bundle/dmg on every build: keep each DMG in bundle/dmg-all, so builds run in a row all stay
if (command === 'build' && result.status === 0) {
  const targetDir = env.CARGO_TARGET_DIR || path.join(__dirname, '..', '..', 'target');
  const bundle = path.join(targetDir, tauriArgs.includes('--debug') ? 'debug' : 'release', 'bundle');
  const dmgs = fs.existsSync(path.join(bundle, 'dmg')) ? fs.readdirSync(path.join(bundle, 'dmg')).filter(f => f.endsWith('.dmg')) : [];
  if (dmgs.length) fs.mkdirSync(path.join(bundle, 'dmg-all'), { recursive: true });
  for (const dmg of dmgs) {
    fs.copyFileSync(path.join(bundle, 'dmg', dmg), path.join(bundle, 'dmg-all', dmg));
    console.log(`💾 Kept ${path.join(bundle, 'dmg-all', dmg)}`);
  }
}
process.exit(result.status ?? 1);
