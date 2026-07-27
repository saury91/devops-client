# devops-client build commands

set positional-arguments := true

# Install dependencies
install:
    pnpm install

# Run in dev mode (Tauri serves src/ on its built-in static server and watches changes)
dev:
    pnpm run dev

# Run a debug build for current platform without file watcher — quick launch for testing
debug:
    pnpm tauri dev --no-watch

# Build for macOS ARM64 (local dev)
build:
    pnpm run build:mac-arm

# Build for macOS x64
build-mac-x64:
    pnpm run build -- --target x86_64-apple-darwin

# Build both macOS architectures (Windows and Linux are built in CI)
build-all:
    pnpm run build -- --target aarch64-apple-darwin
    pnpm run build -- --target x86_64-apple-darwin

# Run cargo check on Rust backend
check:
    cd src-tauri && cargo check

# Run cargo fmt
fmt:
    cd src-tauri && cargo fmt

# Run cargo clippy
lint:
    cd src-tauri && cargo clippy -- -D warnings

# Clean build artifacts
clean:
    rm -rf src-tauri/target
    rm -rf node_modules

# Full build flow: check, then build
verify: check build

# Open the built app
run:
    open src-tauri/target/aarch64-apple-darwin/release/bundle/macos/DevOps\ Client.app
