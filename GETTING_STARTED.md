# Getting Started

## Prerequisites
- Linux (Ubuntu 22.04+ recommended)
- Rust toolchain (`rustup`)
- Node.js 22.6 or newer
- System packages: `build-essential pkg-config curl wget file libssl-dev libwebkit2gtk-4.1-dev libxdo-dev librsvg2-dev libayatana-appindicator3-dev`

## Build Dependencies
Install the system packages required to build the Tauri application:

```bash
sudo apt update
sudo apt install -y build-essential pkg-config curl wget file libssl-dev \
    libwebkit2gtk-4.1-dev libxdo-dev librsvg2-dev libayatana-appindicator3-dev
```

`libwebkit2gtk-4.1-dev` provides the WebKitGTK, JavaScriptCoreGTK, and libsoup development dependencies required by Tauri on Linux.

## Verify Rust backend
```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

## Run the scanner
```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin scan
```
To see the live DICE EAP matrix dimensions and port labels:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin scan -- --matrix
```

## Launch the application
```bash
npm run tauri:dev
```

## Configuration
FireMix stores per‑device configuration in `~/.config/firemix/<device_guid>.json` (or `$XDG_CONFIG_HOME/firemix`). On startup it loads the saved snapshot and applies it, rejecting mismatched configurations.

## Testing
Run mixer math tests:

```bash
npm run test:mixer
```

## Build for release
```bash
npm run tauri:build
```

## Additional Resources
- **Contributing** – see [CONTRIBUTING.md](CONTRIBUTING.md) for how to add support for new devices.
- **Troubleshooting** – see [TROUBLESHOOTING.md](TROUBLESHOOTING.md) for common issues and solutions.