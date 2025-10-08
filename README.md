# TyCode

## Building and Installing the VSCode Extension

### Prerequisites
- Node.js (v18+) and npm
- Visual Studio Code
- Build dependencies: Rust (for tycode-subprocess), make, etc. (handled by dev.sh)

### Build the Extension
1. Navigate to the project root: `cd /tycode` (or clone repo root).
2. Run the build script in package mode: `./dev.sh package`
   - This compiles the extension (TypeScript to JS in tycode-vscode/out/) and packages it into a .vsix file.
   - Output: `tycode-<version>.vsix` in `/tycode/tycode-vscode/`.
   - Why package mode: Ensures optimized release build; default modes compile only for dev.

### Install the Extension
1. Open VSCode.
2. Go to Extensions view (Ctrl+Shift+X).
3. Click the "..." menu > "Install from VSIX...".
4. Select the generated `tycode-<version>.vsix` file.
5. Reload VSCode if prompted.

For development: Use `./dev.sh` (default mode) and press F5 in tycode-vscode/ to debug without packaging.