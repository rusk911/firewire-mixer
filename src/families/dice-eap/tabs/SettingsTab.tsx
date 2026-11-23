import type { DeviceInfo } from "../../../types";

export function SettingsTab({ device, node }: { device: DeviceInfo; node: string }) {
  return (
    <section className="tab-placeholder">
      <span className="eyebrow">{device.name}</span>
      <h1>Settings</h1>
      <p>Vendor and standalone settings workspace</p>
      <code>{node}</code>
    </section>
  );
}