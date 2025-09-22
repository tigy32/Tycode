#!/bin/bash

# Simplified build script for TyCode VSCode Extension

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}TyCode VSCode Extension Builder${NC}"
echo "==================================="
echo ""

# Function to detect platform and architecture
detect_platform() {
    local platform=$(uname -s)
    local arch=$(uname -m)
    
    case "$platform" in
        "Darwin")
            if [ "$arch" = "arm64" ]; then
                echo "darwin-arm64"
            else
                echo "darwin-x64"
            fi
            ;;
        "Linux")
            echo "linux-x64"
            ;;
        *)
            echo "unsupported"
            ;;
    esac
}

# Function to get binary name with extension
get_binary_name() {
    local platform=$1
    if [[ $platform == *"win32"* ]]; then
        echo "tycode-subprocess.exe"
    else
        echo "tycode-subprocess"
    fi
}

# Main build function
build_extension() {
    local build_mode=$1  # "debug" or "release"
    
    echo -e "${YELLOW}Starting ${build_mode} build...${NC}"
    
    # Step 1: Clean old binaries
    echo -e "${YELLOW}1. Cleaning old binaries...${NC}"
    rm -rf tycode-client-typescript/bin
    
    # Step 2: Build tycode-subprocess binary
    echo -e "${YELLOW}2. Building tycode-subprocess binary (${build_mode} mode)...${NC}"
    cd tycode-subprocess
    if [ "$build_mode" = "release" ]; then
        cargo build --release
        SOURCE_PATH="target/release/tycode-subprocess"
    else
        cargo build
        SOURCE_PATH="target/debug/tycode-subprocess"
    fi
    cd ..
    
    # Step 3: Copy binary to client library
    echo -e "${YELLOW}3. Copying binary to client library...${NC}"
    PLATFORM=$(detect_platform)
    BINARY_NAME=$(get_binary_name $PLATFORM)
    
    if [ "$PLATFORM" = "unsupported" ]; then
        echo -e "${RED}Unsupported platform: $(uname -s) $(uname -m)${NC}"
        exit 1
    fi
    
    mkdir -p "tycode-client-typescript/bin/$PLATFORM"
    cp "$SOURCE_PATH" "tycode-client-typescript/bin/$PLATFORM/$BINARY_NAME"
    chmod +x "tycode-client-typescript/bin/$PLATFORM/$BINARY_NAME"
    
    echo -e "${GREEN}Binary copied to: tycode-client-typescript/bin/$PLATFORM/$BINARY_NAME${NC}"
    
    # Step 4: Install and build TypeScript client library
    echo -e "${YELLOW}4. Building TypeScript client library...${NC}"
    cd tycode-client-typescript
    
    # Install dependencies if node_modules doesn't exist
    if [ ! -d "node_modules" ]; then
        echo -e "${YELLOW}   Installing client library dependencies...${NC}"
        npm install
    fi
    
    # Build the library
    npm run build
    cd ..
    
    # Step 5: Install and build VSCode extension
    echo -e "${YELLOW}5. Building VSCode extension...${NC}"
    cd tycode-vscode
    
    # Install dependencies if node_modules doesn't exist
    if [ ! -d "node_modules" ]; then
        echo -e "${YELLOW}   Installing VSCode extension dependencies...${NC}"
        npm install
    fi
    
    # Copy client library files into extension
    echo -e "${YELLOW}   Copying client library into extension...${NC}"
    rm -rf lib bin
    mkdir -p lib bin
    cp -r ../tycode-client-typescript/lib/* lib/
    cp -r ../tycode-client-typescript/bin/* bin/
    
    # Build the extension
    npm run compile
    
    # Copy webview assets
    echo -e "${YELLOW}   Copying webview assets...${NC}"
    mkdir -p out/webview
    cp src/webview/*.css out/webview/ 2>/dev/null || true
    cp src/webview/*.js out/webview/ 2>/dev/null || true
    
    cd ..
    
    echo -e "${GREEN}Build complete! (${build_mode} mode)${NC}"
    echo -e "${YELLOW}Binary location: tycode-client-typescript/bin/$PLATFORM/$BINARY_NAME${NC}"
    echo -e "${YELLOW}VSCode extension built in: tycode-vscode/out/${NC}"
}

case "$1" in
    "")
        # Default: debug build for fast development
        build_extension "debug"
        ;;
        
    "release")
        build_extension "release"
        ;;
        
    "clean")
        echo -e "${YELLOW}Cleaning all build artifacts...${NC}"
        
        # Clean Rust build
        cd tycode-subprocess
        cargo clean
        cd ..
        
        # Clean TypeScript client
        cd tycode-client-typescript
        rm -rf lib/
        rm -rf bin/
        rm -rf node_modules/
        cd ..
        
        # Clean VSCode extension
        cd tycode-vscode
        rm -rf out/
        rm -rf node_modules/
        rm -f *.vsix
        cd ..
        
        echo -e "${GREEN}Clean complete!${NC}"
        ;;
        
    "setup")
        echo -e "${YELLOW}Setting up development environment...${NC}"
        
        # Check for Node.js
        if ! command -v node &> /dev/null; then
            echo -e "${RED}Node.js is not installed. Please install Node.js first.${NC}"
            exit 1
        fi
        
        # Check for Rust
        if ! command -v rustc &> /dev/null; then
            echo -e "${RED}Rust is not installed. Please install Rust first.${NC}"
            exit 1
        fi
        
        # Install Node dependencies for client library
        echo -e "${YELLOW}Installing TypeScript client dependencies...${NC}"
        cd tycode-client-typescript
        npm install
        cd ..
        
        # Install Node dependencies for VSCode extension
        echo -e "${YELLOW}Installing VSCode extension dependencies...${NC}"
        cd tycode-vscode
        npm install
        cd ..
        
        echo -e "${GREEN}Setup complete! Run './dev.sh' to build.${NC}"
        ;;
        
    "package")
        echo -e "${YELLOW}Creating VSIX package...${NC}"
        
        # First do a release build
        build_extension "release"
        
        # Then create the package
        echo -e "${YELLOW}6. Creating VSIX package...${NC}"
        cd tycode-vscode
        npm run package
        
        if ls *.vsix 1> /dev/null 2>&1; then
            echo -e "${GREEN}Package created successfully!${NC}"
            ls -la *.vsix
        else
            echo -e "${RED}Failed to create package${NC}"
            exit 1
        fi
        
        cd ..
        ;;
        
    "watch")
        echo -e "${YELLOW}Starting watch mode...${NC}"
        
        # Do initial debug build
        build_extension "debug"
        
        echo -e "${GREEN}Initial build complete!${NC}"
        echo -e "${YELLOW}Starting TypeScript watch mode...${NC}"
        echo -e "${YELLOW}Note: Rebuild with './dev.sh' if you change Rust code${NC}"
        
        # Start watch mode for VSCode extension
        cd tycode-vscode
        npm run watch
        ;;
        
    *)
        echo "Usage: ./dev.sh [command]"
        echo ""
        echo "Commands:"
        echo "  (none)    - Build extension (debug mode, fastest)"
        echo "  release   - Build extension (release mode, optimized)"
        echo "  setup     - Install dependencies"
        echo "  watch     - Start TypeScript watch mode for development"
        echo "  package   - Create VSIX package for distribution"
        echo "  clean     - Remove all build artifacts"
        echo ""
        echo "Examples:"
        echo "  ./dev.sh           # Quick debug build"
        echo "  ./dev.sh release   # Optimized build"
        echo "  ./dev.sh setup     # First time setup"
        echo "  ./dev.sh watch     # Development with auto-rebuild"
        echo "  ./dev.sh package   # Create installable VSIX"
        exit 1
        ;;
esac