# Unified Mixer Architecture

## 1. Device Discovery Flow
1. Scan `/dev/fw0` .. `/dev/fw15`.
2. Open each with `hinawa::FwNode::new()`, call `node.open(&path)`.
3. Read Config ROM (`node.config_rom()`), parse IEEE 1212 root directory to extract `vendor_id` and `model_id`.
4. Match `(vendor_id, model_id)` against `HARDWARE_PROFILES.md`. Ignore host controllers (Linux FireWire OHCI nodes).
5. Instantiate the corresponding `Box<dyn AudioDevice>` backend.

## 2. Unified Backend Trait (`src-tauri/src/model/device.rs`)
Every card driver implements this trait:

```rust
pub trait AudioDevice: Send + Sync {
    fn info(&self) -> DeviceInfo;
    fn get_state(&mut self) -> Result<MixerState, String>;
    fn set_gain(&mut self, channel: usize, db: f32) -> Result<(), String>;
    fn set_pan(&mut self, channel: usize, pan: f32) -> Result<(), String>;
    fn set_mute(&mut self, channel: usize, muted: bool) -> Result<(), String>;
    fn set_phantom(&mut self, channel: usize, enabled: bool) -> Result<(), String>;
    fn set_routing(&mut self, src: PortId, dst: PortId) -> Result<(), String>;
    fn poll_meters(&mut self) -> Result<Vec<f32>, String>;
}
```

## 3. Tauri IPC Commands
- `list_devices() -> Vec<DeviceInfo>`
- `connect_device(node_path: String) -> MixerState`
- `dispatch_command(node_path: String, cmd: MixerCommand) -> MixerState`
- Event emitted from Rust to Frontend every 40ms: `meter-update` (payload: `{ node_path, peaks: f32[] }`).
