# Contributing to FireMix

Thank you for considering contributing! This document describes how to add support for a new FireWire audio device.

## 1. Set up your development environment
- Install system dependencies (see GETTING_STARTED.md).
- Install Rust toolchain (`rustup`).

## 2. Fork and clone the repository
```bash
git clone https://github.com/your-org/firewire-mixer.git
cd firewire-mixer
```

## 3. Add hardware profile
- Edit `docs/HARDWARE_PROFILES.md` and add a new entry with:
  - Device name
  - Vendor ID
  - Model ID
  - Recommended sample rates / clock sources
  - Any special notes (e.g., requires specific firmware version)

## 4. Implement the device driver
- Create a new file in `src-tauri/src/devices/` (e.g., `mydevice.rs`).
- Implement the `AudioDevice` trait from `src-tauri/src/model/device.rs`.
- Use `hinawa` and appropriate protocol crate (`firewire-dice-protocols` or `firewire-bebob-protocols`) to communicate with the hardware.
- Ensure all methods return `Result<_, MixerError>` (or `String` if you keep the simple approach) and handle `BusReset` gracefully.

## 5. Register the device in the discovery flow
- Add a match arm in the device discovery code (`src-tauri/src/device_discovery.rs`) that maps `(vendor_id, model_id)` to your new driver type.
- Update unit tests to cover the new device.

## 6. Write tests
- Add integration tests using a virtual FireWire loopback device or a mock.
- Run `cargo test` to verify.

## 7. Build and verify
```bash
cargo check
cargo test
npm run tauri:dev   # to test UI integration
```

## 8. Submit a Pull Request
- Ensure your PR follows the project's code style (Rustfmt, Clippy).
- Reference the issue number if applicable.
- Include a brief description of the change and why it was needed.

## 9. Code of Conduct
Please follow the project's code of conduct and use respectful language.

Happy hacking! 🎛️

## See also
- [GETTING_STARTED.md](GETTING_STARTED.md)
- [TROUBLESHOOTING.md](TROUBLESHOOTING.md)