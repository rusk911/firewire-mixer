import type { MatrixState, MixerBus, MixerChannel, MixerState } from "../../types";

export type Gain = number | null;

export interface MatrixChange {
  row: number;
  col: number;
  db: Gain;
}

export function panGain(pan: number, side: "left" | "right"): Gain {
  const position = Math.max(-1, Math.min(1, pan));
  const amplitude = side === "left" ? (1 - position) / 2 : (1 + position) / 2;
  return amplitude === 0 ? null : 20 * Math.log10(amplitude);
}

export function matrixGain(
  fader: Gain,
  master: Gain,
  muted: boolean,
  channelPan?: number,
  masterPan?: number,
  side?: "left" | "right",
): Gain {
  if (muted || fader === null || master === null) return null;
  let gain = fader + master;
  if (side !== undefined) {
    const channelOffset = panGain(channelPan ?? 0, side);
    const masterOffset = panGain(masterPan ?? 0, side);
    if (channelOffset === null || masterOffset === null) return null;
    gain += channelOffset + masterOffset;
  }
  return gain <= -60 ? null : Math.min(12, gain);
}

export function createMixerState(matrix: MatrixState): MixerState {
  return {
    buses: matrix.gains.map((row) => ({
      label: null,
      stereo: false,
      master_db: 0,
      master_pan: 0,
      channels: row.map((gain) => ({
        label: null,
        stereo: false,
        fader_db: gain === null ? null : Math.max(-59.9, Math.min(12, gain)),
        pan: 0,
        muted: false,
        solo: false,
        pfm: false,
        compensation_db: gain === null ? 0 : gain - Math.max(-59.9, Math.min(12, gain)),
        matrix_overrides: [null, null],
      })),
    })),
  };
}

export function busRoot(mixer: MixerState, row: number): number {
  return row > 0 && mixer.buses[row - 1]?.stereo ? row - 1 : row;
}

function logicalChannel(bus: MixerBus, col: number): MixerChannel {
  return col > 0 && bus.channels[col - 1]?.stereo ? bus.channels[col - 1] : bus.channels[col];
}

export function projectBus(mixer: MixerState, row: number): MatrixChange[] {
  const root = busRoot(mixer, row);
  const bus = mixer.buses[root];
  if (!bus) return [];
  const anySolo = bus.channels.some((_, index) => logicalChannel(bus, index).solo);
  const changes: MatrixChange[] = [];
  for (let col = 0; col < bus.channels.length; col += 1) {
    const channel = logicalChannel(bus, col);
    const muted = channel.muted || (anySolo && !channel.solo);
    const fader = channel.fader_db === null ? null : channel.fader_db + channel.compensation_db;
    const sides = bus.stereo && mixer.buses[root + 1] ? 2 : 1;
    for (let side = 0; side < sides; side += 1) {
      const direction = sides === 2 ? (side === 0 ? "left" : "right") : undefined;
      const calculated = matrixGain(fader, bus.master_db, muted, channel.pan, bus.master_pan, direction);
      const override = channel.matrix_overrides[side];
      const overridden = override !== null && bus.master_db !== null ? override + bus.master_db : null;
      const db = !muted && fader !== null && overridden !== null
        ? (overridden <= -60 ? null : Math.min(12, overridden)) : calculated;
      changes.push({ row: root + side, col, db });
    }
  }
  return changes;
}

export function editMatrixGain(
  mixer: MixerState,
  matrix: MatrixState,
  row: number,
  col: number,
  target: Gain,
): { mixer: MixerState; changes: MatrixChange[] } {
  if (target !== null && target <= -60) target = null;
  const next = structuredClone(mixer);
  const root = busRoot(next, row);
  const bus = next.buses[root];
  if (!bus || col < 0 || col >= bus.channels.length) return { mixer: next, changes: [] };
  const logicalCol = col > 0 && bus.channels[col - 1]?.stereo ? col - 1 : col;
  const channel = bus.channels[logicalCol];
  channel.muted = false;
  if (bus.channels.some((candidate) => candidate.solo) && !channel.solo) channel.solo = true;
  channel.matrix_overrides = [null, null];
  const stereo = bus.stereo && !!next.buses[root + 1];
  const side = row - root;
  const limit = stereo ? 6 : 12;
  let required: Gain = target;
  const hardPanValues: [Gain, Gain] = [null, null];

  if (stereo) {
    const values: [Gain, Gain] = [matrix.gains[root]?.[col] ?? null, matrix.gains[root + 1]?.[col] ?? null];
    values[side] = target;
    const offsets = [panGain(bus.master_pan, "left"), panGain(bus.master_pan, "right")];
    const amplitudes = values.map((value, index) => {
      const offset = offsets[index];
      if (value === null || offset === null) return 0;
      return 10 ** ((value - offset) / 20);
    });
    const total = amplitudes[0] + amplitudes[1];
    if (total === 0) {
      required = null;
    } else {
      channel.pan = (amplitudes[1] - amplitudes[0]) / total;
      required = 20 * Math.log10(total);
    }
    for (let index = 0; index < 2; index += 1) {
      if (offsets[index] === null && values[index] !== null) hardPanValues[index] = values[index];
    }
  }

  if (required === null) {
    channel.fader_db = null;
    channel.compensation_db = 0;
  } else {
    if (bus.master_db === null) bus.master_db = Math.min(0, required - limit);
    let needed = required - bus.master_db;
    if (needed > limit && bus.master_db < 0) {
      bus.master_db = Math.min(0, bus.master_db + needed - limit);
      needed = required - bus.master_db;
    }
    channel.fader_db = Math.max(-59.9, Math.min(limit, needed));
    channel.compensation_db = needed - channel.fader_db;
  }
  if (target !== null && stereo && panGain(bus.master_pan, side === 0 ? "left" : "right") === null) {
    if (bus.master_db === null) bus.master_db = 0;
    if (channel.fader_db === null) channel.fader_db = limit;
    hardPanValues[side] = target;
  }
  if (bus.master_db !== null) {
    for (let index = 0; index < hardPanValues.length; index += 1) {
      const value = hardPanValues[index];
      channel.matrix_overrides[index] = value === null ? null : value - bus.master_db;
    }
  }
  const changes = projectBus(next, root).filter((change) => {
    const previous = matrix.gains[change.row]?.[change.col] ?? null;
    return previous === null || change.db === null ? previous !== change.db : Math.abs(previous - change.db) > 0.0001;
  });
  return { mixer: next, changes };
}