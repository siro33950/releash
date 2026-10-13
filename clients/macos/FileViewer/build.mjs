import { build } from 'esbuild';
import { mkdir, copyFile } from 'node:fs/promises';
const outdir = '../.build/FileViewer';
await mkdir(outdir, { recursive: true });
await build({ entryPoints: ['bridge.js'], bundle: true, format: 'iife', platform: 'browser', target: 'safari18', outfile: `${outdir}/viewer.js` });
await copyFile('index.html', `${outdir}/index.html`);
