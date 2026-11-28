import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { DiceHeader } from "./DiceHeader";
import { MixerTab } from "./tabs/MixerTab";
import { MatrixTab } from "./tabs/MatrixTab";
import { RouterTab } from "./tabs/RouterTab";
import { SettingsTab } from "./tabs/SettingsTab";
import type { MatrixState, MeterUpdate, WorkspaceProps } from "../../types";
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
  const [meterListenerNode, setMeterListenerNode] = useState<string | null>(null);
  const meterCommands = useRef<Promise<void>>(Promise.resolve());
  const metersActive = activeTab === "Mixer" || activeTab === "Matrix" || activeTab === "Router";

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<MeterUpdate>("meter-update", ({ payload }) => {
      if (payload.node !== node) return;
      setMeterError(payload.error ?? "");
      matrixStore.setMeters(node, payload.input_peaks_db, payload.destination_peaks, payload.error !== null);
    }).then((stopListening) => {
      if (disposed) stopListening();
      else {
        unlisten = stopListening;
        setMeterListenerNode(node);
      }
    }).catch((reason: unknown) => {
      if (!disposed) {
        setMeterError(reason instanceof Error ? reason.message : String(reason));
      }
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [node]);

  useEffect(() => {
    let disposed = false;
    void invoke<MatrixState>("get_matrix_state", { node })
      .then((state) => {
        if (disposed) return;
        matrixStore.setMatrix(state);
        void matrixStore.loadMixer(node);
      })
      .catch((reason: unknown) => {
        if (!disposed) setError(reason instanceof Error ? reason.message : String(reason));
      });
    return () => { disposed = true; };
  }, [node]);

  useEffect(() => {
    if (!metersActive || meterListenerNode !== node) return;
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
  }, [metersActive, meterListenerNode, node]);

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
        {activeTab === "Mixer" && (
          <div className="workspace-pane">
            <MixerTab active device={device} node={node} />
          </div>
        )}
        {activeTab === "Matrix" && (
          <div className="workspace-pane">
            <MatrixTab device={device} node={node} />
          </div>
        )}
        {activeTab === "Router" && (
          <div className="workspace-pane">
            <RouterTab active device={device} node={node} />
          </div>
        )}
        {activeTab === "Settings" && (
          <div className="workspace-pane">
            <SettingsTab device={device} node={node} />
          </div>
        )}
      </main>
    </div>
  );
}