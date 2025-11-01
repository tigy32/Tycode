#!/usr/bin/env node

/**
 * Collects tycode-subprocess binaries from cargo-dist artifacts and organizes them
 * for VSCode extension packaging.
 * 
 * Usage:
 *   - Local development: npm run collect-binaries:local
 *     Builds for current platform only using cargo
 * 
 *   - CI/Production: npm run collect-binaries:ci
 *     Extracts binaries from cargo-dist artifacts directory
 */

const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');
const os = require('os');

const CARGO_DIST_TO_VSCODE = {
  'aarch64-apple-darwin': 'darwin-arm64',
  'x86_64-apple-darwin': 'darwin-x64',
  'aarch64-unknown-linux-gnu': 'linux-arm64',
  'x86_64-unknown-linux-gnu': 'linux-x64',
  'x86_64-pc-windows-msvc': 'win32-x64',
};

const WINDOWS_TARGETS = ['x86_64-pc-windows-msvc'];

function getCurrentPlatform() {
  const platform = os.platform();
  const arch = os.arch();
  
  if (platform === 'darwin') {
    return arch === 'arm64' ? 'darwin-arm64' : 'darwin-x64';
  }
  if (platform === 'linux') {
    return arch === 'arm64' ? 'linux-arm64' : 'linux-x64';
  }
  if (platform === 'win32') {
    return 'win32-x64';
  }
  
  throw new Error(`Unsupported platform: ${platform}-${arch}`);
}

function buildLocal() {
  console.log('Building tycode-subprocess for current platform...');
  
  const projectRoot = path.resolve(__dirname, '../..');
  const subprocessDir = path.join(projectRoot, 'tycode-subprocess');
  
  execSync('cargo build --release', {
    cwd: subprocessDir,
    stdio: 'inherit',
  });
  
  const platform = getCurrentPlatform();
  const binaryName = platform.startsWith('win32') ? 'tycode-subprocess.exe' : 'tycode-subprocess';
  const sourcePath = path.join(projectRoot, 'target', 'release', binaryName);
  const targetDir = path.join(__dirname, '..', 'bin', platform);
  const targetPath = path.join(targetDir, binaryName);
  
  fs.mkdirSync(targetDir, { recursive: true });
  fs.copyFileSync(sourcePath, targetPath);
  fs.chmodSync(targetPath, 0o755);
  
  console.log(`Binary copied to: ${targetPath}`);
}

function extractFromArchive(archivePath, binaryName, targetPath) {
  const ext = path.extname(archivePath);
  
  if (ext === '.xz') {
    execSync(`tar -xJf "${archivePath}" -C /tmp`, { stdio: 'inherit' });
    const archiveBase = path.basename(archivePath, '.tar.xz');
    const extractedBinary = path.join('/tmp', archiveBase, binaryName);
    fs.copyFileSync(extractedBinary, targetPath);
    execSync(`rm -rf /tmp/${archiveBase}`);
  } else if (ext === '.zip') {
    execSync(`unzip -q "${archivePath}" -d /tmp`, { stdio: 'inherit' });
    const archiveBase = path.basename(archivePath, '.zip');
    const extractedBinary = path.join('/tmp', archiveBase, binaryName);
    fs.copyFileSync(extractedBinary, targetPath);
    execSync(`rm -rf /tmp/${archiveBase}`);
  } else {
    throw new Error(`Unsupported archive format: ${ext}`);
  }
}

function collectFromCargoDistArtifacts() {
  console.log('Collecting binaries from cargo-dist artifacts...');
  
  const projectRoot = path.resolve(__dirname, '../..');
  const artifactsDir = path.join(projectRoot, 'target', 'distrib');
  
  if (!fs.existsSync(artifactsDir)) {
    throw new Error(`Artifacts directory not found: ${artifactsDir}`);
  }
  
  for (const [cargoTarget, vscodePlatform] of Object.entries(CARGO_DIST_TO_VSCODE)) {
    const isWindows = WINDOWS_TARGETS.includes(cargoTarget);
    const binaryName = isWindows ? 'tycode-subprocess.exe' : 'tycode-subprocess';
    const archiveExt = isWindows ? 'zip' : 'tar.xz';
    const archiveName = `tycode-subprocess-${cargoTarget}.${archiveExt}`;
    const archivePath = path.join(artifactsDir, archiveName);
    
    if (!fs.existsSync(archivePath)) {
      console.warn(`Warning: Archive not found: ${archivePath}`);
      continue;
    }
    
    const targetDir = path.join(__dirname, '..', 'bin', vscodePlatform);
    const targetPath = path.join(targetDir, binaryName);
    
    fs.mkdirSync(targetDir, { recursive: true });
    
    console.log(`Extracting ${cargoTarget} -> ${vscodePlatform}...`);
    extractFromArchive(archivePath, binaryName, targetPath);
    fs.chmodSync(targetPath, 0o755);
    
    console.log(`  ✓ ${targetPath}`);
  }
  
  console.log('All binaries collected successfully.');
}

const mode = process.argv[2] || 'local';

if (mode === 'local') {
  buildLocal();
} else if (mode === 'ci') {
  collectFromCargoDistArtifacts();
} else {
  console.error(`Unknown mode: ${mode}`);
  console.error('Usage: node collect-binaries.js [local|ci]');
  process.exit(1);
}
