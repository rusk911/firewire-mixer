import { memo, useEffect, useEffectEvent, useRef, useState } from "react";
import { createPortal } from "react-dom";

interface RotaryCellKnobProps {
  row: number;
  col: number;
  db: number | null;
  inputPeakDb: number | null;
  onChange: (db: number | null) => void;
  onCommit: (db: number | null) => void;
}

interface DragState {
  startY: number;
  startDb: number;
  lastDb: number;
  moved: boolean;
}

interface TooltipState {
  x: number;
  y: number;
}

interface PendingWheelValue {
  db: number | null;
}

const MIN_DB = -60;
const MAX_DB = 12;
const START_ANGLE = 135;
const SWEEP_ANGLE = 270;
const WHEEL_COMMIT_DELAY_MS = 50;

function progressForDb(db: number): number {
  return Math.max(0, Math.min(1, (db - MIN_DB) / (MAX_DB - MIN_DB)));
}

function anglePoint(angle: number, radius: number): [number, number] {
  const radians = (angle * Math.PI) / 180;
  return [24 + radius * Math.cos(radians), 24 + radius * Math.sin(radians)];
}

function arcPath(start: number, end: number, radius: number): string {
  const [startX, startY] = anglePoint(start, radius);
  const [endX, endY] = anglePoint(end, radius);
  const largeArc = end - start > 180 ? 1 : 0;
  return `M ${startX} ${startY} A ${radius} ${radius} 0 ${largeArc} 1 ${endX} ${endY}`;
}

function formatDb(db: number | null): string {
  return db === null ? "-inf dB" : `${db.toFixed(1)} dB`;
}

function clampDb(db: number): number {
  return Math.max(MIN_DB, Math.min(MAX_DB, db));
}

