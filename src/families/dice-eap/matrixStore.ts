import { invoke } from "@tauri-apps/api/core";
import { useSyncExternalStore } from "react";
import type { BusStripProjection, DestinationPeak, MatrixState, MixerBus, MixerChannel, MixerState, RouterRoutesSnapshot, RouterState } from "../../types";
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
const routerStateListeners = new Set<() => void>();
const destinationRouteValues = new Map<string, string | null>();
const destinationRouteListeners = new Map<string, Set<() => void>>();
const routerRouteCountListeners = new Set<() => void>();
let routerRouteCount = 0;
const inputPeakValues = new Map<number, number | null>();
const destinationPeakValues = new Map<string, number | null>();
const destinationSignalValues = new Map<string, boolean>();
const meterListeners = new Set<() => void>();
let meterRevision = 0;
let meterNode: string | null = null;
const inputPeakListeners = new Map<number, Set<() => void>>();
const destinationSignalListeners = new Map<string, Set<() => void>>();

function subscribeTo<K>(subscriptions: Map<K, Set<() => void>>, key: K, listener: () => void): () => void {
  let keyListeners = subscriptions.get(key);
  if (!keyListeners) {
    keyListeners = new Set();
    subscriptions.set(key, keyListeners);
  }
  keyListeners.add(listener);
  return () => {
    keyListeners?.delete(listener);
    if (keyListeners?.size === 0) subscriptions.delete(key);
  };
}

