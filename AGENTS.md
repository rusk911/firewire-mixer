# Project: FireMix (Modern Linux FireWire Audio Interface Mixer)

## Goal
Build a modern, lightweight GUI replacement for `ffado-mixer` on Linux using **Tauri (v2)** + **Rust** + **React/TypeScript**.
The app controls hardware DSP mixers, routing matrices, and preamps of IEEE 1394 audio interfaces **coexisting peacefully with ALSA/PipeWire kernel drivers** (`snd-dice`, `snd-bebob`, `snd-fireface`).

## Strict Architectural Rules (DO NOT VIOLATE)
1. **NO FFADO / NO D-Bus:** Never use `libffado`, `ffado-dbus-server`, or Python bindings. All communication happens via `/dev/fw*` character devices.
2. **USE `hinawa` + `firewire-*-protocols`:** Do NOT write raw `ioctl` calls to `linux/firewire-cdev.h` and do NOT reverse-engineer FireWire register offsets from scratch. Use Takashi Sakamoto's crates:
   - `hinawa` (for `FwNode`, `FwReq`, `FwResp`, `FwFcp` transport).
   - `firewire-dice-protocols` (for DICE EAP and vendor extensions).
   - `firewire-bebob-protocols` (for BridgeCo BeBoB / AV/C FCP devices).
3. **GLib MainLoop Awareness:** `hinawa` relies on GLib/GObject signals. On Linux, Tauri uses WebKitGTK, which already runs a GLib `GMainContext` on the main thread, OR spawn a dedicated thread running `glib::MainLoop` for hardware event polling/notifications.
4. **Hardware-Agnostic Frontend:** The TypeScript UI must NEVER know about DICE, BeBoB, or specific register layouts. The Rust backend translates hardware-specific protocols into a unified `DeviceState` JSON schema and accepts generic `MixerCommand` enums via Tauri IPC.
5. **Graceful Error Handling:** FireWire bus resets (`BusReset`) happen frequently. Never `.unwrap()` on `hinawa` transactions; always propagate errors cleanly to the UI.

## Reference Codebase
When implementing protocol bindings, follow the exact patterns used in `alsa-project/snd-firewire-ctl-services` (`protocols/dice` and `protocols/bebob`).

## Development Workflow
- Backend code lives in `src-tauri/src/`.
- Keep hardware drivers modular:
  - `src-tauri/src/transport/` (hinawa wrappers)
  - `src-tauri/src/devices/` (implementations per card/protocol)
  - `src-tauri/src/model/` (unified mixer traits & DTOs for Tauri)
- Always run `cargo check` after modifying Rust files.
