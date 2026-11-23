# Supported & Test Hardware Profiles

All devices are identified via IEEE 1212 Config ROM (`vendor_id`, `model_id`) read from `/dev/fw*` via `hinawa::FwNode`.

## Group 1: TCAT DICE EAP (`firewire-dice-protocols`)
These 4 devices share the standard **DICE Extended Application Protocol (EAP)** (`tcat::extension`) for Mixer, Router, Peak meters, and Standalone flash storage, plus small vendor-specific sections.

1. **Presonus FireStudio Mobile** (Primary MVP Target)
   - Vendor ID: `0x000a92` | Model ID: `0x000011`
   - Protocol module: `firewire_dice_protocols::presonus::fstudio_mobile`
   - Features: 18x8 EAP mixer/router, stereo channel linking, output trim. No software-controlled preamps.

2. **M-Audio ProFire 2626**
   - Vendor ID: `0x000d6c` | Model ID: `0x000010` (verify via Config ROM)
   - Protocol module: `firewire_dice_protocols::maudio::profire2626`
   - Features: 26x26 EAP mixer/router, master volume knob assignment, optical port mode (ADAT/SPDIF), mute button.

3. **Focusrite Saffire PRO 26** (Grey chassis, DICE generation — NOT the older PRO 26 I/O)
   - Vendor ID: `0x00130e` | Model ID: `0x000012`
   - Protocol module: `firewire_dice_protocols::focusrite::saffirepro26`
   - Features: EAP mixer/router + Focusrite vendor block (Monitor Dim/Mute, HW/SW knob control, line/inst/pad switches).

4. **Focusrite Saffire PRO 40**
   - Vendor ID: `0x00130e` | Model ID: `0x000005` (observed Config ROM), `0x000007`, or `0x0000de` for other revisions
   - Protocol module: `firewire_dice_protocols::focusrite::saffirepro40`
   - Features: Same architecture as PRO 26, larger channel count (20x20), dual ADAT/SPDIF routing.

## Group 2: BridgeCo BeBoB (`firewire-bebob-protocols`)
Uses AV/C FCP (Function Control Protocol) transactions via `hinawa::FwFcp` instead of direct memory-mapped registers.

5. **Focusrite Saffire PRO 26 I/O** (Older silver/blue rack unit)
   - Vendor ID: `0x00130e` | Model ID: `0x000003`
   - Protocol module: `firewire_bebob_protocols::focusrite::saffirepro26io`
   - Features: BridgeCo Extended Audio Subunit mixer matrix + Focusrite AV/C vendor commands (+48V phantom power, clock source, front-panel knob modes).
