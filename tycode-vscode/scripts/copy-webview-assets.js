#!/usr/bin/env node
const fs = require('fs');
const path = require('path');

const srcDir = path.join(__dirname, '..', 'src', 'webview');
const outDir = path.join(__dirname, '..', 'out', 'webview');

// Ensure output directory exists
fs.mkdirSync(outDir, { recursive: true });

// Copy CSS and JS files
const files = fs.readdirSync(srcDir);
for (const file of files) {
  if (file.endsWith('.css') || file.endsWith('.js')) {
    const src = path.join(srcDir, file);
    const dest = path.join(outDir, file);
    fs.copyFileSync(src, dest);
    console.log(`Copied: ${file}`);
  }
}
