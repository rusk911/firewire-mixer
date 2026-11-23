# UI/UX & Backend Specification: Focusrite-Style Strip Mixer (`MixerTab`) & State Persistence

## 1. Persistent Configuration Storage (`~/.config/firemix/`)
- **Storage Location:** `~/.config/firemix/` (resolve via `dirs::config_dir()`). Store per-device state files identified by hardware GUID/serial (e.g., `device_<guid>.json` or `.toml` — use human-readable `serde_json` pretty-printed or `toml` for Rust convenience and future preset export/import).
- **Complete Snapshot:** Save the entire device configuration:
  - Full raw matrix gains (`matrix[row][col]`), router assignments, clock source, sample rate.
  - All `MixerTab` UI state: bus stereo links, custom bus tab names, per-bus channel stereo links, custom channel labels, fader dB positions, pan values, `Mute`, `Solo`, `PFM` (Pre-Fader Meter) toggles, and software Master fader/pan positions.
- **Hardware Overwrite on Startup:** Because interfaces like *M-Audio ProFire 2626* boot with default standalone states under Linux, **the saved disk configuration is the authoritative source on app launch**. When a device connects/initializes, load its saved config file (if present) and immediately push all matrix levels, router entries, and global settings to the hardware DSP. Save state automatically on changes (debounced ~500ms) and on clean exit.

