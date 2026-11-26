import assert from "node:assert/strict";
import { test } from "node:test";
import { createMixerState, editMatrixGain, matrixGain, panGain, projectBus } from "../src/families/dice-eap/mixerMath.ts";

function matrix(rows = 2, cols = 2) {
  return {
    node: "test", rows, cols, row_labels: [], column_labels: [],
    gains: Array.from({ length: rows }, () => Array(cols).fill(null)),
    input_peaks_db: Array(cols).fill(null),
  };
}

test("pan follows the linear amplitude -6 dB law on channel only", () => {
  assert.equal(panGain(-1, "left"), 0);
  assert.equal(panGain(-1, "right"), null);
  assert.equal(panGain(1, "left"), null);
  assert.ok(Math.abs(panGain(0, "left") + 6.0206) < 0.001);
  assert.ok(Math.abs(matrixGain(0, 0, false, 0, "left") + 6.0206) < 0.001);
  assert.equal(matrixGain(-60, 0, false), null);
});

test("mute, solo and master silence affect only the selected bus", () => {
  const state = matrix();
  const mixer = createMixerState(state);
  const [bus0] = mixer.buses;
  const [ch0, ch1] = bus0.channels;
  ch0.fader_db = 0;
  ch1.fader_db = -12;
  ch0.solo = true;
  assert.deepEqual(projectBus(mixer, 0).map((cell) => cell.db), [0, null]);
  bus0.master_db = null;
  assert.deepEqual(projectBus(mixer, 0).map((cell) => cell.db), [null, null]);
  assert.equal(ch1.fader_db, -12);
});

test("matrix edit raises a muted master without moving sibling faders", () => {
  const state = matrix();
  const mixer = createMixerState(state);
  const [bus0] = mixer.buses;
  const [ch0, ch1] = bus0.channels;
  bus0.stereo = true;
  bus0.master_db = null;
  ch1.fader_db = -12;
  const result = editMatrixGain(mixer, state, 0, 0, 12);
  const [resBus0] = result.mixer.buses;
  const [resCh0, resCh1] = resBus0.channels;
  assert.equal(resCh0.fader_db, 6);
  assert.equal(resCh1.fader_db, -12);
  assert.equal(resBus0.master_db, 0);
  assert.ok(result.changes.some((cell) => cell.row === 0 && cell.col === 0 && Math.abs(cell.db - 12) < 0.001));
  assert.ok(result.changes.some((cell) => cell.col === 1));
  assert.equal(bus0.master_db, null);
});


test("matrix priority works when master pan makes one side silent", () => {
  const state = matrix();
  const mixer = createMixerState(state);
  mixer.buses[0].stereo = true;
  mixer.buses[0].master_db = 0;
  mixer.buses[0].channels[0].fader_db = 6;
  mixer.buses[0].channels[0].pan = 0;

  const centered = projectBus(mixer, 0);
  assert.ok(Math.abs(centered.find((c) => c.row === 0 && c.col === 0).db - (-0.0206)) < 0.001);
  assert.ok(Math.abs(centered.find((c) => c.row === 1 && c.col === 0).db - (-0.0206)) < 0.001);

  mixer.buses[0].channels[0].pan = -1;
  const hardLeft = projectBus(mixer, 0);
  assert.equal(hardLeft.find((c) => c.row === 0 && c.col === 0).db, 6);
  assert.equal(hardLeft.find((c) => c.row === 1 && c.col === 0).db, null);
});

test("linked channels share mute and solo without modifying the next bus", () => {
  const state = matrix(2, 3);
  const mixer = createMixerState(state);
  mixer.buses[0].channels[0].stereo = true;
  mixer.buses[0].channels[0].fader_db = 0;
  mixer.buses[0].channels[0].solo = true;
  mixer.buses[0].channels[2].fader_db = -6;
  assert.deepEqual(projectBus(mixer, 0).map((cell) => cell.db), [0, 0, null]);
  mixer.buses[0].channels[0].muted = true;
  assert.deepEqual(projectBus(mixer, 0).map((cell) => cell.db), [null, null, null]);
  assert.equal(mixer.buses[1].channels[0].muted, false);
});

test("initial mixer preserves hardware levels outside visible fader range", () => {
  const state = matrix(1, 2);
  state.gains[0] = [12.04, -64];
  const mixer = createMixerState(state);
  assert.equal(mixer.buses[0].channels[0].fader_db, 12);
  assert.ok(Math.abs(mixer.buses[0].channels[0].compensation_db - 0.04) < 0.0001);
  assert.equal(mixer.buses[0].channels[1].fader_db, -59.9);
  assert.ok(Math.abs(mixer.buses[0].channels[1].compensation_db + 4.1) < 0.0001);
});

test("stereo channel on stereo bus routes L->L and R->R only", () => {
  const state = matrix(2, 2);
  const mixer = createMixerState(state);
  const [bus0] = mixer.buses;
  const [ch0] = bus0.channels;

  bus0.stereo = true;
  bus0.master_db = 0;
  ch0.stereo = true;
  ch0.fader_db = 6;
  ch0.pan = 0;

  const centered = projectBus(mixer, 0);
  const cell = (r, c) => centered.find((item) => item.row === r && item.col === c).db;
  assert.ok(Math.abs(cell(0, 0) - (-0.0206)) < 0.001);
  assert.equal(cell(0, 1), null);
  assert.equal(cell(1, 0), null);
  assert.ok(Math.abs(cell(1, 1) - (-0.0206)) < 0.001);

  ch0.pan = -1;
  const hardLeft = projectBus(mixer, 0);
  const cellLeft = (r, c) => hardLeft.find((item) => item.row === r && item.col === c).db;
  assert.equal(cellLeft(0, 0), 6);
  assert.equal(cellLeft(0, 1), null);
  assert.equal(cellLeft(1, 0), null);
  assert.equal(cellLeft(1, 1), null);

  ch0.pan = 1;
  const hardRight = projectBus(mixer, 0);
  const cellRight = (r, c) => hardRight.find((item) => item.row === r && item.col === c).db;
  assert.equal(cellRight(0, 0), null);
  assert.equal(cellRight(0, 1), null);
  assert.equal(cellRight(1, 0), null);
  assert.equal(cellRight(1, 1), 6);
});
