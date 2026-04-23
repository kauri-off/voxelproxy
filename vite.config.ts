import { defineConfig } from 'vite';

// Dev is served by esbuild via `node dev.mjs` (see package.json "dev").
// This config only runs for `vite build`. JSX is handled by Vite's built-in
// esbuild transform using `jsx: "react-jsx"` from tsconfig.json, so no
// React plugin is needed.
export default defineConfig({
  clearScreen: false,
<<<<<<< HEAD
=======
  server: {
    port: 5173,
    strictPort: true,
    host: '127.0.0.1',
    watch: {
      ignored: ["/src-tauri/"],
    },
  },
>>>>>>> 70f455d (Fix)
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'esnext',
    minify: !process.env.TAURI_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_DEBUG,
  },
});
