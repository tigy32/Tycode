# Tycode

Tycode is an AI-powered coding assistant that operates as both a command-line tool and a Visual Studio Code extension. Tycode follows a bring-your-own-key model where you maintain direct control over your AI provider and costs. You pay your AI provider directly (AWS Bedrock or OpenRouter) rather than through a subscription service.

![CI](https://github.com/tigy32/Tycode/actions/workflows/ci.yml/badge.svg)

## Installation

Start by cloning the repository and building the distribution package:

```bash
git clone https://github.com/tigy32/Tycode
cd Tycode
./dev.sh package
```

This build process produces both the CLI binary and the VSCode extension package.

### Command Line Interface

The CLI can be run directly from the repository using Cargo:

```bash
cargo run --bin tycode
```

### Visual Studio Code Extension

The packaging script generates a VSIX file in the tycode-vscode directory. Install it through VSCode by opening the command palette (Cmd+Shift+P or Ctrl+Shift+P), selecting "Extensions: Install from VSIX", and choosing the generated file.

## Configuration

Tycode stores its configuration in `~/.tycode/settings.toml`. While you can edit this file directly, the recommended approach is to use the built-in commands to manage your settings.

### Provider Setup

You must configure an AI provider before using Tycode. The system supports three  options:

**AWS Bedrock** requires an AWS account with access to Bedrock's LLM services. You'll need an AWS CLI profile configured with appropriate credentials. To configure Bedrock as your provider:

```bash
/provider add <name> bedrock <profile-name>
```

For example, if your AWS profile is named "default":

```bash
/provider add default-bedrock bedrock default
```

**OpenRouter** provides a simpler alternative and is recommended unless you must run in AWS. OpenRouter aggregates multiple LLM providers under a single API. Configuration follows a similar pattern:

```bash
/provider add <name> openrouter <api-key>
```

**Claude Code** allows you to use your Claude subscription with Tycode by leveraging the `claude` CLI command. This provider is ideal if you already have a Claude.ai subscription and want to use those credits directly:

```bash
/provider add <name> claude_code
```

You can optionally specify a custom command path or additional arguments if your `claude` CLI is installed in a non-standard location.

### Cost Controls

You can control the cost and quality of responses by specifying a cost tier:

```bash
/cost set <tier>
```

Available tiers include `unlimited` for maximum quality (using top-tier models like Claude), `low` for budget-conscious usage (currently routing to models like Grok-2-fast), and intermediate options. The `low` tier provides surprisingly capable performance for everyday development tasks while minimizing costs.

### Example Configuration

A typical configuration file looks like this:

```toml
active_provider = "default"
model_quality = "unlimited"
review_level = "None"

[providers.default]
type = "bedrock"
profile = "default"
region = "us-west-2"
```

This configuration uses AWS Bedrock through the "default" profile and sets quality to unlimited.

## Using Tycode

Tycode operates within strict directory boundaries. The model cannot read or write files outside the current workspace (in VSCode) or working directory (in the CLI). This sandboxing ensures that the AI remains focused on your current project and cannot accidentally modify unrelated files.

The model respects your .gitignore file and will treat ignored files as if they do not exist. This prevents the AI from reading build artifacts, dependencies, or other files you've chosen to exclude from version control.

Working with git is strongly recommended. Models can occasionally damage code while attempting to implement features, and having version control makes recovery trivial. A productive workflow starts from a clean git state, lets the AI make progress on a feature, and commits only once the implementation is complete and working. If something goes wrong during development, you can simply revert the changes and try a different approach.

## Commands Reference

Tycode provides slash commands for configuration and control. Type `/help` to see available commands.

### Session Commands

| Command | Description |
|---------|-------------|
| `/clear` | Clear the conversation history |
| `/quit` | Exit the application |
| `/help` | Show available commands |

### Model & Provider Commands

| Command | Description |
|---------|-------------|
| `/provider [list\|add\|remove\|switch]` | Manage AI providers |
| `/model <model-name>` | Set the AI model for all agents |
| `/models` | List available AI models |
| `/agentmodel <agent> <model> [options]` | Set model for a specific agent with tunings |
| `/cost [set <tier>\|reset]` | Show/set model cost tier (unlimited, high, medium, low) |

### Agent Commands

| Command | Description |
|---------|-------------|
| `/agent [name]` | Switch to a different agent or list available agents |
| `/review_level [None\|Task]` | Set the review level for responses |

### Context & Files

| Command | Description |
|---------|-------------|
| `/context` | Show what files would be included in the AI context |
| `/fileapi [patch\|find-replace]` | Set the file modification API |

### Configuration

| Command | Description |
|---------|-------------|
| `/settings` | Display current settings and configuration |
| `/profile [switch\|save\|list\|current]` | Manage settings profiles |
| `/trace [on\|off]` | Enable/disable trace logging to `.tycode/trace/` |

### Sessions & Memory

| Command | Description |
|---------|-------------|
| `/sessions [list\|resume\|delete\|gc]` | Manage conversation sessions |
| `/memory [summarize]` | Manage conversation memories |

### Skills & Plugins

| Command | Description |
|---------|-------------|
| `/skills [info <name>\|reload]` | List and manage available skills |
| `/skill <name>` | Manually invoke a skill |
| `/plugins` | List all installed plugins |
| `/plugin [install\|uninstall\|info\|enable\|disable\|reload]` | Manage plugins |
| `/hooks [events]` | List registered hooks from plugins |

### MCP Servers

| Command | Description |
|---------|-------------|
| `/mcp [list\|add\|remove]` | Manage MCP server configurations |

## Skills

Tycode supports Claude Code Agent Skills - modular capabilities that extend the agent with specialized workflows. Skills are automatically discovered and can be invoked when the AI detects a matching request.

### Skill Discovery

Skills are discovered from the following locations (in priority order):

1. `~/.claude/skills/` - User-level Claude Code compatibility
2. `~/.tycode/skills/` - User-level Tycode skills
3. `.claude/skills/` in workspace - Project-level Claude Code compatibility
4. `.tycode/skills/` in workspace - Project-level (highest priority)

### Creating a Skill

Each skill is a directory containing a `SKILL.md` file with YAML frontmatter:

```markdown
---
name: my-skill
description: When to use this skill
---

# My Skill Instructions

Step-by-step instructions for the AI to follow...
```

### Using Skills

List available skills:
```bash
/skills
```

Manually invoke a skill:
```bash
/skill <name>
```

View skill details:
```bash
/skills info <name>
```

Skills are also automatically invoked when the AI detects a user request matching a skill's description.

## MCP Server Configuration

Tycode supports locally running MCP servers over stdio transport. You can add or remove MCP servers using slash commands.

To add an MCP server:
```bash
/mcp add <name> <command> [--args "args..."] [--env "KEY=VALUE"]
```

To remove an MCP server:
```bash
/mcp remove <name>
```

Installed MCP servers are stored in your configuration file. Here's an example of a configured fetch server:

```toml
[mcp_servers.fetch]
command = "uvx"
args = ["mcp-server-fetch"]
```

## Plugins

Tycode features a plugin system that is **drop-in compatible with Claude Code plugins**. This means plugins built for Claude Code will work in Tycode with minimal or no modifications.

### Plugin Discovery

Plugins are discovered from multiple directories (in priority order, later overrides earlier):

1. `~/.claude/plugins/` - User-level Claude Code compatibility (lowest priority)
2. `~/.tycode/plugins/` - User-level Tycode plugins
3. `.claude/plugins/` in workspace - Project-level Claude Code compatibility
4. `.tycode/plugins/` in workspace - Project-level (highest priority)

### Installing Plugins

Install plugins from GitHub or local paths using the `/plugin install` command:

```bash
# From GitHub (shorthand)
/plugin install owner/repo

# From GitHub with custom name
/plugin install myname@owner/repo

# From GitHub with specific branch/tag
/plugin install owner/repo@main

# Alternative dash format (see note below)
/plugin install myname@owner-repo

# From local path
/plugin install ./path/to/plugin
/plugin install /absolute/path/to/plugin
```

**Examples:**

```bash
# Install the obsidian-skills plugin
/plugin install kepano/obsidian-skills

# Install with a custom name
/plugin install obsidian@kepano/obsidian-skills
```

**Note:** The dash format (`myname@owner-repo`) splits on the first dash, so usernames containing dashes may parse incorrectly. Use the slash format (`myname@owner/repo`) for reliable parsing.

### Managing Plugins

```bash
# List all installed plugins
/plugins

# Show plugin details
/plugin info <name>

# Enable/disable a plugin
/plugin enable <name>
/plugin disable <name>

# Uninstall a plugin
/plugin uninstall <name>

# Reload all plugins
/plugin reload
```

### Viewing Hooks

Use the `/hooks` command to inspect registered hooks from all enabled plugins:

```bash
# List all registered hooks grouped by event
/hooks

# List all available hook events with descriptions
/hooks events
```

The `/hooks` command displays:
- Hook event type (PreToolUse, PostToolUse, etc.)
- Plugin name providing the hook
- Command that will be executed
- Tool filters (if any)
- Timeout configuration

### Plugin Components

A plugin can provide any combination of the following:

| Component | Description | Location |
|-----------|-------------|----------|
| **Commands** | Slash commands (markdown files) | `commands/` |
| **Agents** | Subagent definitions (markdown files) | `agents/` |
| **Skills** | Agent capabilities (SKILL.md files) | `skills/` |
| **Hooks** | Event handlers (shell scripts) | `hooks/hooks.json` |
| **MCP Servers** | Tool integrations | `.mcp.json` |

### Plugin Structure

A Claude Code compatible plugin follows this structure:

```
my-plugin/
├── .claude-plugin/
│   └── plugin.json          # Plugin manifest (required)
├── commands/                 # Slash commands
│   ├── my-command.md
│   └── another-command.md
├── agents/                   # Subagent definitions
│   └── my-agent.md
├── skills/                   # Agent skills
│   └── my-skill/
│       └── SKILL.md
├── hooks/
│   └── hooks.json           # Hook configuration
├── scripts/                  # Hook scripts
│   └── pre-tool.sh
└── .mcp.json                # MCP server definitions
```

### Plugin Manifest

The `plugin.json` manifest defines the plugin metadata and component locations:

```json
{
  "name": "my-plugin",
  "version": "1.0.0",
  "description": "What this plugin does",
  "author": {
    "name": "Your Name",
    "email": "you@example.com"
  },
  "commands": "./commands/",
  "agents": "./agents/",
  "skills": "./skills/",
  "hooks": "./hooks/hooks.json",
  "mcpServers": "./.mcp.json"
}
```

All path fields are optional. If omitted, the plugin system uses these defaults:
- `commands` → `./commands/`
- `agents` → `./agents/`
- `skills` → `./skills/`
- `hooks` → `./hooks/hooks.json`
- `mcpServers` → `./.mcp.json`

Custom paths allow flexible plugin organization:

```json
{
  "name": "custom-layout-plugin",
  "version": "1.0.0",
  "commands": "./src/commands/",
  "agents": "./src/agents/",
  "skills": "./capabilities/",
  "hooks": "./config/hooks.json",
  "mcpServers": "./config/mcp.json"
}
```

### Creating Plugin Commands

Commands are markdown files with YAML frontmatter:

```markdown
---
name: my-command
description: What this command does
allowed_tools:
  - read_file
  - write_file
  - execute_command
---

# My Command

Instructions for the AI when this command is invoked...

## Steps

1. First, do this
2. Then, do that
```

Invoke with `/my-command [args]`.

### Creating Plugin Skills

Skills follow the same format as standalone skills:

```markdown
---
name: my-skill
description: When to use this skill
---

# My Skill Instructions

Detailed instructions for the AI...
```

Plugin skills appear in `/skills` and can be invoked automatically or manually.

### Plugin Hooks

Hooks allow plugins to intercept and modify behavior at various points. Configure hooks in `hooks/hooks.json`:

```json
{
  "hooks": [
    {
      "event": "PreToolUse",
      "command": "${CLAUDE_PLUGIN_ROOT}/scripts/pre-tool.sh",
      "timeout": 5000
    },
    {
      "event": "PostToolUse",
      "command": "${CLAUDE_PLUGIN_ROOT}/scripts/post-tool.sh",
      "timeout": 5000
    }
  ]
}
```

**Supported Hook Events:**

| Event | Trigger | Status |
|-------|---------|--------|
| `PreToolUse` | Before tool execution | ✅ Implemented |
| `PostToolUse` | After successful tool execution | ✅ Implemented |
| `PostToolUseFailure` | After tool execution fails | ✅ Implemented |
| `UserPromptSubmit` | When user submits prompt | ✅ Implemented |
| `SessionStart` | At session start | ✅ Implemented |
| `SessionEnd` | At session end | ⏳ TODO |
| `Stop` | When agent finishes | ⏳ TODO |
| `SubagentStart` | When subagent starts | ⏳ TODO |
| `SubagentStop` | When subagent stops | ⏳ TODO |
| `PermissionRequest` | When permission dialog shown | ⏳ TODO |
| `Notification` | When notifications sent | ⏳ TODO |
| `PreCompact` | Before context compaction | ⏳ TODO |

Hooks receive JSON input via stdin and output JSON to stdout. See [Claude Code Hooks Reference](https://docs.anthropic.com/en/docs/claude-code/hooks) for the full protocol.

**Security Note:** Hook commands are executed as shell processes with full access to your system. Only install plugins from trusted sources. Review hook configurations in `hooks/hooks.json` before enabling a plugin. Malicious hooks could read sensitive files, execute arbitrary code, or exfiltrate data.

### Plugin MCP Servers

Define MCP servers in `.mcp.json`:

```json
{
  "mcpServers": {
    "my-server": {
      "command": "${CLAUDE_PLUGIN_ROOT}/bin/server",
      "args": ["--config", "${CLAUDE_PLUGIN_ROOT}/config.json"],
      "env": {
        "API_KEY": "${MY_API_KEY}"
      }
    }
  }
}
```

The `${CLAUDE_PLUGIN_ROOT}` variable expands to the plugin's root directory.

### Configuration

Control plugin behavior in your `settings.toml`:

```toml
[plugins]
enabled = true                    # Enable/disable plugin system
enable_claude_code_compat = true  # Check ~/.claude/plugins/ paths
allow_native = true               # Allow native Rust plugins

# Disable specific plugins
disabled_plugins = ["experimental-plugin"]

# Additional plugin directories
additional_dirs = ["/opt/shared-plugins"]
```

### Example: Installing and Using Obsidian Plugin

```bash
# Install the plugin
/plugin install obsidian@kepano/obsidian-skills

# Verify installation
/plugins

# Check available skills from the plugin
/skills

# The plugin adds skills for working with Obsidian:
# - obsidian-markdown: For .md files in Obsidian vaults
# - obsidian-bases: For .base files
# - json-canvas: For .canvas files
```

## Plugin System TODOs

The following features are planned for future implementation:

### Hook Events
- [ ] **SessionEnd Hook** - Fire when session ends
- [ ] **Stop Hook** - Fire when agent completes task
- [ ] **SubagentStart/SubagentStop Hooks** - Fire when subagents spawn/complete
- [ ] **PermissionRequest Hook** - Allow plugins to handle permission dialogs
- [ ] **Notification Hook** - Fire when notifications are sent
- [ ] **PreCompact Hook** - Fire before context compaction
- [ ] **Hook Output Handling** - Process `updatedInput` from PreToolUse hooks to modify tool inputs

### Plugin Features
- [ ] **Plugin Agents as Spawnable Subagents** - Create `Agent` trait wrapper for plugin-defined agents
- [ ] **Native Plugin Support** - Load Rust `.dylib`/`.so` plugins via `tycode-plugin.toml`
- [ ] **Plugin Dependency Resolution** - Handle plugin dependencies
- [ ] **Plugin Auto-Update** - Check for and apply plugin updates
