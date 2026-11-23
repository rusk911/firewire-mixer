import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { familyWorkspaceRegistry } from "./families/registry";
import type { DeviceInfo, DeviceFamily, DiceGlobalState } from "./types";

type LoadState = "loading" | "ready" | "error";

export default function App() {
  const node = new URLSearchParams(window.location.search).get("node") ?? "";
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [device, setDevice] = useState<DeviceInfo | null>(null);
  const [globalState, setGlobalState] = useState<DiceGlobalState | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let active = true;

    async function loadDevice() {
      if (!node) {
        setError("No FireWire node was specified for this window.");
        setLoadState("error");
        return;
      }

      try {
        const devices = await invoke<DeviceInfo[]>("list_devices");
        const current = devices.find((candidate) => candidate.node_path === node);
        if (!current) throw new Error(`No connected device found at ${node}.`);

        setDevice(current);
        if (current.family === "dice_eap") {
          const state = await invoke<DiceGlobalState>("get_dice_global_state", { node });
          if (active) setGlobalState(state);
        }
        if (active) setLoadState("ready");
      } catch (reason) {
        if (!active) return;
        setError(reason instanceof Error ? reason.message : String(reason));
        setLoadState("error");
      }
    }

    void loadDevice();
    return () => {
      active = false;
    };
  }, [node]);

  async function updateGlobalState(operation: Promise<DiceGlobalState>) {
    const updated = await operation;
    setGlobalState(updated);
    return updated;
  }

  if (loadState === "loading") {
    return <main className="launch-state">Connecting to FireWire device…</main>;
  }
  if (loadState === "error" || !device) {
    return (
      <main className="launch-state launch-state--error">
        <span className="eyebrow">FireMix · Device unavailable</span>
        <h1>Could not open mixer</h1>
        <p>{error || "The requested FireWire device is unavailable."}</p>
        <code>{node || "No node path"}</code>
      </main>
    );
  }

  const family = device.family as DeviceFamily | null;
  const Workspace = family ? familyWorkspaceRegistry[family] : undefined;
  if (!Workspace) {
    return (
      <main className="launch-state launch-state--error">
        <span className="eyebrow">FireMix · Unsupported family</span>
        <h1>{device.name}</h1>
        <p>No workspace is registered for this device family.</p>
        <code>{node}</code>
      </main>
    );
  }

  return (
    <Workspace
      device={device}
      node={node}
      globalState={globalState ?? undefined}
      onNicknameChange={(name) =>
        updateGlobalState(invoke<DiceGlobalState>("set_dice_nickname", { node, name }))
      }
      onClockSourceChange={(source) =>
        updateGlobalState(invoke<DiceGlobalState>("set_dice_clock_source", { node, source }))
      }
      onSampleRateChange={(rate) =>
        updateGlobalState(invoke<DiceGlobalState>("set_dice_sample_rate", { node, rate }))
      }
    />
  );
}