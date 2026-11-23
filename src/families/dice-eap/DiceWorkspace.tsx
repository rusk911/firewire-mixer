import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { DiceHeader } from "./DiceHeader";
import { MixerTab } from "./tabs/MixerTab";
import { MatrixTab } from "./tabs/MatrixTab";
import { RouterTab } from "./tabs/RouterTab";
import { SettingsTab } from "./tabs/SettingsTab";
import type { MeterUpdate, WorkspaceProps } from "../../types";
import { matrixStore } from "./matrixStore";

const tabs = ["Mixer", "Matrix", "Router", "Settings"] as const;
type Tab = (typeof tabs)[number];

export function DiceWorkspace({
  device,
  node,
  globalState,
  onNicknameChange,
  onClockSourceChange,
  onSampleRateChange,
}: WorkspaceProps) {
  const [activeTab, setActiveTab] = useState<Tab>("Mixer");
  const [error, setError] = useState("");
  const [meterError, setMeterError] = useState("");
  const meterCommands = useRef<Promise<void>>(Promise.resolve());
  const metersActive = activeTab === "Mixer" || activeTab === "Matrix" || activeTab === "Router";

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<MeterUpdate>("meter-update", ({ payload }) => {
      if (payload.node === node) setMeterError(payload.error ?? "");
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch((reason: unknown) => {
      if (!disposed) setMeterError(reason instanceof Error ? reason.message : String(reason));
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [node]);

  useEffect(() => {
    if (!metersActive) return;
    meterCommands.current = meterCommands.current.then(() => invoke<void>("start_meter_polling", { node }))
      .catch((reason: unknown) => {
        setError(reason instanceof Error ? reason.message : String(reason));
      });
    return () => {
      meterCommands.current = meterCommands.current.then(() => invoke<void>("stop_meter_polling", { node }))
        .catch((reason: unknown) => {
          setError(reason instanceof Error ? reason.message : String(reason));
        });
    };
  }, [metersActive, node]);

  async function safelyUpdate<T>(operation: ((value: T) => Promise<unknown>) | undefined, value: T) {
    if (!operation || value === undefined) return;
    try {
      await operation(value);
      matrixStore.persist();
      setError("");
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
  }

  if (!globalState) {
    return <main className="launch-state">Reading DICE Global Section…</main>;
  }

  return (
    <div className="dice-workspace">
      <DiceHeader
        device={device}
        state={globalState}
        onNicknameSave={(name) => safelyUpdate(onNicknameChange, name)}
        onClockSourceSelect={(source) => safelyUpdate(onClockSourceChange, source)}
        onSampleRateSelect={(rate) => safelyUpdate(onSampleRateChange, rate)}
      />
      <nav className="workspace-tabs" aria-label="Device workspace">
        {tabs.map((tab) => (
          <button
            aria-current={activeTab === tab ? "page" : undefined}
            className={activeTab === tab ? "workspace-tab is-active" : "workspace-tab"}
            key={tab}
            onClick={() => setActiveTab(tab)}
            type="button"
          >
            {tab}
          </button>
        ))}
      </nav>
      <main className="workspace-content">
        {error && <div className="workspace-error" role="alert">{error}</div>}
        {!error && metersActive && meterError && (
          <div className="workspace-error" role="alert">Meter polling: {meterError}</div>
        )}
        <div className="workspace-pane" hidden={activeTab !== "Mixer"}>
          <MixerTab active={activeTab === "Mixer"} device={device} node={node} />
        </div>
        <div className="workspace-pane" hidden={activeTab !== "Matrix"}>
          <MatrixTab device={device} node={node} />
        </div>
        <div className="workspace-pane" hidden={activeTab !== "Router"}>
          <RouterTab active={activeTab === "Router"} device={device} node={node} />
        </div>
        <div className="workspace-pane" hidden={activeTab !== "Settings"}>
          <SettingsTab device={device} node={node} />
        </div>
      </main>
    </div>
  );
}