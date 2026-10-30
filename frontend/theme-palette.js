/**
 * Themeable Tailwind palette.
 *
 * Most of the UI is written with plain palette classes (bg-white, text-gray-900,
 * bg-blue-50 ...). Instead of adding a `dark:` twin to every one of them, the
 * palette itself is defined as CSS variables that `.dark` remaps:
 *
 * - gray (and white) are mirrored onto a hand-tuned dark scale, so surfaces,
 *   borders and text keep their light-mode roles (white = card, gray-50 = page,
 *   gray-900 = strong text).
 * - accent families mirror their shade numbers (50 <-> 950 ...), so tinted
 *   backgrounds become dark tints and 600/700 text becomes a readable 300/400.
 *
 * Because of this, `dark:` variants on palette colors are not needed (and would
 * be remapped twice); use them only for semantic shadcn tokens.
 *
 * The neon theme (`.dark.theme-neon`, high contrast) builds on dark mode: the
 * neutrals become a deep magenta scale with pale pink text, the blue family turns
 * purple, and borders turn neon green (globals.css), all at AA contrast or more.
 */
const defaultColors = require('tailwindcss/colors');

const SHADES = ['50', '100', '200', '300', '400', '500', '600', '700', '800', '900', '950'];

const ACCENT_FAMILIES = [
  'red', 'orange', 'amber', 'yellow', 'lime', 'green', 'emerald', 'teal', 'cyan',
  'sky', 'blue', 'indigo', 'violet', 'purple', 'fuchsia', 'pink', 'rose',
];
const NEUTRAL_FAMILIES = ['gray', 'slate', 'zinc', 'neutral', 'stone'];

/** Dark-mode accent shade used for each light-mode shade. */
const ACCENT_DARK_SHADE = {
  50: '950', 100: '900', 200: '800', 300: '700', 400: '400', 500: '400',
  600: '400', 700: '300', 800: '200', 900: '100', 950: '50',
};

/** Dark-mode neutral scale: same roles as light mode, mirrored. */
const DARK_NEUTRAL = {
  white: '#16181d', // cards / surfaces
  50: '#0e1013', // page background
  100: '#1d2026',
  200: '#2a2e36', // borders
  300: '#3b414b',
  400: '#5f6775',
  500: '#868e9c',
  600: '#a6adba',
  700: '#c4c9d2',
  800: '#dcdfe5',
  900: '#eceef2',
  950: '#f6f7f9',
};

/** Neon neutral scale: deep magenta surfaces, pale pink text (same roles as dark mode). */
const NEON_NEUTRAL = {
  white: '#2b0730', // cards / surfaces
  50: '#1c0320', // page background
  100: '#3a0b41',
  200: '#4d1257',
  300: '#66196f',
  400: '#c98bc0', // placeholders, icons: 7.3:1 on the page, 6.7:1 on cards
  500: '#e2a6d8',
  600: '#f1c3ea',
  700: '#f9daf4',
  800: '#fdeafa',
  900: '#fff4fd',
  950: '#ffffff',
};

/** Neon accents: the families the UI uses for links and focus become purple and pink. */
const NEON_FAMILY = { blue: 'fuchsia', indigo: 'violet', sky: 'purple', cyan: 'pink' };

function hexToChannels(hex) {
  const h = hex.replace('#', '');
  const full = h.length === 3 ? h.split('').map(c => c + c).join('') : h;
  const n = parseInt(full, 16);
  return `${(n >> 16) & 255} ${(n >> 8) & 255} ${n & 255}`;
}

const varName = (family, shade) => `--palette-${family}-${shade}`;
const ref = (family, shade) => `rgb(var(${varName(family, shade)}) / <alpha-value>)`;

function buildColors() {
  const colors = { white: ref('base', 'white') };
  for (const family of [...NEUTRAL_FAMILIES, ...ACCENT_FAMILIES]) {
    colors[family] = Object.fromEntries(SHADES.map(shade => [shade, ref(family, shade)]));
  }
  return colors;
}

function buildVariables() {
  const light = { [varName('base', 'white')]: hexToChannels('#ffffff') };
  const dark = { [varName('base', 'white')]: hexToChannels(DARK_NEUTRAL.white) };
  const neon = { [varName('base', 'white')]: hexToChannels(NEON_NEUTRAL.white) };
  for (const family of NEUTRAL_FAMILIES) {
    for (const shade of SHADES) {
      light[varName(family, shade)] = hexToChannels(defaultColors[family][shade]);
      dark[varName(family, shade)] = hexToChannels(DARK_NEUTRAL[shade]);
      neon[varName(family, shade)] = hexToChannels(NEON_NEUTRAL[shade]);
    }
  }
  for (const family of ACCENT_FAMILIES) {
    for (const shade of SHADES) {
      light[varName(family, shade)] = hexToChannels(defaultColors[family][shade]);
      dark[varName(family, shade)] = hexToChannels(defaultColors[family][ACCENT_DARK_SHADE[shade]]);
      neon[varName(family, shade)] = hexToChannels(defaultColors[NEON_FAMILY[family] ?? family][ACCENT_DARK_SHADE[shade]]);
    }
  }
  return { light, dark, neon };
}

/** Tailwind plugin that emits the light (:root), dark (.dark) and neon palette variables. */
function paletteVariablesPlugin({ addBase }) {
  const { light, dark, neon } = buildVariables();
  addBase({ ':root': light, '.dark': dark, '.dark.theme-neon': neon });
}

/** Typography (prose) colors bound to the themed gray scale. */
const proseColors = {
  '--tw-prose-body': 'rgb(var(--palette-gray-700))',
  '--tw-prose-headings': 'rgb(var(--palette-gray-900))',
  '--tw-prose-lead': 'rgb(var(--palette-gray-600))',
  '--tw-prose-links': 'rgb(var(--palette-blue-600))',
  '--tw-prose-bold': 'rgb(var(--palette-gray-900))',
  '--tw-prose-counters': 'rgb(var(--palette-gray-500))',
  '--tw-prose-bullets': 'rgb(var(--palette-gray-300))',
  '--tw-prose-hr': 'rgb(var(--palette-gray-200))',
  '--tw-prose-quotes': 'rgb(var(--palette-gray-900))',
  '--tw-prose-quote-borders': 'rgb(var(--palette-gray-200))',
  '--tw-prose-captions': 'rgb(var(--palette-gray-500))',
  '--tw-prose-kbd': 'rgb(var(--palette-gray-900))',
  '--tw-prose-code': 'rgb(var(--palette-gray-900))',
  '--tw-prose-pre-code': 'rgb(var(--palette-gray-200))',
  '--tw-prose-pre-bg': 'rgb(var(--palette-gray-800))',
  '--tw-prose-th-borders': 'rgb(var(--palette-gray-300))',
  '--tw-prose-td-borders': 'rgb(var(--palette-gray-200))',
};

module.exports = { buildColors, paletteVariablesPlugin, proseColors };
