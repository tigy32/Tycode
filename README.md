# TyCode

## Getting Started
Tycode provides an AI coding assistant via CLI or VSCode extension. Checkout the repo from https://github.com/tigy32/Tycode and run `./dev.sh package` to build.


### Using Tycode
- **CLI**: Build and run with `cargo run --bin tycode` from the project root.
- **VSCode Extension [Recommended]**: See below for building the .vsix.

## Building and Installing the VSCode Extension

### Prerequisites
- Node.js (v20+) and npm
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

### First Time Users
1. **Add Provider**: Supported providers are AWSBedrock and OpenAI. Run `/provider add <name> bedrock <profile name>` (or use OpenRouter for OpenAI if testing personally).
2. **Set Cost Mode**: Run `/cost set unlimited` (or `/cost set low` for personal, using grok-4-fast which is cost-effective).

Sample configuration in `~/.tycode/settings.toml`:

```toml
active_provider = "default"
model_quality = "unlimited"
review_level = "None"

[providers.default]
type = "bedrock"
profile = "cline"
region = "us-west-2"

[security]
mode = "all"

[agent_models]
```

### Advanced Users

#### Security Mode
Security mode determines what tool actions the AI can perform, such as file modifications or command execution.
- **ReadOnly**: Restrict to read-only operations (e.g., no writes or spawns).
- **Auto**: Allow with heuristic safety checks (balanced for development).
- **All**: Permit all tools without restrictions (use cautiously; enables full file/system access).

Usage in chat:
- `/security`: Show current mode.
- `/security set <readonly|auto|all>`: Update mode for session (call `/settings save` to persist).

Why this separation: Pushes permission policy to user configs, surfacing risks early to avoid silent failures.
Errors (e.g., invalid mode) returned as chat messages immediately. For "all" mode, enable via sample settings.toml [security] section.