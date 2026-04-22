import { defineConfig } from 'vite';

// Dev is served by esbuild via `node dev.mjs` (see package.json "dev").
// This config only runs for `vite build`. JSX is handled by Vite's built-in
// esbuild transform using `jsx: "react-jsx"` from tsconfig.json, so no
// React plugin is needed.
export default defineConfig({
  clearScreen: false,
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'esnext',
    minify: !process.env.TAURI_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_DEBUG,
  },
});
