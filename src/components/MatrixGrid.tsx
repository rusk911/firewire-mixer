import { Fragment, memo, useSyncExternalStore, type CSSProperties } from "react";
import type { MatrixState } from "../types";
import { matrixStore } from "../families/dice-eap/matrixStore";
import { RotaryCellKnob } from "./RotaryCellKnob";

interface MatrixGridProps {
  matrix: MatrixState;
  style: CSSProperties;
  onLocalChange: (row: number, col: number, db: number | null) => void;
  onCommit: (row: number, col: number, db: number | null) => void;
}

function MatrixGridView({ matrix, style, onLocalChange, onCommit }: MatrixGridProps) {
  return (
    <div className="matrix-grid" style={style}>
      <div className="matrix-corner" aria-hidden="true">OUT / IN</div>
      {matrix.column_labels.map((label, col) => (
        <div className="matrix-column-label" key={`column-${col}`} title={label}>{label}</div>
      ))}
      {matrix.row_labels.map((label, row) => (
        <Fragment key={`row-${row}`}>
          <div className="matrix-row-label" title={label}>{label}</div>
          {matrix.column_labels.map((_, col) => (
            <MatrixCell
              key={`cell-${row}-${col}`}
              row={row}
              col={col}
              onChange={(db) => onLocalChange(row, col, db)}
              onCommit={(db) => onCommit(row, col, db)}
            />
          ))}
        </Fragment>
      ))}
    </div>
  );
}

function MatrixCell({
  row,
  col,
  onChange,
  onCommit,
}: {
  row: number;
  col: number;
  onChange: (db: number | null) => void;
  onCommit: (db: number | null) => void;
}) {
  const db = useSyncExternalStore(
    matrixStore.subscribe,
    () => matrixStore.getGain(row, col),
    () => matrixStore.getGain(row, col),
  );
  const inputPeakDb = useSyncExternalStore(
    matrixStore.subscribe,
    () => matrixStore.getInputPeak(col),
    () => matrixStore.getInputPeak(col),
  );

  return (
    <RotaryCellKnob
      row={row}
      col={col}
      db={db}
      inputPeakDb={inputPeakDb}
      onChange={onChange}
      onCommit={onCommit}
    />
  );
}

export const MatrixGrid = memo(MatrixGridView, (previous, next) =>
  previous.matrix.node === next.matrix.node
  && previous.matrix.rows === next.matrix.rows
  && previous.matrix.cols === next.matrix.cols
  && previous.matrix.row_labels === next.matrix.row_labels
  && previous.matrix.column_labels === next.matrix.column_labels
  && previous.matrix.gains === next.matrix.gains,
);