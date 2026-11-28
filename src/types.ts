export type DeviceFamily = "dice_eap" | "bebob";

export interface DeviceInfo {
  node_path: string;
  guid: string | null;
  family: DeviceFamily | null;
  vendor_id: number | null;
  model_id: number | null;
  vendor_name: string | null;
  model_name: string | null;
  nickname: string | null;
  name: string;
  protocol: string | null;
  supported: boolean;
  is_host_controller: boolean;
  scan_error: string | null;
}

export interface ClockSourceOption {
  value: string;
  label: string;
}

export interface DiceGlobalState {
  node: string;
  nickname: string;
  clock_sources: ClockSourceOption[];
  clock_source: string;
  sample_rates: number[];
  sample_rate: number | null;
  detected_sample_rate: number | null;
  clock_locked: boolean;
  stream_active: boolean;
}

export interface MatrixState {
  node: string;
  rows: number;
  cols: number;
  row_labels: string[];
  column_labels: string[];
  gains: (number | null)[][];
  input_peaks_db: (number | null)[];
}

export interface MeterUpdate {
  node: string;
  input_peaks_db: (number | null)[];
  destination_peaks: DestinationPeak[];
  error: string | null;
}

export interface DestinationPeak {
  port_id: string;
  peak_db: number | null;
}

export interface RouterPort {
  id: string;
  label: string;
  group: string;
  stereo_partner_id: string | null;
}

export interface RouterDestination {
  port: RouterPort;
  source_id: string | null;
}

export interface RouterRoute {
  destination_id: string;
  source_id: string | null;
}

export interface RouterRoutesSnapshot {
  sample_rate: number;
  routes: RouterRoute[];
}

export interface RouterState {
  node: string;
  sample_rate: number;
  sources: RouterPort[];
  destinations: RouterDestination[];
  matrix_row_labels: string[];
  matrix_column_labels: string[];
  matrix_input_port_ids: string[];
  matrix_output_port_ids: string[];
}

export interface BusStripProjection {
  pair: number;
  fader_db: number | null;
  pan: number;
  mute: boolean;
  solo: boolean;
}

export interface MixerChannel {
  label: string | null;
  stereo: boolean;
  fader_db: number | null;
  pan: number;
  muted: boolean;
  solo: boolean;
  pfm: boolean;
  compensation_db: number;
  matrix_overrides: [number | null, number | null];
}

export interface MixerBus {
  label: string | null;
  stereo: boolean;
  master_db: number | null;
  channels: MixerChannel[];
}

export interface MixerState {
  buses: MixerBus[];
}

export interface WorkspaceProps {
  device: DeviceInfo;
  node: string;
  globalState?: DiceGlobalState;
  onNicknameChange?: (name: string) => Promise<DiceGlobalState>;
  onClockSourceChange?: (source: string) => Promise<DiceGlobalState>;
  onSampleRateChange?: (rate: number) => Promise<DiceGlobalState>;
}