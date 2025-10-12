# TyCode

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