# System Tray & Multi-Window Architecture (Tauri v2)

## 1. Startup Behavior (Background Daemon Mode)
- The app must start **without opening a main window** (remove default windows from `tauri.conf.json` or set `"visible": false`).
- Keep the application alive when all windows are closed by intercepting `RunEvent::ExitRequested` in `lib.rs` and calling `api.prevent_exit()`.
- Only exit the process when the user explicitly clicks **"Quit"** in the system tray menu.

## 2. Background Device Polling
- Spawn a background async task (`tauri::async_runtime::spawn`) on startup that scans `/dev/fw*` every 2 seconds (or listens to `udev` / `notify` events on `/dev`).
- Maintain shared state `AppState { devices: Mutex<Vec<DeviceInfo>> }`.
- When the list of connected supported devices changes:
  1. Rebuild the System Tray `Menu` dynamically (`tray.set_menu(Some(new_menu))`).
  2. Emit a Tauri global event `"devices-changed"` so any open UI reacts immediately.
  3. If a device was unplugged while its mixer window was open, close that window (`window.close()`).

## 3. System Tray Menu Structure (`tauri::tray::TrayIconBuilder`)
Use feature `tray-icon` in `tauri` crate (`Cargo.toml`).
Menu items:
- Header / Status (disabled item): `"FireMix — X device(s) connected"` (or `"No FireWire devices found"`).
- Separator.
- Dynamic list of connected devices:
  - Item ID: `"open_device:<node_path>"` (e.g., `"open_device:/dev/fw1"`), Label: `"Focusrite Saffire PRO 40 (/dev/fw1)"`.
- Separator.
- `"Rescan Now"` (ID: `"rescan"`).
- `"Quit"` (ID: `"quit"`).

## 4. Multi-Window Mixer Management (`WebviewWindowBuilder`)
- When the user clicks a device item `"open_device:<node_path>"`:
  - Generate a safe window label from `node_path`, e.g., `mixer_fw1`.
  - Check if a window with label `mixer_fw1` already exists via `app.get_webview_window("mixer_fw1")`.
  - If it exists: call `.show()` and `.set_focus()`.
  - If it does not exist: create a new window via `WebviewWindowBuilder::new(app, "mixer_fw1", WebviewUrl::App("index.html?node=/dev/fw1".into()))` with title `"FireMix — <Model Name>"` and reasonable default size (`1100x700`).
- **Frontend Routing:** On startup, the frontend reads `new URLSearchParams(window.location.search).get("node")`. It passes this `node` path to all Tauri commands (`get_device_state(node)`, `set_gain(node, ...)`), allowing multiple independent mixer windows to control multiple cards simultaneously.

## 5. Modular Device Family Architecture (Plugin-like Separation)
To keep implementations of different hardware families isolated and easy for future contributors to extend, both Backend and Frontend must be strictly organized by **Protocol / Device Family**:

### Backend Directory Structure (`src-tauri/src/`)
```text
src-tauri/src/
├── transport/          # Raw FireWire / hinawa wrappers (discovery, FwNode, FwReq, FwFcp)
├── families/           # Isolated protocol implementations (each self-contained)
│   ├── mod.rs          # Trait `DeviceFamilyDriver` & registry matching DeviceInfo -> Driver
│   ├── dice_eap/       # Everything specific to TCAT DICE EAP devices
│   │   ├── mod.rs      # Global status (Clock, Sample Rate, Nickname, Stream lock)
│   │   ├── mixer.rs    # Bus strip projection logic
│   │   ├── matrix.rs   # 16x18 Matrix mixer read/write & Peak meters
│   │   ├── router.rs   # DICE EAP Router patchbay
│   │   └── vendors/    # Vendor-specific extensions (focusrite.rs, maudio.rs, presonus.rs)
│   └── bebob/          # Placeholder/template for BridgeCo BeBoB (e.g. Saffire PRO 26 I/O)
└── commands/           # Thin Tauri IPC router delegating calls to active family driver
```

### Frontend Directory Structure (`src/`)
```text
src/
├── components/         # Shared UI primitives (RotaryKnob, PeakMeter, Dropdown, etc.)
├── families/           # Isolated UI workspaces per protocol family
│   ├── registry.ts     # Maps `device.family` ("dice_eap", "bebob", etc.) -> Root Workspace Component
│   ├── dice-eap/       # Complete UI workspace for all DICE EAP cards
│   │   ├── DiceWorkspace.tsx   # Header bar + Top-level Tabs shell
│   │   ├── DiceHeader.tsx      # Model, Nickname, Clock Source, Sample Rate, Stream LED
│   │   └── tabs/
│   │       ├── MixerTab.tsx    # Bus strip mixers (placeholder for now)
│   │       ├── MatrixTab.tsx   # Responsive Matrix Mixer (next stage)
│   │       ├── RouterTab.tsx   # Patchbay router (placeholder for now)
│   │       └── SettingsTab.tsx # Vendor & standalone settings (placeholder for now)
│   └── bebob/          # Isolated UI workspace for BeBoB devices (placeholder)
└── App.tsx             # Resolves `?node=/dev/fwN`, fetches DeviceInfo, renders family workspace
```

## 6. DICE EAP Window Layout Specification (`DiceWorkspace`)
Every DICE EAP device window shares the exact same top-level layout:
1. **Device Header Bar (directly below window titlebar):**
   - **Identity:** Device Model name (e.g., `Focusrite Saffire PRO 40`) + editable/displayed **Nickname** (stored in DICE Global section).
   - **Clock & Sync Controls:**
     - `Clock Source` dropdown (e.g., `Internal`, `ADAT`, `S/PDIF`, `WordClock` — populated dynamically from device capabilities).
     - `Sample Rate` dropdown (`44.1 kHz`, `48 kHz`, `88.2 kHz`, `96 kHz` — read/write via DICE Global section).
   - **Status Indicators:**
     - `Sync / Clock Lock` LED indicator (green when locked, gray when unlocked).
     - `Stream Active` LED indicator (shows if iso TX/RX audio streaming with PC is active).
2. **Top-Level Navigation Tabs:**
   - Clean horizontal tab bar right below the header with 4 tabs:
     - **`Mixer`** (Bus strip mixers)
     - **`Matrix`** (Full crosspoint matrix mixer)
     - **`Router`** (Hardware/DAW/Mixer routing matrix)
     - **`Settings`** (Vendor-specific hardware switches: +48V, Pad, Monitor Dim/Mute, Save to Flash)
3. **Viewport Discipline:**
   - The Header + Tabs must have a fixed compact height (`~72px` total), leaving 100% of the remaining window height (`calc(100vh - 72px)`) to the active tab content without vertical page overflow.
