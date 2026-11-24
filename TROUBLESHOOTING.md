# Troubleshooting

## 1. Device not detected
- Ensure the FireWire device is connected and powered.
- Run `cargo run --bin scan` to list `/dev/fw*` devices.
- Verify that the device appears (e.g., `/dev/fw0`).
- If not, check kernel modules (`snd-firewire`, `snd-dice`, etc.) are loaded.

## 2. Application fails to start
- Check terminal output for panic messages.
- Run with `RUST_LOG=debug npm run tauri:dev` to get detailed logs.
- Ensure you have the correct version of Node.js (>=22.6).
- Verify that `npm install` has completed successfully.

## 3. No audio or muted channels
- Verify that the device is selected in the UI (Device dropdown).
- Ensure that the sample rate and clock source match the device capabilities.
- Check that the mixer faders are not set to `-∞` (muted) and that the master fader is up.
- Run `npm run test:mixer` to verify audio processing.

## 4. Configuration load fails
- Verify that the config file exists at `~/.config/firemix/<device_guid>.json`.
- Ensure JSON is well‑formed; you can validate with `jq . <file>`.
- If the device GUID changed, delete the old config or update the GUID.

## 5. BusReset errors
- BusReset is normal on FireWire; the app should recover automatically.
- If the UI shows “Device offline”, try disconnecting and reconnecting the device.
- Ensure that the app has permission to access `/dev/fw*` (add user to `audio` group or run with appropriate privileges).

## 6. UI not updating meters
- Confirm that the `meter-update` event is being emitted (check logs or enable `RUST_LOG=debug`).
- Ensure that the dedicated GLib thread (if used) is running; restart the app if it may have crashed.

## 7. Build failures
- Run `cargo check` to see compilation errors.
- Ensure you have the correct Rust toolchain (`rustup update`).
- Verify that all required system packages are installed (see GETTING_STARTED).

## 8. General logs
- Use environment variable `RUST_LOG=info` or `debug` to control verbosity.
- Tauri logs can be inspected via `npm run tauri:dev` console.
- System journal (`journalctl -u firemix.service` if running as a service).

## 9. Re‑install / reset
- Remove the config directory: `rm -rf ~/.config/firemix`.
- Re‑run the app; it will recreate a fresh config on next connection.