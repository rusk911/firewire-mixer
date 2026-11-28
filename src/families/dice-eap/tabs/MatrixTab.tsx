import type { CSSProperties } from "react";
import type { DeviceInfo } from "../../../types";
import { matrixStore, useMatrixStore } from "../matrixStore";
import { MatrixGrid } from "../../../components/MatrixGrid";

export function MatrixTab({ device, node }: { device: DeviceInfo; node: string }) {
  const { matrix, mixerError } = useMatrixStore();

  if (!matrix) {
    return <div className="matrix-state-message">Reading DICE mixer matrix…</div>;
  }

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
        {mixerError && <span className="matrix-error" role="alert">{mixerError}</span>}
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