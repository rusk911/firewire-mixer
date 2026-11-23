import type { WorkspaceProps } from "../../types";

export function BebobWorkspace({ device, node }: WorkspaceProps) {
  return (
    <main className="launch-state">
      <span className="eyebrow">BeBoB · {node}</span>
      <h1>{device.name}</h1>
      <p>The BeBoB workspace is registered and ready for its family-specific controls.</p>
    </main>
  );
}