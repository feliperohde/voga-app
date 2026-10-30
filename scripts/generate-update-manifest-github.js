#!/usr/bin/env node
/**
 * Writes the updater's latest.json for a version published in feliperohde/voga's Releases.
 *
 *   node scripts/generate-update-manifest-github.js <version> [bundle dir] [output] [notes]
 *
 * bundle dir: where the signed updater files are (default: target/release/bundle/macos). Each
 * <file>.sig there names one: Voga.app.tar.gz (macOS), the setup .exe or .msi (Windows), the
 * .AppImage (Linux). The architecture comes from the file name, else from this computer.
 * output: default latest.json. notes: default the version's entry in RELEASE_NOTES.md.
 *
 * The site repo's scripts/release.mjs runs it and uploads the result with the files it names.
 */

const fs = require('fs');
const path = require('path');

const REPO = 'feliperohde/voga';

const [versionArg, bundleDir = 'target/release/bundle/macos', output = 'latest.json', notesArg] = process.argv.slice(2);
if (!versionArg) {
  console.error('Usage: node generate-update-manifest-github.js <version> [bundle dir] [output] [notes]');
  process.exit(1);
}
const version = versionArg.replace(/^v/, '');

// "## [x.y.z] - date" up to the next "## [" heading
function releaseNotes(version) {
  const file = path.join(__dirname, '..', 'RELEASE_NOTES.md');
  if (!fs.existsSync(file)) return '';
  const text = fs.readFileSync(file, 'utf8');
  const start = text.indexOf(`\n## [${version}]`);
  if (start < 0) return '';
  const body = text.slice(text.indexOf('\n', start + 1) + 1);
  const next = body.search(/^## \[/m);
  return (next < 0 ? body : body.slice(0, next)).trim();
}

function os(file) {
  if (file.endsWith('.app.tar.gz')) return 'darwin';
  if (/\.(exe|msi)$/i.test(file)) return 'windows';
  if (/\.appimage$/i.test(file)) return 'linux';
  return null;
}

const HOST_ARCH = { arm64: 'aarch64', x64: 'x86_64', ia32: 'i686' };
function arch(file) {
  if (/aarch64|arm64/i.test(file)) return 'aarch64';
  if (/x86_64|x64|amd64/i.test(file)) return 'x86_64';
  if (/i686|x86/i.test(file)) return 'i686';
  return HOST_ARCH[process.arch];
}

if (!fs.existsSync(bundleDir)) {
  console.error(`No folder ${bundleDir}: build the release first (cd frontend && pnpm tauri:build)`);
  process.exit(1);
}

const platforms = {};
for (const sig of fs.readdirSync(bundleDir).filter((name) => name.endsWith('.sig')).sort()) {
  const file = sig.slice(0, -'.sig'.length);
  const key = os(file) && `${os(file)}-${arch(file)}`;
  if (!key || !fs.existsSync(path.join(bundleDir, file))) continue;
  if (platforms[key]) {
    console.warn(`${key}: keeping ${path.basename(platforms[key].url)}, skipping ${file}`);
    continue;
  }
  platforms[key] = {
    signature: fs.readFileSync(path.join(bundleDir, sig), 'utf8').trim(),
    url: `https://github.com/${REPO}/releases/download/v${version}/${encodeURIComponent(file)}`,
  };
  console.log(`${key}: ${file}`);
}

if (Object.keys(platforms).length === 0) {
  console.error(`No signed updater file (<file>.sig next to a .app.tar.gz, .exe, .msi or .AppImage) in ${bundleDir}`);
  process.exit(1);
}

const notes = notesArg || releaseNotes(version);
if (!notes) console.warn(`RELEASE_NOTES.md has no "## [${version}]" entry`);

const manifest = { version, notes: notes || `Voga ${version}`, pub_date: new Date().toISOString(), platforms };
fs.writeFileSync(output, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`\n${path.resolve(output)} written. Upload it as latest.json to the v${version} release, with the files above.`);
