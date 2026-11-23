import { invoke } from "@tauri-apps/api/core";
import { useSyncExternalStore } from "react";
import type { BusStripProjection, DestinationPeak, MatrixState, MixerBus, MixerChannel, MixerState, RouterState } from "../../types";
import { createMixerState, editMatrixGain, projectBus, type MatrixChange } from "./mixerMath";

export interface MatrixSnapshot {
  matrix: MatrixState | null;
  busStrips: BusStripProjection[];
  routerState: RouterState | null;
  mixer: MixerState | null;
  mixerLoaded: boolean;
  mixerError: string;
}

let snapshot: MatrixSnapshot = { matrix: null, busStrips: [], routerState: null, mixer: null, mixerLoaded: false, mixerError: "" };
const listeners = new Set<() => void>();
const portLabels = new Map<string, { rows: string[]; columns: string[] }>();
const MIN_PEAK_UI_INTERVAL_MS = 50;
let lastPeakUiUpdate = 0;
const pendingWrites = new Map<string, MatrixChange>();
const previewWrites = new Map<string, MatrixChange>();
const stagedWrites = new Map<string, MatrixChange>();
let writeInFlight = false;
let mixerRevision = 0;
let saveTimer: ReturnType<typeof setTimeout> | undefined;
let saveInFlight = false;
let saveRequested = false;

function publish(next: MatrixSnapshot) {
  snapshot = next;
  listeners.forEach((listener) => listener());
}

function stageChanges(changes: MatrixChange[]) {
  const { matrix, mixer, mixerLoaded } = snapshot;
  if (!matrix || !mixer || !mixerLoaded) return;
  for (const change of changes) stagedWrites.set(`${change.row}:${change.col}`, change);
  mixerRevision += 1;
  void invoke("stage_mixer_configuration", {
    node: matrix.node,
    revision: mixerRevision,
    mixerState: mixer,
    updates: [...stagedWrites.values()],
  }).catch((reason: unknown) => {
    if (snapshot.matrix?.node === matrix.node) {
      publish({ ...snapshot, mixerError: reason instanceof Error ? reason.message : String(reason) });
    }
  });
}

function saveLater() {
  if (!snapshot.mixerLoaded || !snapshot.mixer || !snapshot.matrix) return;
  saveRequested = true;
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => { void saveConfiguration(); }, 500);
}

async function saveConfiguration() {
  if (!saveRequested || saveInFlight || writeInFlight || pendingWrites.size > 0 || previewWrites.size > 0) {
    if (saveRequested) saveTimer = setTimeout(() => { void saveConfiguration(); }, 500);
    return;
  }
  const { matrix, mixer, mixerLoaded } = snapshot;
  if (!matrix || !mixer || !mixerLoaded) return;
  saveRequested = false;
  saveInFlight = true;
  const revision = mixerRevision;
  try {
    await invoke("save_mixer_configuration", { node: matrix.node, mixerState: mixer, revision });
    if (mixerRevision === revision) stagedWrites.clear();
  } catch (reason) {
    publish({ ...snapshot, mixerError: reason instanceof Error ? reason.message : String(reason) });
  } finally {
    saveInFlight = false;
    if (saveRequested) saveLater();
  }
}

function applyChanges(matrix: MatrixState, changes: MatrixChange[]): MatrixState {
  const gains = matrix.gains.map((row) => [...row]);
  for (const change of changes) {
    if (gains[change.row]?.[change.col] !== undefined) gains[change.row][change.col] = change.db;
  }
  return { ...matrix, gains };
}

function queueChanges(changes: MatrixChange[]) {
  for (const change of changes) pendingWrites.set(`${change.row}:${change.col}`, change);
  stageChanges(changes);
  if (pendingWrites.size > 0) void drainWrites();
  else saveLater();
}

async function drainWrites() {
  if (writeInFlight || !snapshot.matrix) return;
  writeInFlight = true;
  const node = snapshot.matrix.node;
  try {
    while (pendingWrites.size > 0) {
      const updates = [...pendingWrites.values()];
      pendingWrites.clear();
      await invoke<MatrixState>("set_matrix_cells", { node, updates });
    }
    saveLater();
  } catch (reason) {
    pendingWrites.clear();
    previewWrites.clear();
    stagedWrites.clear();
    saveRequested = false;
    mixerRevision += 1;
    const message = reason instanceof Error ? reason.message : String(reason);
    try {
      await invoke("discard_mixer_configuration", { node, revision: mixerRevision });
      const matrix = await invoke<MatrixState>("get_matrix_state", { node });
      if (snapshot.matrix?.node === node) {
        publish({ ...snapshot, matrix, mixer: createMixerState(matrix), mixerLoaded: false, mixerError: message });
      }
    } catch (reloadReason) {
      publish({ ...snapshot, mixerLoaded: false, mixerError: `${message}; ${String(reloadReason)}` });
    }
  } finally {
    writeInFlight = false;
    if (pendingWrites.size > 0) void drainWrites();
  }
}

function amplitude(db: number | null): number {
  return db === null ? 0 : 10 ** (db / 20);
}

