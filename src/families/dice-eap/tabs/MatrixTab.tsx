import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState, type CSSProperties } from "react";
import type { DeviceInfo } from "../../../types";
import type { MatrixState, MeterUpdate } from "../../../types";
import { matrixStore, useMatrixStore } from "../matrixStore";
import { MatrixGrid } from "../../../components/MatrixGrid";

export function MatrixTab({ device, node }: { device: DeviceInfo; node: string }) {
  const { matrix, mixerError } = useMatrixStore();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen<MeterUpdate>("meter-update", ({ payload }) => {
      if (payload.node === node) {
        matrixStore.setMeters(node, payload.input_peaks_db, payload.destination_peaks, payload.error !== null);
      }
    }).then((stopListening) => {
      if (disposed) stopListening();
      else unlisten = stopListening;
    }).catch((reason: unknown) => {
      if (!disposed) setError(reason instanceof Error ? reason.message : String(reason));
    });

    void invoke<MatrixState>("get_matrix_state", { node })
      .then((state) => {
        if (disposed) return;
        matrixStore.setMatrix(state);
        void matrixStore.loadMixer(node);
        setError("");
      })
      .catch((reason: unknown) => {
        if (!disposed) setError(reason instanceof Error ? reason.message : String(reason));
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [node]);

  if (loading && !matrix) {
    return <div className="matrix-state-message">Reading DICE mixer matrix…</div>;
  }
  if (error && !matrix) {
    return (
      <div className="matrix-state-message matrix-state-message--error" role="alert">
        <strong>Matrix unavailable</strong>
        <span>{error}</span>
        <code>{node}</code>
      </div>
    );
  }
  if (!matrix) return null;

  const gridStyle = {
    "--matrix-cols": matrix.cols,
    "--matrix-rows": matrix.rows,
    "--matrix-min-width": `${110 + matrix.cols * 36}px`,
    "--matrix-min-height": `${48 + matrix.rows * 36}px`,
  } as CSSProperties;

  return (
    <section className="matrix-tab" aria-label={`${device.name} mixer matrix`}>
      <div className="matrix-toolbar">
        <div>
          <strong>Crosspoint matrix</strong>
          <span>{matrix.rows} outputs × {matrix.cols} inputs</span>
        </div>
        {(error || mixerError) && <span className="matrix-error" role="alert">{error || mixerError}</span>}
      </div>
      <div className="matrix-viewport">
        <MatrixGrid
          matrix={matrix}
          style={gridStyle}
          onLocalChange={(row, col, db) => matrixStore.previewMatrixCell(row, col, db)}
          onCommit={() => matrixStore.commitMatrixEdits()}
        />
      </div>
      <div className="matrix-footer">
        <span>{device.nickname || device.name}</span>
        <span>{node}</span>
      </div>
    </section>
  );
}