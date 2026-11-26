# UI/UX & Backend Specification: Focusrite-Style Strip Mixer (`MixerTab`) & State Persistence

## 1. Persistent Configuration Storage (`~/.config/firemix/`)
- **Storage Location:** `~/.config/firemix/` (resolve via `dirs::config_dir()`). Store per-device state files identified by hardware GUID/serial (e.g., `<guid>.json` — human-readable `serde_json` pretty-printed).
- **Complete Snapshot:** Save the entire device configuration:
  - Full raw matrix gains (`matrix[row][col]`), router assignments, clock source, sample rate.
  - All `MixerTab` UI state: bus stereo links, custom bus tab names, per-bus channel stereo links, custom channel labels, fader dB positions, channel pan/balance values, `Mute`, `Solo`, `PFM` (Pre-Fader Meter) toggles, and software Master fader positions.
- **Hardware Overwrite on Startup:** Because interfaces like *M-Audio ProFire 2626* boot with default standalone states under Linux, **the saved disk configuration is the authoritative source on app launch**. When a device connects/initializes, load its saved config file (if present) and immediately push all matrix levels, router entries, and global settings to the hardware DSP. Save state automatically on changes (debounced ~500ms) and on clean exit.

## 2. Bus Tabs & Stereo Bus Linking (16 Buses)
- **Sub-tabs inside `MixerTab`:** Render a horizontal bar of Mixer Buses corresponding to the hardware mixer outputs (e.g., 16 buses: `Mix Out 1` .. `Mix Out 16` by default; dynamically matching device mixer output count).
- **Custom Bus Names:** Clicking/double-clicking a bus tab title (or header label) allows renaming it (persisted to disk).
- **Master Stereo Button:**
  - Located in the Master section of the active bus. Available on all buses except the very last odd/final bus (since there is no right neighbor to pair with).
  - **Linking (`Bus N` + `Bus N+1`):** Merges tab `N` and tab `N+1` into a single stereo bus tab (default title `Mix Out 1/2`, or preserves left tab's custom name). The Master section displays stereo peak meters without a master pan control.
  - **Unlinking:** Splits back into two mono tabs. Left tab keeps the custom name; right tab resets to default (`Mix Out N+1`).

## 3. Channel Strips Layout (18 Inputs + Pinned Master Section)
Each bus displays input channels (`col 0..N-1`, e.g., 18 channels) on the left and a **visually distinct Master Section pinned to the right edge** of the viewport:
- **Responsive Sizing & Overflow:** Channel strip width and fader height scale dynamically with window size. If the window becomes too narrow for minimum ergonomic strip widths, **only the input channels scroll horizontally** (`overflow-x: auto`), while the **Master Section stays pinned on the right** with a distinct background contrast.
- **Channel Strip Top-to-Bottom Anatomy:**
  1. **Source Selector Header:** Displays the current routed source name (synced with `RouterTab`) + dropdown triangle (`▼`) to reassign the source directly from the mixer.
     - *Mono Channel:* Lists all individual sources + `Disconnected`.
     - *Stereo Linked Channel:* Lists **only odd/even stereo pairs** (e.g., `Analog 1/2`, `DAW 1/2`, `ADAT 1/2`). Odd/arbitrary routing must be configured in `RouterTab`.
  2. **Stereo Link Button (`Stereo`):** Links channel `i` with `i+1` **for the current bus only**. Disabled on the last channel or when the right neighbor is already linked.
     - *Unlinking:* Left channel retains its custom user label; right channel resets its label to its routed source name.
  3. **PFM Button (`Pre-Fader Meter`):** Tooltip `"Pre-Fader Meter"`. When active, the strip's peak meter displays the raw input signal level before fader attenuation (logarithmic dB scale) to assist hardware preamp gain staging. When inactive, displays post-fader level.
  4. **Pan / Balance Slider (Visible ONLY when the active Bus is Stereo):**
     - Horizontal slider (`-100% L` .. `0 C` .. `+100% R`). Acts as a pan control for mono channels and a left/right balance control for stereo-linked channels.
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
- **Software-Only Gain Stage:** DICE EAP has no dedicated hardware master fader register; the Master Fader acts as a pure software attenuator/offset (`-inf` to `0.0 dB`) applied equally to both left and right outputs of that bus before computing final hardware matrix coefficients. The Master section does **not** have a pan control and does **not** apply a `-6 dB` pan law.
- **Scale & Controls:**
  - Logarithmic fader scale from **`-inf` to `0.0 dB`** (attenuates only; `0.0 dB` passes channel levels unmodified).
  - **Stereo Bus Master:** Includes **Stereo Link** toggle and twin stereo peak meters (no master pan slider).
  - **Master Peak Meters:** Derived from the hardware destination peak meters where `Mix Out N` (and `N+1`) are routed in `RouterTab`, scaled by the software Master fader position.
  - If Master Fader is at **`-inf`**, moving channel faders/pans on that bus updates in-memory/disk UI state **without writing changes to the hardware matrix** (all hardware cells for that bus remain at `-inf` / `0x0000`).

## 5. Exact Gain, Pan (-6 dB Law) & Matrix Priority Math

### A. Forward Calculation (`MixerTab` -> `MatrixTab` & Hardware DSP)
For a channel with fader level $F$ (in dB), mute state $M$, solo state, pan/balance $P \in [-1, +1]$ (where $0$ is center), and Bus Master fader $M_{\text{bus}} \in [-\infty, 0.0\text{ dB}]$:
- If channel is muted (explicitly or via bus Solo) OR $F = -\infty$ OR $M_{\text{bus}} = -\infty$:
  - Hardware matrix cell(s) = $-\infty$ (`0x0000`).
- **Pan Attenuation Law (`-6 dB` Center Law — Applied to Channel Only):**
  - Pan offsets $\Delta L(P), \Delta R(P)$ use linear amplitude pan $g_L = \frac{1 - P}{2}, g_R = \frac{1 + P}{2}$ converted to dB: $20\log_{10}(g)$.
  - At center ($P = 0$): $\Delta L \approx -6.02\text{ dB}, \Delta R \approx -6.02\text{ dB}$.
  - Hard left ($P = -1$): $\Delta L = 0\text{ dB}, \Delta R = -\infty$.
  - Hard right ($P = +1$): $\Delta L = -\infty, \Delta R = 0\text{ dB}$.
  - Master fader $M_{\text{bus}}$ does **not** apply any pan offset.

1. **Mono Channel (`col`) on Mono Bus (`row`):**
   - $\text{Matrix}[row][col] = \text{clamp}(F + M_{\text{bus}}, -\infty, +12.0\text{ dB})$

2. **Stereo Linked Channel (`col_L`, `col_R`) on Mono Bus (`row`):**
   - Both left and right inputs feed the single mono output equally without pan attenuation:
   - $\text{Matrix}[row][col_L] = \text{clamp}(F + M_{\text{bus}}, -\infty, +12.0\text{ dB})$
   - $\text{Matrix}[row][col_R] = \text{clamp}(F + M_{\text{bus}}, -\infty, +12.0\text{ dB})$

3. **Mono Channel (`col`) on Stereo Bus (`row_L`, `row_R`):**
   - The mono source is panned across both bus outputs:
   - $\text{Matrix}[row_L][col] = \text{clamp}(F + \Delta L(P) + M_{\text{bus}}, -\infty, +12.0\text{ dB})$
   - $\text{Matrix}[row_R][col] = \text{clamp}(F + \Delta R(P) + M_{\text{bus}}, -\infty, +12.0\text{ dB})$
   - *Example:* At $F = +6.0\text{ dB}$, $M_{\text{bus}} = 0.0\text{ dB}$, center pan ($P = 0$) yields $\approx 0.0\text{ dB}$ in both $row_L$ and $row_R$; hard left ($P = -1$) yields $+6.0\text{ dB}$ in $row_L$ and $-\infty$ in $row_R$.

4. **Stereo Linked Channel (`col_L`, `col_R`) on Stereo Bus (`row_L`, `row_R`):**
   - Left input routes **exclusively** to Left output (`row_L`), and Right input routes **exclusively** to Right output (`row_R`). Cross-feed cells are always $-\infty$ (`0x0000`), and $P$ acts as a stereo balance control:
   - $\text{Matrix}[row_L][col_L] = \text{clamp}(F + \Delta L(P) + M_{\text{bus}}, -\infty, +12.0\text{ dB})$
   - $\text{Matrix}[row_L][col_R] = -\infty$
   - $\text{Matrix}[row_R][col_L] = -\infty$
   - $\text{Matrix}[row_R][col_R] = \text{clamp}(F + \Delta R(P) + M_{\text{bus}}, -\infty, +12.0\text{ dB})$
   - *Example:* At $F = +6.0\text{ dB}$, $M_{\text{bus}} = 0.0\text{ dB}$, center balance ($P = 0$) yields $\approx 0.0\text{ dB}$ at $[row_L][col_L]$ and $[row_R][col_R]$, and $-\infty$ at $[row_L][col_R]$ and $[row_R][col_L]$. Moving balance hard left ($P = -1$) raises $[row_L][col_L]$ to $+6.0\text{ dB}$ and attenuates $[row_R][col_R]$ to $-\infty$.

### B. Reverse Sync (`MatrixTab` -> `MixerTab`, Matrix Has Absolute Priority)
When the user directly turns a knob $\text{Matrix}[row][col]$ to a target hardware value $G_{\text{target}}$:
1. **Matrix Never Blocked:** `MixerTab` must NEVER block or clamp a change made in `MatrixTab`. (For a stereo channel on a stereo bus, only the diagonal cells $[row_L][col_L]$ and $[row_R][col_R]$ control the channel strip; cross-cells remain $-\infty$).
2. **Adjust Channel First:** Calculate the channel fader $F$ (and pan/balance $P$ if stereo bus) needed to produce $G_{\text{target}}$ given the current $M_{\text{bus}}$.
3. **Push Master Fader Only When Channel Hits Ceiling:**
• If producing $G_{\text{target}}$ requires $F$ to exceed the channel fader ceiling (+12 dB for mono bus, +6 dB for stereo bus) — including when $M_{\text{bus}} = -\infty$ and the user raises a matrix cell from $-\infty$:
• Automatically raise $M_{\text{bus}}$ (up to 0.0 dB) just enough so that $F$ sits cleanly at its ceiling (+12 dB or +6 dB) while achieving $G_{\text{target}}$ in the hardware matrix.
• Crucial Invariant for Sibling Channels: When $M_{\text{bus}}$ is pushed upward by a matrix edit on channel $k$, do NOT move the UI faders of other channels ($j \neq k$) on that bus — instead, recalculate and write their hardware matrix cells using their existing fader/pan positions combined with the new $M_{\text{bus}}$ value.