## 2. Bus Tabs & Stereo Bus Linking (16 Buses)
- **Sub-tabs inside `MixerTab`:** Render a horizontal bar of Mixer Buses corresponding to the hardware mixer outputs (e.g., 16 buses: `Mix Out 1` .. `Mix Out 16` by default; dynamically matching device mixer output count).
- **Custom Bus Names:** Clicking/double-clicking a bus tab title (or header label) allows renaming it (persisted to disk).
- **Master Stereo Button:**
  - Located in the Master section of the active bus. Available on all buses except the very last odd/final bus (since there is no right neighbor to pair with).
  - **Linking (`Bus N` + `Bus N+1`):** Merges tab `N` and tab `N+1` into a single stereo bus tab (default title `Mix Out 1/2`, or preserves left tab's custom name). The Master section widens slightly to accommodate stereo controls (stereo meters + pan slider).
  - **Unlinking:** Splits back into two mono tabs. Left tab keeps the custom name; right tab resets to default (`Mix Out N+1`).

## 3. Channel Strips Layout (18 Inputs + Pinned Master Section)
Each bus displays input channels (`row 0..N-1`, e.g., 18 channels) on the left and a **visually distinct Master Section pinned to the right edge** of the viewport:
- **Responsive Sizing & Overflow:** Channel strip width and fader height scale dynamically with window size. If the window becomes too narrow for minimum ergonomic strip widths, **only the input channels scroll horizontally** (`overflow-x: auto`), while the **Master Section stays pinned on the right** with a distinct background contrast.
- **Channel Strip Top-to-Bottom Anatomy:**
  1. **Source Selector Header:** Displays the current routed source name (synced with `RouterTab`) + dropdown triangle (`▼`) to reassign the source directly from the mixer.
     - *Mono Channel:* Lists all individual sources + `Disconnected`.
     - *Stereo Linked Channel:* Lists **only odd/even stereo pairs** (e.g., `Analog 1/2`, `DAW 1/2`, `ADAT 1/2`). Odd/arbitrary routing must be configured in `RouterTab`.
  2. **Stereo Link Button (`S` / `∞`):** Links channel `i` with `i+1` **for the current bus only**. Hidden on the last channel.
     - *Unlinking:* Left channel retains its custom user label; right channel resets its label to its routed source name.
  3. **PFM Button (`Pre-Fader Meter`):** Tooltip `"Pre-Fader Meter"`. When active, the strip's peak meter displays the raw input signal level before fader attenuation (logarithmic dB scale) to assist hardware preamp gain staging. When inactive, displays post-fader level.
  4. **Pan Slider (Visible ONLY when the active Bus is Stereo):**
     - Horizontal slider (`-100% L` .. `0 C` .. `+100% R`).
     - **Double-click:** Resets to Center (`0`).
     - **Right-click:** Opens a compact inline numeric input to type exact pan value manually.
     - **Hover > 1s:** Shows tooltip with exact current pan value.
  5. **Vertical Fader + Logarithmic Peak Meter:**
     - **Fader Scale:** Logarithmic visual taper in dB:
       - Mono Bus: `-inf` to **`+12.0 dB`**.
       - Stereo Bus: `-inf` to **`+6.0 dB`**.
     - **Fader Modifiers:**
       - `Double-click` -> `0.0 dB`
       - `Shift + Click` -> Max (`+6.0 dB` on Stereo bus, `+12.0 dB` on Mono bus)
       - `Ctrl + Click` -> `-12.0 dB`
       - `Alt + Click` -> `-inf`
     - **Peak Meter (to the right of the fader):** Logarithmic dB scale, stereo twin bars if channel is stereo-linked, single bar if mono.
  6. **Solo (`S`) & Mute (`M`) Buttons:** Placed side-by-side below the fader. Standard DAW solo-in-place logic per bus (soloing any channel(s) on a bus mutes unsoloed channels on that bus).
  7. **Editable User Label:** Text input at the bottom of the strip. Defaults to the routed source name; persists custom user text.

## 4. Master Section Anatomy (Right-Pinned)
- **Software-Only Gain Stage:** DICE EAP has no dedicated hardware master fader register; the Master Fader acts as a software multiplier/offset applied to all channels feeding that bus before computing final hardware matrix coefficients.
- **Scale & Controls:**
  - Logarithmic fader scale from **`-inf` to `0.0 dB`**.
  - **Stereo Bus Master:** Includes **Stereo Link** toggle, **Pan slider** (with double-click center, right-click manual input, >1s hover tooltip), and stereo peak meters.
  - **Master Peak Meters:** Derived from the hardware destination peak meters where `Mix Out N` (and `N+1`) are routed in `RouterTab`, scaled by the software Master fader position.
  - If Master Fader is at **`-inf`**, moving channel faders/pans on that bus updates in-memory/disk UI state **without writing changes to the hardware matrix** (all hardware cells for that bus remain at `-inf` / `0x0000`).

## 5. Exact Gain, Pan (-6 dB Law) & Matrix Priority Math

### A. Forward Calculation (`MixerTab` -> `MatrixTab` & Hardware DSP)
For a channel with fader level $F$ (in dB), mute state $M$, solo state, pan $P \in [-1, +1]$ (where $0$ is center), and Bus Master fader $M_{\text{bus}}$ (in dB, $\le 0$) and Master pan $P_{\text{bus}}$:
- If channel is muted (explicitly or via bus Solo) OR $F = -\infty$ OR $M_{\text{bus}} = -\infty$:
  - Hardware matrix cell(s) = $-\infty$ (`0x0000`).
- **Mono Bus (`col`):**
  - $\text{Matrix}[row][col] = \text{clamp}(F + M_{\text{bus}}, -\infty, +12.0\text{ dB})$
- **Stereo Bus (`col_L`, `col_R`) — `-6 dB` Center Pan Law:**
  - At center pan ($P = 0$) and $F = 0\text{ dB}$, **both** Left and Right matrix cells receive **`-6.0 dB`** (similarly, a Stereo Master at `0.0 dB` with center pan applies `-6.0 dB` to L and R).
  - Pan attenuation offsets $\Delta L(P), \Delta R(P)$ follow the `-6 dB` pan law (at $P = 0$: $\Delta L = -6\text{ dB}, \Delta R = -6\text{ dB}$; hard left $P = -1$: $\Delta L = 0\text{ dB}, \Delta R = -\infty$; hard right $P = +1$: $\Delta L = -\infty, \Delta R = 0\text{ dB}$, using linear amplitude pan $g_L = \frac{1 - P}{2}, g_R = \frac{1 + P}{2}$ converted to dB: $20\log_{10}(g)$).
  - Resulting hardware cells:
    - $\text{Matrix}[row][col_L] = F + \Delta L(P) + M_{\text{bus}} + \Delta L_{\text{master}}(P_{\text{bus}})$ (clamped to hardware max `+12 dB`).
    - $\text{Matrix}[row][col_R] = F + \Delta R(P) + M_{\text{bus}} + \Delta R_{\text{master}}(P_{\text{bus}})$ (clamped to hardware max `+12 dB`).

### B. Reverse Sync (`MatrixTab` -> `MixerTab`, Matrix Has Absolute Priority)
When the user directly turns a knob $\text{Matrix}[row][col]$ to a target hardware value $G_{\text{target}}$:
1. **Matrix Never Blocked:** `MixerTab` must NEVER block or clamp a change made in `MatrixTab`.
2. **Adjust Channel First:** Calculate the channel fader $F$ (and pan $P$ if stereo) needed to produce $G_{\text{target}}$ given the current $M_{\text{bus}}$.
3. **Push Master Fader Only When Channel Hits Ceiling:**
   - If producing $G_{\text{target}}$ requires $F$ to exceed the channel fader ceiling (`+12 dB` for mono, `+6 dB` for stereo) — including when $M_{\text{bus}} = -\infty$ and the user raises a matrix cell from $-\infty$:
   - Automatically raise $M_{\text{bus}}$ (up to `0.0 dB`) just enough so that $F$ sits cleanly at its ceiling (`+12 dB` or `+6 dB`) while achieving $G_{\text{target}}$ in the hardware matrix.
   - **Crucial Invariant for Sibling Channels:** When $M_{\text{bus}}$ is pushed upward by a matrix edit on channel $k$, **do NOT move the UI faders of other channels ($j \neq k$) on that bus** — instead, recalculate and write their hardware matrix cells using their existing fader/pan positions combined with the new $M_{\text{bus}}$ value.