function notify<K>(subscriptions: Map<K, Set<() => void>>, key: K) {
  subscriptions.get(key)?.forEach((listener) => listener());
}
const portLabels = new Map<string, { rows: string[]; columns: string[] }>();
const MIN_PEAK_UI_INTERVAL_MS = 100;
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

  getRouterState(): RouterState | null {
    return snapshot.routerState;
  },

  subscribeRouterState(listener: () => void): () => void {
    routerStateListeners.add(listener);
    return () => routerStateListeners.delete(listener);
  },

  getGain(row: number, col: number): number | null {
    return snapshot.matrix?.gains[row]?.[col] ?? null;
  },

  getInputPeak(col: number): number | null {
    return inputPeakValues.get(col) ?? null;
  },

  getMeterRevision(): number {
    return meterRevision;
  },

  subscribeMeters(listener: () => void): () => void {
    meterListeners.add(listener);
    return () => meterListeners.delete(listener);
  },

  subscribeInputPeak(col: number, listener: () => void): () => void {
    return subscribeTo(inputPeakListeners, col, listener);
  },

  getDestinationPeak(portId: string): number | null {
    return destinationPeakValues.get(portId) ?? null;
  },

  getDestinationRoute(portId: string): string | null {
    return destinationRouteValues.get(portId) ?? null;
  },

  subscribeDestinationRoute(portId: string, listener: () => void): () => void {
    return subscribeTo(destinationRouteListeners, portId, listener);
  },

  getRouterRouteCount(): number {
    return routerRouteCount;
  },

  subscribeRouterRouteCount(listener: () => void): () => void {
    routerRouteCountListeners.add(listener);
    return () => routerRouteCountListeners.delete(listener);
  },

  getDestinationSignal(portId: string): boolean {
    return destinationSignalValues.get(portId) ?? false;
  },

  subscribeDestinationSignal(portId: string, listener: () => void): () => void {
    return subscribeTo(destinationSignalListeners, portId, listener);
  },

  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },

  setMatrix(matrix: MatrixState): void {
    if (meterNode !== matrix.node) {
      meterNode = matrix.node;
      inputPeakValues.clear();
      destinationPeakValues.clear();
      destinationSignalValues.clear();
    }
    matrix.input_peaks_db.forEach((peak, col) => {
      if (!inputPeakValues.has(col)) inputPeakValues.set(col, peak);
    });
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

  setRouterRoutes(node: string, update: RouterRoutesSnapshot): boolean {
    const routerState = snapshot.routerState;
    if (!routerState || routerState.node !== node) return false;
    if (routerState.sample_rate !== update.sample_rate) return true;
    let routeCountChanged = false;
    for (const route of update.routes) {
      if (!destinationRouteValues.has(route.destination_id)) continue;
      const previousSource = destinationRouteValues.get(route.destination_id);
      if (previousSource === route.source_id) continue;
      if (previousSource !== null && previousSource !== undefined) routerRouteCount -= 1;
      if (route.source_id !== null) routerRouteCount += 1;
      destinationRouteValues.set(route.destination_id, route.source_id);
      const destination = routerState.destinations.find((item) => item.port.id === route.destination_id);
      if (destination) destination.source_id = route.source_id;
      notify(destinationRouteListeners, route.destination_id);
      routeCountChanged = true;
    }
    if (routeCountChanged) routerRouteCountListeners.forEach((listener) => listener());
    return false;
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
    const previous = snapshot.routerState;
    const sameTopology = previous !== null
      && previous.node === routerState.node
      && previous.sources.length === routerState.sources.length
      && previous.destinations.length === routerState.destinations.length
      && previous.sources.every((port, index) => {
        const next = routerState.sources[index];
        return port.id === next.id && port.label === next.label && port.group === next.group
          && port.stereo_partner_id === next.stereo_partner_id;
      })
      && previous.destinations.every((destination, index) => {
        const next = routerState.destinations[index];
        return destination.port.id === next.port.id
          && destination.port.label === next.port.label
          && destination.port.group === next.port.group
          && destination.port.stereo_partner_id === next.port.stereo_partner_id;
      })
      && previous.matrix_row_labels.length === routerState.matrix_row_labels.length
      && previous.matrix_row_labels.every((label, index) => label === routerState.matrix_row_labels[index])
      && previous.matrix_column_labels.length === routerState.matrix_column_labels.length
      && previous.matrix_column_labels.every((label, index) => label === routerState.matrix_column_labels[index])
      && previous.matrix_input_port_ids.length === routerState.matrix_input_port_ids.length
      && previous.matrix_input_port_ids.every((id, index) => id === routerState.matrix_input_port_ids[index])
      && previous.matrix_output_port_ids.length === routerState.matrix_output_port_ids.length
      && previous.matrix_output_port_ids.every((id, index) => id === routerState.matrix_output_port_ids[index]);

    if (!sameTopology) {
      destinationRouteValues.clear();
      routerRouteCount = 0;
    }
    let routeCountChanged = !sameTopology;
    for (const destination of routerState.destinations) {
      const previousSource = destinationRouteValues.get(destination.port.id);
      if (previousSource !== destination.source_id) {
        if (previousSource !== undefined && previousSource !== null) routerRouteCount -= 1;
        if (destination.source_id !== null) routerRouteCount += 1;
        destinationRouteValues.set(destination.port.id, destination.source_id);
        notify(destinationRouteListeners, destination.port.id);
        routeCountChanged = true;
      }
    }
    if (routeCountChanged) routerRouteCountListeners.forEach((listener) => listener());

    if (sameTopology && previous) {
      // Update only route values; keep the topology arrays and objects stable.
      routerState.destinations.forEach((destination, index) => {
        previous.destinations[index].source_id = destination.source_id;
      });
      return;
    }

    publish({ ...snapshot, matrix, routerState });
    routerStateListeners.forEach((listener) => listener());
  },

  setMeters(node: string, inputPeaks: (number | null)[], destinationPeaks: DestinationPeak[], force = false): void {
    const updatesMatrix = snapshot.matrix?.node === node;
    const updatesRouter = snapshot.routerState?.node === node;
    if (!updatesMatrix && !updatesRouter) return;
    if (!force && Date.now() - lastPeakUiUpdate < MIN_PEAK_UI_INTERVAL_MS) return;

    lastPeakUiUpdate = Date.now();
    let changed = false;

    if (updatesMatrix) {
      inputPeaks.forEach((peak, col) => {
        if (inputPeakValues.get(col) !== peak) {
          inputPeakValues.set(col, peak);
          notify(inputPeakListeners, col);
          changed = true;
        }
      });
    }

    if (updatesRouter) {
      destinationPeaks.forEach(({ port_id, peak_db }) => {
        if (destinationPeakValues.get(port_id) !== peak_db) {
          destinationPeakValues.set(port_id, peak_db);
          changed = true;
        }
        const hasSignal = peak_db !== null;
        if (destinationSignalValues.get(port_id) !== hasSignal) {
          destinationSignalValues.set(port_id, hasSignal);
          notify(destinationSignalListeners, port_id);
        }
      });
    }

    if (changed) {
      meterRevision += 1;
      meterListeners.forEach((listener) => listener());
    }
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