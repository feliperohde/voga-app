const path = require('path');
const { PHASE_DEVELOPMENT_SERVER } = require('next/constants');

/** @type {import('next').NextConfig} */
const nextConfig = {
  output: 'export',
  images: { unoptimized: true },
  webpack: (config, { isServer }) => {
    // Mock mode (scripts/mock.js): app commands answered by src/mocks with demo data.
    if (process.env.NEXT_PUBLIC_MOCK === '1') {
      config.resolve.alias = {
        ...config.resolve.alias,
        '@tauri-apps/api/core$': path.resolve(__dirname, 'src/mocks/tauri-core.ts'),
        '@tauri-real/core$': path.join(path.dirname(require.resolve('@tauri-apps/api/package.json')), 'core.js'),
      };
      // Real and mock builds share .next/cache, whose key doesn't see this alias: without a
      // version of its own, a build could reuse the other mode's @tauri-apps/api/core.
      if (config.cache) config.cache.version = `${config.cache.version}|mock`;
    }
    if (!isServer) {
      config.resolve.fallback = { ...config.resolve.fallback, fs: false, path: false, os: false };
    }
    return config;
  },
};

// Mock mode keeps its own dirs: the dev cache (.next-mock) survives `build:mock`, and
// the static demo (out-mock) never ends up in the Tauri bundle (out).
module.exports = (phase) => {
  if (process.env.NEXT_PUBLIC_MOCK !== '1') return nextConfig;
  return { ...nextConfig, distDir: phase === PHASE_DEVELOPMENT_SERVER ? '.next-mock' : 'out-mock' };
};
