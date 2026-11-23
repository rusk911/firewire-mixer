# UI/UX & Backend Specification: DICE EAP Router (`RouterTab`)

## 1. Visual Concept (FFADO Crossbar Style)
Instead of a massive 2D boolean grid, implement a clean, compact **Destination-centric Crossbar Router** modeled after FFADO's Crossbar Router:
- Each **Destination** port on the hardware/bus is rendered as a compact card or row item.
- Above or inside the top edge of each Destination selector sits a **thin horizontal Peak Meter bar** (`3px`–`4px` height, no numeric scale) providing instant visual feedback on whether audio is currently reaching that destination.
- Below the meter bar is a **Dropdown (`<select>` / custom combobox)** listing all available **Source** ports grouped by hardware/stream block, plus a disconnected state.

## 2. Protocol & Hardware Mapping (`firewire-dice-protocols`)
Inspect `src-tauri/vendor/firewire-dice-protocols-*/src/tcat/extension/router_entry.rs` and `caps_entry.rs` before implementing:
- **Dynamic Port Discovery:**
  - Read current router entries (`RouterEntry { dst, src }`) and device router capabilities (`RouterCaps`: max routes, available `SrcBlk` and `DstBlk` types/counts).
  - Also read stream format names (`TxStreamFormat` / `RxStreamFormat` in `tcat/tx_stream_format.rs` & `rx_stream_format.rs`) to resolve real human-readable channel names (e.g., `Analog In 1/2`, `ADAT In 1..8`, `DAW Out 1..18`, `Mixer Out 1..18`) instead of raw block IDs.
- **Disconnected vs. Mute Verification:**
  - Check `SrcBlkId` variants in `router_entry.rs`. In TCAT DICE EAP, unrouted destinations are assigned `SrcBlkId::Mute` (`0x0f`).
  - If `Mute` and `Disconnected` map to the exact same hardware `SrcBlkId` value, expose **only ONE option** at the top of the dropdown labeled **`Disconnected`**. Only expose two separate options if the protocol defines distinct hardware IDs for them.

## 3. Grouping & Responsive Grid Layout
To fit all destinations comfortably without clutter:
- **Group Destinations by Block Type** into logical columns or collapsible sections (matching the hardware architecture):
  1. **Physical Outputs** (Analog Line Outs, Phones, S/PDIF Out, ADAT Outs)
  2. **DAW / FireWire Inputs (PC Capture)** (Iso TX streams sent to computer)
  3. **Mixer Inputs** (The 16 inputs feeding the DICE EAP Matrix Mixer)
- Within each group, arrange destination items in a responsive CSS Grid (`grid-template-columns: repeat(auto-fill, minmax(180px, 1fr))`) so it adapts cleanly to any window width.
- **Visual Cues:**
  - Highlight unassigned (`Disconnected`) dropdowns with dimmed/muted text so active routes stand out immediately.
  - Highlight destinations routed from `Mixer Out` vs direct hardware pass-through (`Analog In` / `DAW Out`) with subtle color accents.

## 4. Real-Time Peak Meters on Destinations
- DICE EAP `PeakEntry` (`peak_entry.rs`) reports peak levels for router entries (`RouterEntry` array).
- Reuse the existing ~40ms polling loop (only active while `RouterTab` or `MatrixTab` is visible) to push peak values for every active router destination to the frontend.
- Render the horizontal meter above each dropdown with smooth CSS/Canvas decay and standard Green -> Yellow -> Red color thresholds.

## 5. State Coupling with Matrix Mixer
- Changing a route that feeds a **Mixer Input** (`DstBlk::Mixer`) or a **Physical Output** (`DstBlk::Ins` / `Aes` / `Adat`) must immediately invalidate/update the row and column port labels in `MatrixTab` so both tabs always reflect live hardware routing.

## 6. Tauri IPC Commands (`families/dice_eap/router.rs`)
- `get_router_state(node: String) -> RouterState` (returns grouped destinations, available grouped sources, and current `dst -> src` assignments).
- `set_route(node: String, dst: PortId, src: Option<PortId>) -> Result<(), String>` (updates the specific entry in the DICE EAP router table and writes the updated route list to hardware; `None` maps to `SrcBlkId::Mute`).
