# UI/UX & Backend Specification: Responsive Matrix Mixer (DICE EAP)

## 1. Window Persistence & Responsive Layout
- **Window State Persistence:** Save and restore window width, height, and `(x, y)` position per device model/GUID (using `tauri-plugin-window-state` or custom JSON state in app config dir).
- **Zero-Scroll Responsive Grid:** The matrix mixer must fit entirely inside `1366x768` / `1400x900` (accounting for OS panels and window decorations) **without scrollbars** under normal desktop sizes.
  - Use CSS Grid (`grid-template-columns: minmax(110px, auto) repeat(var(--cols), minmax(36px, 1fr))` and `grid-template-rows: minmax(48px, auto) repeat(var(--rows), minmax(36px, 1fr))`).
  - Every cell control (rotary knob + ring meter) scales proportionally via CSS `clamp()` / container queries (`cqmin`) down to a reasonable minimum size (`~34px x 34px` per cell), only enabling scrollbars if the window is shrunk below minimum ergonomic thresholds (`< 900x650`).

## 2. Matrix Orientation & Dynamic Dimensions (No Hardcoding)
- **Orientation:**
  - **Rows (Y-axis, left headers):** Mixer outputs / Buses (typically 16 rows on DICE EAP, e.g. `18x16` matrix — horizontal dimension is wider than vertical to match widescreen monitors).
  - **Columns (X-axis, top headers):** Mixer inputs (typically 18 columns on DICE EAP).
- **Dynamic Source of Truth:** Query actual `input_count`, `output_count`, and current **Router connections** from the device via `firewire-dice-protocols` (`tcat::extension::router_entry` & `mixer_entry`) when opening the window.
- **Port Labels in Headers:** Resolve what physical/stream port is routed into each Mixer Input (column header, e.g., `Mic/Line 1`, `ADAT 1`, `DAW 1`) and where each Mixer Output is routed (row header, e.g., `Line Out 1`, `Phones 1 L`, `ADAT Out 1`). Fallback to `Mix In N` / `Mix Out N` if unrouted.

## 3. Cell Control: Rotary Knob + Ring Peak Meter
Each cell `(row, col)` is a single compact SVG/Canvas widget:
- **Outer Ring Peak Meter (2–3px stroke):**
  - Renders real-time signal level around the knob perimeter (e.g., 270° arc matching the knob travel) POST FADER, e.g. input value adjusted by knob value (EAP does not report per cell peak values, only mixer input source).
  - Color zones: Green (`-inf` to `-12 dB`), Yellow (`-12 dB` to `-3 dB`), Red (`-3 dB` to `0 dB+`).
  - Driven by `meter-update` events from Rust (polling DICE EAP `PeakEntry` every ~40ms while the window is visible).
- **Inner Rotary Knob:**
  - **Linear dB Scale:** Visual angle (270° sweep, from 7 o'clock to 5 o'clock) maps **linearly in decibels**:
    - Bottom stop (0%): `-inf` (hardware coefficient `0x0000`)
    - Linear scale from `-60.0 dB` up to `+12.0 dB` (100%).
    - Clear visual tick/notch at `0.0 dB`.
  - **Mouse Interaction:**
    - Vertical drag (or mouse wheel) adjusts dB smoothly; `Shift + Drag` enables fine-tuning (`0.1 dB` steps).
    - **Double-Click:** Set to `0.0 dB`.
    - **Shift + Click:** Set to maximum `+12.0 dB`.
    - **Ctrl + Click:** Set to `-12.0 dB`.
    - **Alt + Click:** Set to `-inf` (mute cell).
    - **Right Click:** text field popup to enter value manually.
  - Show a compact floating tooltip with exact dB value (e.g., `-4.5 dB` or `-inf`) after 1 second of hover idle cursor. Below value list the above shortcuts.


## 4. State Synchronization & Bus Strip Coupling (Single Source of Truth)
- **Initial Sync Only:** On window open, Rust reads the entire mixer coefficient matrix from the DICE hardware once (`read_mixer_state`), converts raw DSP coefficients to dB, and sends them to the frontend.
- **In-Memory Manipulation:** Subsequent user drags write directly to the hardware DSP (`write_mixer_gain`) AND update the in-memory `MixerMatrixState` without redundant hardware reads.
- **Coupling with Future Bus Strip Mixers (Fader / Pan / Mute / Solo):**
  - The raw matrix `gains[row][col]` is the **single source of truth**.
  - A Bus Strip channel (e.g., stereo output pair `col_L, col_R`) is a mathematical projection of two columns in the matrix:
    - `strip.fader_db = max(gain_L, gain_R)`
    - `strip.pan` (`-1.0`..`+1.0`) derived from the L/R dB balance.
    - `strip.mute` / `strip.solo` stored in frontend/backend state, temporarily writing `-inf` (`0x0000`) to the hardware cell while restoring the stored dB value when unmuted/unsoloed.
  - Design the Zustand/React store (or state module) so that moving any matrix cell `(row, col)` immediately recalculates the derived `fader`, `pan`, `mute`, and `solo` state for the corresponding Bus Strip, and vice versa.
