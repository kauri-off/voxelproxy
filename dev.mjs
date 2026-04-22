// Minimal esbuild-based dev server, replacing `vite dev`.
// Builds src/main.tsx to dev/main.js on every file change; serves the
// `dev/` folder on http://127.0.0.1:5173. Auto-reloads the page on rebuild
// via esbuild's built-in /esbuild SSE endpoint.
//
// `npm run build` still uses Vite.
import * as esbuild from 'esbuild';

const ctx = await esbuild.context({
  entryPoints: ['src/main.tsx'],
  bundle: true,
  outfile: 'dev/main.js',
  loader: { '.css': 'css' },
  format: 'esm',
  target: 'esnext',
  sourcemap: true,
  jsx: 'automatic',
  jsxDev: true,
  define: {
    'process.env.NODE_ENV': '"development"',
  },
  // Client script that reloads the page when esbuild finishes a rebuild.
  banner: {
    js: "new EventSource('/esbuild').addEventListener('change', () => location.reload());",
  },
  logLevel: 'info',
});

await ctx.watch();

const { hosts, port } = await ctx.serve({
  host: '127.0.0.1',
  port: 5173,
  servedir: 'dev',
});

console.log(`\n  esbuild dev server: http://${hosts[0]}:${port}\n`);
