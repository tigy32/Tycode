const fs = require('fs');
const path = require('path');

// Check for release version from environment variable
const releaseVersion = process.env.RELEASE_VERSION;

let buildTime;
if (releaseVersion) {
  // Use release version (e.g., "0.2.0" from tag "v0.2.0")
  buildTime = releaseVersion.replace(/^v/, '');
} else {
  // Generate build timestamp in compact format: YYYYMMDD.HHMM
  const now = new Date();
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, '0');
  const day = String(now.getDate()).padStart(2, '0');
  const hours = String(now.getHours()).padStart(2, '0');
  const minutes = String(now.getMinutes()).padStart(2, '0');
  buildTime = `${year}${month}${day}.${hours}${minutes}`;
}

const timestamp = new Date().toISOString();

// Generate TypeScript file content
const tsContent = `// Auto-generated build info - DO NOT EDIT
export const buildInfo = {
    buildTime: '${buildTime}',
    timestamp: '${timestamp}'
};
`;

// Write to src directory as TypeScript file
const outputPath = path.join(__dirname, '..', 'src', 'build-info.ts');
fs.writeFileSync(outputPath, tsContent);

console.log(`Build info generated: ${buildTime}`);