function projectBusStrips(matrix: MatrixState, previous: BusStripProjection[]): BusStripProjection[] {
  const previousByPair = new Map(previous.map((strip) => [strip.pair, strip]));
  const strips: BusStripProjection[] = [];

  for (let row = 0; row < matrix.rows; row += 2) {
    const left = matrix.gains[row] ?? [];
    const right = matrix.gains[row + 1] ?? left;
    const leftPower = left.reduce<number>((total, db) => total + amplitude(db) ** 2, 0);
    const rightPower = right.reduce<number>((total, db) => total + amplitude(db) ** 2, 0);
    const allGains = [...left, ...right].filter((db): db is number => db !== null);
    const totalPower = leftPower + rightPower;
    const prior = previousByPair.get(row / 2);

    strips.push({
      pair: row / 2,
      fader_db: allGains.length ? Math.max(...allGains) : null,
      pan: totalPower === 0 ? 0 : (rightPower - leftPower) / totalPower,
      mute: allGains.length === 0,
      solo: prior?.solo ?? false,
    });
  }

  return strips;
}

export const matrixStore = {
  getSnapshot(): MatrixSnapshot {
    return snapshot;
  },

  getGain(row: number, col: number): number | null {
    return snapshot.matrix?.gains[row]?.[col] ?? null;
  },

  getInputPeak(col: number): number | null {
    return snapshot.matrix?.input_peaks_db[col] ?? null;
  },

  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },

  setMatrix(matrix: MatrixState): void {
    const current = snapshot.matrix;
    const labels = portLabels.get(matrix.node);
    let nextMatrix = current?.node === matrix.node
      ? { ...matrix, input_peaks_db: current.input_peaks_db }
      : matrix;
    if (labels?.rows.length === nextMatrix.rows && labels.columns.length === nextMatrix.cols) {
      nextMatrix = { ...nextMatrix, row_labels: labels.rows, column_labels: labels.columns };
    }
    publish({
      ...snapshot,
      matrix: nextMatrix,
      busStrips: projectBusStrips(nextMatrix, snapshot.busStrips),
      mixer: current?.node === matrix.node ? snapshot.mixer ?? createMixerState(nextMatrix) : createMixerState(nextMatrix),
      mixerLoaded: current?.node === matrix.node ? snapshot.mixerLoaded : false,
    });
  },

  async loadMixer(node: string): Promise<void> {
    try {
      const { mixer: saved, revision } = await invoke<{ mixer: MixerState | null; revision: number }>("get_saved_mixer_state", { node });
      if (snapshot.matrix?.node !== node) return;
      if (saved && (saved.buses.length !== snapshot.matrix.rows
        || saved.buses.some((bus) => bus.channels.length !== snapshot.matrix?.cols))) {
        throw new Error("Saved mixer dimensions do not match the active matrix");
      }
      mixerRevision = revision;
      publish({ ...snapshot, mixer: saved ?? snapshot.mixer ?? createMixerState(snapshot.matrix), mixerLoaded: true, mixerError: "" });
      stageChanges([]);
    } catch (reason) {
      if (snapshot.matrix?.node === node) {
        publish({ ...snapshot, mixerLoaded: false, mixerError: reason instanceof Error ? reason.message : String(reason) });
      }
    }
  },

  updateBus(row: number, patch: Partial<Omit<MixerBus, "channels">>, writeMatrix = true): void {
    const { mixer, matrix } = snapshot;
    if (!mixer || !matrix || !mixer.buses[row]) return;
    const next = structuredClone(mixer);
    Object.assign(next.buses[row], patch);
    if ("master_pan" in patch) {
      for (const channel of next.buses[row].channels) channel.matrix_overrides = [null, null];
    }
    const changes = writeMatrix ? projectBus(next, row) : [];
    const effective = changes.filter((change) => matrix.gains[change.row]?.[change.col] !== change.db);
    publish({ ...snapshot, mixer: next, matrix: applyChanges(matrix, effective) });
    queueChanges(effective);
  },

  updateChannel(row: number, col: number, patch: Partial<MixerChannel>, writeMatrix = true): void {
    const { mixer, matrix } = snapshot;
    if (!mixer || !matrix || !mixer.buses[row]?.channels[col]) return;
    const next = structuredClone(mixer);
    const channel = next.buses[row].channels[col];
    Object.assign(channel, patch);
    if ("fader_db" in patch || "pan" in patch) {
      channel.compensation_db = 0;
      channel.matrix_overrides = [null, null];
    }
    const changes = writeMatrix && next.buses[row].master_db !== null ? projectBus(next, row) : [];
    const effective = changes.filter((change) => matrix.gains[change.row]?.[change.col] !== change.db);
    publish({ ...snapshot, mixer: next, matrix: applyChanges(matrix, effective) });
    queueChanges(effective);
  },

  setBusStereo(row: number, stereo: boolean): void {
    const { mixer, matrix } = snapshot;
    if (!mixer || !matrix || !mixer.buses[row + 1] || mixer.buses[row - 1]?.stereo) return;
    if (stereo && mixer.buses[row + 1].stereo) return;
    const next = structuredClone(mixer);
    next.buses[row].stereo = stereo;
    if (stereo) {
      for (const channel of next.buses[row].channels) {
        if (channel.fader_db !== null && channel.fader_db > 6) {
          channel.compensation_db += channel.fader_db - 6;
          channel.fader_db = 6;
        }
      }
    }
    if (!stereo) next.buses[row + 1].label = null;
    const changes = projectBus(next, row);
    if (!stereo) changes.push(...projectBus(next, row + 1));
    const effective = changes.filter((change) => matrix.gains[change.row]?.[change.col] !== change.db);
    publish({ ...snapshot, mixer: next, matrix: applyChanges(matrix, effective) });
    queueChanges(effective);
  },

  setChannelStereo(row: number, col: number, stereo: boolean): void {
    const { mixer, matrix } = snapshot;
    const bus = mixer?.buses[row];
    if (!mixer || !matrix || !bus?.channels[col + 1] || bus.channels[col - 1]?.stereo) return;
    if (stereo && bus.channels[col + 1].stereo) return;
    const next = structuredClone(mixer);
    next.buses[row].channels[col].stereo = stereo;
    if (!stereo) next.buses[row].channels[col + 1].label = null;
    const changes = next.buses[row].master_db === null ? [] : projectBus(next, row);
    const effective = changes.filter((change) => matrix.gains[change.row]?.[change.col] !== change.db);
    publish({ ...snapshot, mixer: next, matrix: applyChanges(matrix, effective) });
    queueChanges(effective);
  },

  previewMatrixCell(row: number, col: number, db: number | null): void {
    const { matrix, mixer } = snapshot;
    if (!matrix || !mixer) return;
    const result = editMatrixGain(mixer, matrix, row, col, db);
    for (const change of result.changes) previewWrites.set(`${change.row}:${change.col}`, change);
    publish({ ...snapshot, mixer: result.mixer, matrix: applyChanges(matrix, result.changes) });
    stageChanges(result.changes);
  },

  commitMatrixEdits(): void {
    const changes = [...previewWrites.values()];
    previewWrites.clear();
    queueChanges(changes);
  },

  persist(): void {
    stageChanges([]);
    saveLater();
  },

  setCell(row: number, col: number, db: number | null): void {
    const current = snapshot.matrix;
    if (!current || row < 0 || row >= current.rows || col < 0 || col >= current.cols) return;
    const gains = current.gains.map((line) => [...line]);
    gains[row][col] = db !== null && db <= -60 ? null : db;
    const matrix = { ...current, gains };
    publish({ ...snapshot, matrix, busStrips: projectBusStrips(matrix, snapshot.busStrips) });
  },

  setRouterState(routerState: RouterState): void {
    portLabels.set(routerState.node, {
      rows: routerState.matrix_row_labels,
      columns: routerState.matrix_column_labels,
    });

    const current = snapshot.matrix;
    const matrix = current?.node === routerState.node
      && routerState.matrix_row_labels.length === current.rows
      && routerState.matrix_column_labels.length === current.cols
      ? {
          ...current,
          row_labels: routerState.matrix_row_labels,
          column_labels: routerState.matrix_column_labels,
        }
      : current;
    publish({ ...snapshot, matrix, routerState });
  },

  setMeters(node: string, inputPeaks: (number | null)[], destinationPeaks: DestinationPeak[], force = false): void {
    const current = snapshot.matrix;
    const router = snapshot.routerState;
    const updatesMatrix = current?.node === node;
    const updatesRouter = router?.node === node;
    if (!updatesMatrix && !updatesRouter) return;

    const matrixUnchanged = !updatesMatrix || (inputPeaks.length === current.input_peaks_db.length
      && inputPeaks.every((peak, index) => peak === current.input_peaks_db[index]));
    const peakByPort = new Map(destinationPeaks.map((peak) => [peak.port_id, peak.peak_db]));
    const routerUnchanged = !updatesRouter || router.destinations.every((destination) =>
      (peakByPort.get(destination.port.id) ?? null) === destination.peak_db,
    );
    if (matrixUnchanged && routerUnchanged) return;
    if (!force && Date.now() - lastPeakUiUpdate < MIN_PEAK_UI_INTERVAL_MS) return;

    lastPeakUiUpdate = Date.now();
    const nextMatrix = updatesMatrix ? { ...current, input_peaks_db: inputPeaks } : current;
    const nextRouter = updatesRouter
      ? {
          ...router,
          destinations: router.destinations.map((destination) => ({
            ...destination,
            peak_db: peakByPort.get(destination.port.id) ?? null,
          })),
        }
      : router;
    publish({ ...snapshot, matrix: nextMatrix, routerState: nextRouter });
  },

  setSolo(pair: number, solo: boolean): void {
    publish({
      ...snapshot,
      busStrips: snapshot.busStrips.map((strip) =>
        strip.pair === pair ? { ...strip, solo } : strip,
      ),
    });
  },
};

export function useMatrixStore(): MatrixSnapshot {
  return useSyncExternalStore(matrixStore.subscribe, matrixStore.getSnapshot, matrixStore.getSnapshot);
}