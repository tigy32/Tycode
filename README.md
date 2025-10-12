# Tycode

Tycode is an AI-powered coding assistant that operates as both a command-line tool and a Visual Studio Code extension. Tycode follows a bring-your-own-key model where you maintain direct control over your AI provider and costs. You pay your AI provider directly (AWS Bedrock or OpenRouter) rather than through a subscription service.

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

You must configure an AI provider before using Tycode. The system supports two primary options:

**AWS Bedrock** requires an AWS account with access to Bedrock's LLM services. You'll need an AWS CLI profile configured with appropriate credentials. To configure Bedrock as your provider:

```bash
/provider add <name> bedrock <profile-name>
```

For example, if your AWS profile is named "default":

```bash
/provider add default-bedrock bedrock default
```

**OpenRouter** provides a simpler alternative for personal projects or those without AWS infrastructure. OpenRouter aggregates multiple LLM providers under a single API. Configuration follows a similar pattern:

```bash
/provider add <name> openrouter <api-key>
```

### Cost Controls

You can control the cost and quality of responses by specifying a cost tier:

```bash
/cost set <tier>
```

Available tiers include `unlimited` for maximum quality (using top-tier models like Claude), `low` for budget-conscious usage (currently routing to models like Grok-2-fast), and intermediate options. The `low` tier provides surprisingly capable performance for everyday development tasks while minimizing costs.

### Security Mode

You can control the security mode to determine what operations Tycode is allowed to perform. Available modes are `readonly`, `auto`, or `all`. The `all` mode is recommended as it allows the model to build code and run tests, though be aware that models may execute destructive commands so use this setting cautiously.

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

[security]
mode = "all"
```

This configuration uses AWS Bedrock through the "default" profile, sets quality to unlimited, and enables full command execution.

## Using Tycode

Tycode operates within strict directory boundaries. The model cannot read or write files outside the current workspace (in VSCode) or working directory (in the CLI). This sandboxing ensures that the AI remains focused on your current project and cannot accidentally modify unrelated files.

The model respects your .gitignore file and will treat ignored files as if they do not exist. This prevents the AI from reading build artifacts, dependencies, or other files you've chosen to exclude from version control.

Working with git is strongly recommended. Models can occasionally damage code while attempting to implement features, and having version control makes recovery trivial. A productive workflow starts from a clean git state, lets the AI make progress on a feature, and commits only once the implementation is complete and working. If something goes wrong during development, you can simply revert the changes and try a different approach.


TyCode is an AI coding assistant available as a CLI or VSCode extension. 

## Using TyCode
- **CLI**: Build and run with `cargo run --bin tycode` from the project root.
- **VSCode Extension**: See below for building and installing the .vsix.

### Building and Installing the VSCode Extension

1. Run the build script in package mode: `./dev.sh package`
   - This compiles the extension and packages it into a .vsix file in `tycode-vscode/`.
2. Open VSCode.
3. Go to Extensions view (Ctrl+Shift+X).
4. Click the "..." menu > "Install from VSIX...".
5. Select the generated `tycode-<version>.vsix` file.
6. Reload VSCode.

### Configuration 

Upon running TyCode for the first time, you will need to configure an AI service provider and optionally change some configurations. 

1. **Add Provider**: You need to configure an AI service provider - openrouter.ai or AWS Bedrock are supported. If you have neither I recommend creating an account openrouter.ai - its super easy. 
  - For OpenRouter `/provider add default openrouter <api key>`  
  - For AWS Bedrock `/provider add default bedrock <profile name>` 
2. **Set Cost Mode**: Run `/cost set unlimited` to use the highest quality models or `/cost set low` for lower quality, but cheaper, models. As a rough sizing guide - building a static HTML website might cost about $2-10 on the highest quality model or 2-10 cents on the cheapest models. 
3. **Set Security Mode**: Run `/security set <readonly|auto|all>` to change the models permissions. By default, the model will have access to tools to read and modify files in the current working directory (excluding any .git directories) but not run any commands. You will generally get better results allowing the model to run commands to run builds and tests and validate changes.