function RotaryCellKnobView({ row, col, db, inputPeakDb, onChange, onCommit }: RotaryCellKnobProps) {
  const [previewDb, setPreviewDb] = useState<number | null>(db);
  const [tooltip, setTooltip] = useState<TooltipState | null>(null);
  const [editorPosition, setEditorPosition] = useState<TooltipState | null>(null);
  const [editorValue, setEditorValue] = useState("");
  const [editorError, setEditorError] = useState("");
  const editorInput = useRef<HTMLInputElement>(null);
  const knobRef = useRef<SVGSVGElement>(null);
  const drag = useRef<DragState | null>(null);
  const hoverTimer = useRef<number | undefined>(undefined);
  const wheelCommitTimer = useRef<number | undefined>(undefined);
  const pendingWheelValue = useRef<PendingWheelValue | null>(null);
  const tooltipPosition = useRef<TooltipState>({ x: 0, y: 0 });
  const onCommitRef = useRef(onCommit);

  useEffect(() => {
    if (!drag.current) setPreviewDb(db);
  }, [db]);

  useEffect(() => {
    if (editorPosition) editorInput.current?.focus();
  }, [editorPosition]);

  useEffect(() => {
    onCommitRef.current = onCommit;
  }, [onCommit]);

  useEffect(() => {
    const element = knobRef.current;
    if (!element) return;

    element.addEventListener("wheel", handleWheel, { passive: false });

    return () => {
      element.removeEventListener("wheel", handleWheel);
    };
  }, []);

  useEffect(() => () => {
    if (wheelCommitTimer.current !== undefined) window.clearTimeout(wheelCommitTimer.current);
    const pending = pendingWheelValue.current;
    if (pending) onCommitRef.current(pending.db);
  }, []);

  function clearHoverTimer() {
    if (hoverTimer.current !== undefined) window.clearTimeout(hoverTimer.current);
    hoverTimer.current = undefined;
  }

  function queueTooltip(event: React.PointerEvent<SVGSVGElement>) {
    tooltipPosition.current = { x: event.clientX, y: event.clientY };
    clearHoverTimer();
    hoverTimer.current = window.setTimeout(() => setTooltip(tooltipPosition.current), 1000);
  }

  function flushWheelCommit() {
    if (wheelCommitTimer.current !== undefined) window.clearTimeout(wheelCommitTimer.current);
    wheelCommitTimer.current = undefined;
    const pending = pendingWheelValue.current;
    pendingWheelValue.current = null;
    if (pending) onCommit(pending.db);
  }

  function cancelWheelCommit() {
    if (wheelCommitTimer.current !== undefined) window.clearTimeout(wheelCommitTimer.current);
    wheelCommitTimer.current = undefined;
    pendingWheelValue.current = null;
  }

  function beginDrag(event: React.PointerEvent<SVGSVGElement>) {
    if (event.button !== 0 || event.altKey || event.ctrlKey || event.shiftKey) return;
    flushWheelCommit();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = {
      startY: event.clientY,
      startDb: previewDb ?? MIN_DB,
      lastDb: previewDb ?? MIN_DB,
      moved: false,
    };
  }

  function moveDrag(event: React.PointerEvent<SVGSVGElement>) {
    const active = drag.current;
    if (!active) {
      queueTooltip(event);
      return;
    }

    const step = event.shiftKey ? 0.1 : 0.35;
    const next = clampDb(active.startDb + (active.startY - event.clientY) * step);
    if (Math.abs(event.clientY - active.startY) > 2) active.moved = true;
    if (active.moved && next !== active.lastDb) {
      active.lastDb = next;
      setPreviewDb(next);
      onChange(next);
      setTooltip({ x: event.clientX, y: event.clientY });
    }
    tooltipPosition.current = { x: event.clientX, y: event.clientY };
  }

  function endDrag(event: React.PointerEvent<SVGSVGElement>) {
    const active = drag.current;
    if (!active) return;
    drag.current = null;
    if (active.moved) onCommit(active.lastDb);
    if (!event.currentTarget.matches(":hover")) setTooltip(null);
  }

  function setValue(value: number | null) {
    cancelWheelCommit();
    setPreviewDb(value);
    onChange(value);
    onCommit(value);
  }

  function handleClick(event: React.MouseEvent<SVGSVGElement>) {
    if (event.altKey) {
      event.preventDefault();
      setValue(null);
    } else if (event.ctrlKey || event.metaKey) {
      event.preventDefault();
      setValue(-12);
    } else if (event.shiftKey) {
      event.preventDefault();
      setValue(MAX_DB);
    }
  }

  const handleWheel = useEffectEvent((event: WheelEvent) => {
    event.preventDefault();
    event.stopPropagation();
    const startDb = pendingWheelValue.current?.db ?? previewDb ?? MIN_DB;
    const next = clampDb(startDb + (event.deltaY < 0 ? 0.5 : -0.5));
    pendingWheelValue.current = { db: next };
    setPreviewDb(next);
    onChange(next);
    if (wheelCommitTimer.current !== undefined) window.clearTimeout(wheelCommitTimer.current);
    wheelCommitTimer.current = window.setTimeout(flushWheelCommit, WHEEL_COMMIT_DELAY_MS);
    setTooltip({ x: event.clientX, y: event.clientY });
  });

  function handleContextMenu(event: React.MouseEvent<SVGSVGElement>) {
    event.preventDefault();
    const currentDb = pendingWheelValue.current?.db ?? previewDb;
    flushWheelCommit();
    clearHoverTimer();
    setTooltip(null);
    setEditorValue(currentDb?.toFixed(1) ?? "-inf");
    setEditorError("");
    setEditorPosition({ x: event.clientX, y: event.clientY });
  }

  function applyEditorValue(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const normalized = editorValue.trim().toLowerCase();
    if (normalized === "-inf" || normalized === "-infinity") {
      setValue(null);
      setEditorPosition(null);
      return;
    }
    const value = Number(normalized);
    if (!Number.isFinite(value)) {
      setEditorError("Enter a dB value or -inf");
      return;
    }
    setValue(clampDb(value));
    setEditorPosition(null);
  }

  const knobProgress = previewDb === null ? 0 : progressForDb(previewDb);
  const pointerAngle = START_ANGLE + knobProgress * SWEEP_ANGLE;
  const postFaderPeak = inputPeakDb === null || previewDb === null ? null : inputPeakDb + previewDb;
  const meterProgress = postFaderPeak === null ? 0 : Math.max(0, Math.min(1, (postFaderPeak + 60) / 66));
  const meterColor = postFaderPeak === null ? "#596762" : postFaderPeak >= -3 ? "#f07163" : postFaderPeak >= -12 ? "#e5bf62" : "#6fc29b";
  const notchAngle = START_ANGLE + progressForDb(0) * SWEEP_ANGLE;
  const [notchStartX, notchStartY] = anglePoint(notchAngle, 14);
  const [notchEndX, notchEndY] = anglePoint(notchAngle, 17);
  const [pointerX, pointerY] = anglePoint(pointerAngle, 11);
  const tooltipStyle = tooltip
    ? {
        left: Math.max(8, Math.min(tooltip.x + 12, window.innerWidth - 214)),
        top: Math.max(8, tooltip.y - 74),
      }
    : undefined;

  return (
    <div className="rotary-cell">
      <svg
        ref={knobRef}
        aria-label={`Output ${row + 1}, input ${col + 1}: ${formatDb(previewDb)}`}
        className="rotary-svg"
        onClick={handleClick}
        onContextMenu={handleContextMenu}
        onDoubleClick={(event) => {
          event.preventDefault();
          setValue(0);
        }}
        onPointerDown={beginDrag}
        onPointerEnter={queueTooltip}
        onPointerLeave={() => {
          clearHoverTimer();
          if (!drag.current) setTooltip(null);
        }}
        onPointerMove={moveDrag}
        onPointerUp={endDrag}
        role="slider"
        tabIndex={0}
        viewBox="0 0 48 48"
      >
        <path d={arcPath(START_ANGLE, START_ANGLE + SWEEP_ANGLE, 21)} fill="none" stroke="#34423e" strokeWidth="2.5" />
        {meterProgress > 0 && (
          <path
            d={arcPath(START_ANGLE, START_ANGLE + SWEEP_ANGLE * meterProgress, 21)}
            fill="none"
            stroke={meterColor}
            strokeLinecap="round"
            strokeWidth="2.5"
          />
        )}
        <line x1={notchStartX} y1={notchStartY} x2={notchEndX} y2={notchEndY} stroke="#f1ca75" strokeWidth="1.3" />
        <circle cx="24" cy="24" r="13" fill="#202d2a" stroke="#53645d" strokeWidth="1" />
        <line x1="24" y1="24" x2={pointerX} y2={pointerY} stroke="#e9b18f" strokeLinecap="round" strokeWidth="2" />
        <circle cx="24" cy="24" r="2" fill="#d6e2dc" />
      </svg>
      {tooltip && (
        <div className="rotary-tooltip" style={tooltipStyle} role="tooltip">
          <strong>{formatDb(previewDb)}</strong>
          <span>Double-click · 0 dB</span>
          <span>Shift-click · +12 dB</span>
          <span>Ctrl-click · -12 dB</span>
          <span>Alt-click · -inf</span>
          <span>Right-click · enter value</span>
        </div>
      )}
      {editorPosition && createPortal(
        <form
          className="gain-editor"
          onContextMenu={(event) => event.preventDefault()}
          onSubmit={applyEditorValue}
          style={{
            left: Math.max(8, Math.min(editorPosition.x, window.innerWidth - 224)),
            top: Math.max(8, Math.min(editorPosition.y, window.innerHeight - 116)),
          }}
        >
          <label htmlFor={`gain-value-${row}-${col}`}>Gain · output {row + 1} / input {col + 1}</label>
          <div className="gain-editor-entry">
            <input
              ref={editorInput}
              id={`gain-value-${row}-${col}`}
              onChange={(event) => {
                setEditorValue(event.target.value);
                setEditorError("");
              }}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  event.preventDefault();
                  setEditorPosition(null);
                }
              }}
              value={editorValue}
            />
            <span>dB</span>
          </div>
          {editorError && <span className="gain-editor-error" role="alert">{editorError}</span>}
          <div className="gain-editor-actions">
            <button onClick={() => setEditorPosition(null)} type="button">Cancel</button>
            <button className="gain-editor-apply" type="submit">Apply</button>
          </div>
        </form>,
        document.body,
      )}
    </div>
  );
}

export const RotaryCellKnob = memo(
  RotaryCellKnobView,
  (previous, next) =>
    previous.row === next.row
    && previous.col === next.col
    && previous.db === next.db
    && previous.inputPeakDb === next.inputPeakDb,
);