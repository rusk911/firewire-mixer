use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{matrix::{MatrixCellUpdate, MatrixStore}, router, DiceGlobalState};
use crate::families::DeviceFamily;
use crate::transport::discovery::{self, DeviceInfo};

const SNAPSHOT_VERSION: u32 = 1;

#[derive(Default)]
pub struct RestoreStatus {
	errors: Mutex<HashMap<String, String>>,
}

impl RestoreStatus {
	pub fn set(&self, node: &str, error: Option<String>) {
		let mut errors = self.errors.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
		if let Some(error) = error {
			errors.insert(node.to_owned(), error);
		} else {
			errors.remove(node);
		}
	}

	pub fn check(&self, node: &str) -> Result<(), String> {
		let errors = self.errors.lock().map_err(|_| "Restore status lock poisoned".to_owned())?;
		match errors.get(node) {
			Some(error) => Err(format!("Device configuration was not restored: {error}")),
			None => Ok(()),
		}
	}
}

#[derive(Clone)]
struct PendingMixer {
	revision: u64,
	mixer: MixerState,
	updates: Vec<MatrixCellUpdate>,
	dirty: bool,
	valid: bool,
}

#[derive(Default)]
pub struct MixerSessions {
	entries: Mutex<HashMap<String, PendingMixer>>,
	save_lock: Mutex<()>,
}

impl MixerSessions {
	pub fn stage(&self, node: &str, revision: u64, mixer: MixerState, updates: Vec<MatrixCellUpdate>) -> Result<(), String> {
		let mut entries = self.entries.lock().map_err(|_| "Mixer session lock poisoned".to_owned())?;
		if entries.get(node).is_some_and(|entry| entry.revision > revision) {
			return Ok(());
		}
		entries.insert(node.to_owned(), PendingMixer { revision, mixer, updates, dirty: true, valid: true });
		Ok(())
	}

	pub fn current(&self, node: &str) -> Option<(u64, Option<MixerState>)> {
		self.entries.lock().ok()?.get(node).map(|entry| (entry.revision, entry.valid.then(|| entry.mixer.clone())))
	}

	pub fn discard(&self, node: &str, revision: u64) -> Result<(), String> {
		let mut entries = self.entries.lock().map_err(|_| "Mixer session lock poisoned".to_owned())?;
		if entries.get(node).is_some_and(|entry| entry.revision > revision) {
			return Ok(());
		}
		entries.insert(node.to_owned(), PendingMixer {
			revision, mixer: MixerState { buses: Vec::new() }, updates: Vec::new(), dirty: false, valid: false,
		});
		Ok(())
	}

	pub fn forget(&self, node: &str) {
		if let Ok(mut entries) = self.entries.lock() {
			entries.remove(node);
		}
	}

	pub fn save(&self, node: &str, store: &MatrixStore, status: &RestoreStatus, revision: u64, mixer: MixerState) -> Result<(), String> {
		status.check(node)?;
		let _guard = self.save_lock.lock().map_err(|_| "Mixer save lock poisoned".to_owned())?;
		save_configuration(node, store, mixer)?;
		let mut entries = self.entries.lock().map_err(|_| "Mixer session lock poisoned".to_owned())?;
		if let Some(entry) = entries.get_mut(node)
			&& entry.revision <= revision {
				entry.dirty = false;
				entry.updates.clear();
			}
		Ok(())
	}

	pub fn flush(&self, node: &str, store: &MatrixStore, status: &RestoreStatus) -> Result<(), String> {
		status.check(node)?;
		let _guard = self.save_lock.lock().map_err(|_| "Mixer save lock poisoned".to_owned())?;
		loop {
			let pending = self.entries.lock().map_err(|_| "Mixer session lock poisoned".to_owned())?.get(node).cloned();
			let Some(pending) = pending.filter(|entry| entry.dirty) else { return Ok(()) };
			if !pending.updates.is_empty() {
				store.get_state(node)?;
				store.set_cells(node, &pending.updates)?;
			}
			save_configuration(node, store, pending.mixer)?;
			let mut entries = self.entries.lock().map_err(|_| "Mixer session lock poisoned".to_owned())?;
			if let Some(entry) = entries.get_mut(node)
				&& entry.revision == pending.revision {
					entry.dirty = false;
					entry.updates.clear();
					return Ok(());
				}
		}
	}

	pub fn flush_all(&self, store: &MatrixStore, status: &RestoreStatus) {
		let nodes: Vec<_> = self.entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner).keys().cloned().collect();
		for node in nodes {
			if let Err(error) = self.flush(&node, store, status) {
				eprintln!("Could not save mixer for {node} on exit: {error}");
			}
		}
	}
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MixerChannel {
	pub label: Option<String>,
	pub stereo: bool,
	pub fader_db: Option<f32>,
	pub pan: f32,
	pub muted: bool,
	pub solo: bool,
	pub pfm: bool,
	pub compensation_db: f32,
	#[serde(default)]
	pub matrix_overrides: [Option<f32>; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MixerBus {
	pub label: Option<String>,
	pub stereo: bool,
	pub master_db: Option<f32>,
	pub channels: Vec<MixerChannel>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MixerState {
	pub buses: Vec<MixerBus>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SavedSourceBlock {
	Aes, Adat, Mixer, Ins0, Ins1, ArmAprAudio, Avs0, Avs1, Mute, Reserved(u8),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SavedDestinationBlock {
	Aes, Adat, MixerTx0, MixerTx1, Ins0, Ins1, ArmApbAudio, Avs0, Avs1, Reserved(u8),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedSource {
	pub block: SavedSourceBlock,
	pub channel: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedDestination {
	pub block: SavedDestinationBlock,
	pub channel: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedRoute {
	pub source: SavedSource,
	pub destination: SavedDestination,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DeviceSnapshot {
	pub version: u32,
	pub guid: String,
	pub vendor_id: u32,
	pub model_id: u32,
	pub nickname: String,
	pub clock_source: String,
	pub sample_rate: Option<u32>,
	pub matrix: Vec<Vec<u16>>,
	pub routes: Vec<SavedRoute>,
	pub mixer: MixerState,
}

impl DeviceSnapshot {
	pub fn validate(&self, guid: &str, vendor_id: u32, model_id: u32) -> Result<(), String> {
		validate_guid(guid)?;
		if self.version != SNAPSHOT_VERSION || self.guid != guid
			|| self.vendor_id != vendor_id || self.model_id != model_id {
			return Err("Configuration does not match the connected device or supported version".to_owned());
		}
		if self.matrix.is_empty() || self.matrix[0].is_empty()
			|| self.matrix.iter().any(|row| row.len() != self.matrix[0].len())
			|| self.mixer.buses.is_empty() || self.mixer.buses.len() > self.matrix.len()
			|| self.mixer.buses.iter().any(|bus| bus.channels.is_empty()
				|| bus.channels.len() != self.mixer.buses[0].channels.len()
				|| bus.channels.len() > self.matrix[0].len()) {
			return Err("Configuration has inconsistent mixer dimensions".to_owned());
		}
		if self.routes.iter().any(|route| {
			route.source.channel > 15 || route.destination.channel > 15
				|| matches!(route.source.block, SavedSourceBlock::Reserved(value) if value > 15)
				|| matches!(route.destination.block, SavedDestinationBlock::Reserved(value) if value > 15)
		}) {
			return Err("Configuration contains invalid router blocks or channels".to_owned());
		}
		for (index, bus) in self.mixer.buses.iter().enumerate() {
			if bus.stereo && (index + 1 >= self.mixer.buses.len() || (index > 0 && self.mixer.buses[index - 1].stereo)) {
				return Err("Configuration contains overlapping stereo buses".to_owned());
			}
			if bus.master_db.is_some_and(|db| !db.is_finite() || !(-60.0..=0.0).contains(&db))
				|| bus.channels.iter().enumerate().any(|(channel_index, channel)| {
					!(-1.0..=1.0).contains(&channel.pan)
						|| channel.fader_db.is_some_and(|db| !db.is_finite() || !(-60.0..=12.0).contains(&db))
						|| !channel.compensation_db.is_finite()
						|| channel.matrix_overrides.iter().flatten().any(|db| !db.is_finite())
						|| (channel.stereo && (channel_index + 1 >= bus.channels.len()
							|| (channel_index > 0 && bus.channels[channel_index - 1].stereo)))
				}) {
				return Err("Configuration contains invalid mixer values".to_owned());
			}
		}
		Ok(())
	}
}

fn validate_guid(guid: &str) -> Result<(), String> {
	if guid.len() != 16 || !guid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
		return Err("Invalid device GUID".to_owned());
	}
	Ok(())
}

fn snapshot_path(directory: &Path, guid: &str) -> Result<PathBuf, String> {
	validate_guid(guid)?;
	Ok(directory.join(format!("{guid}.json")))
}

pub fn config_directory() -> Result<PathBuf, String> {
	dirs::config_dir().map(|directory| directory.join("firemix"))
		.ok_or_else(|| "Could not determine the configuration directory".to_owned())
}

pub fn load_snapshot(directory: &Path, guid: &str) -> Result<Option<DeviceSnapshot>, String> {
	let path = snapshot_path(directory, guid)?;
	match fs::read(&path) {
		Ok(data) => serde_json::from_slice(&data).map(Some)
			.map_err(|error| format!("Invalid configuration {}: {error}", path.display())),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(error) => Err(format!("Could not read configuration {}: {error}", path.display())),
	}
}

pub fn save_snapshot(directory: &Path, snapshot: &DeviceSnapshot) -> Result<(), String> {
	snapshot.validate(&snapshot.guid, snapshot.vendor_id, snapshot.model_id)?;
	let path = snapshot_path(directory, &snapshot.guid)?;
	let data = serde_json::to_vec_pretty(snapshot).map_err(|error| error.to_string())?;
	fs::create_dir_all(directory).map_err(|error| format!("Could not create configuration directory: {error}"))?;
	let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
	let temporary = directory.join(format!(".{}.{}.{}.tmp", snapshot.guid, std::process::id(), nonce));
	let result = (|| {
		let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
		file.write_all(&data)?;
		file.sync_all()?;
		fs::rename(&temporary, &path)
	})();
	if result.is_err() {
		let _ = fs::remove_file(&temporary);
	}
	result.map_err(|error| format!("Could not save configuration {}: {error}", path.display()))
}

fn dice_identity(node: &str) -> Result<(DeviceInfo, String, u32, u32), String> {
	let device = discovery::device_info_for_path(node)?;
	if device.family != Some(DeviceFamily::DiceEap) {
		return Err(format!("{node} is not a DICE EAP device"));
	}
	let guid = device.guid.clone().ok_or_else(|| format!("{node} has no valid hardware GUID"))?;
	let vendor = device.vendor_id.ok_or_else(|| "Device vendor ID is unavailable".to_owned())?;
	let model = device.model_id.ok_or_else(|| "Device model ID is unavailable".to_owned())?;
	Ok((device, guid, vendor, model))
}

pub fn capture_configuration(node: &str, store: &MatrixStore, mixer: MixerState) -> Result<DeviceSnapshot, String> {
	let (_, guid, vendor_id, model_id) = dice_identity(node)?;
	let global = super::get_global_state(node)?;
	let snapshot = DeviceSnapshot {
		version: SNAPSHOT_VERSION,
		guid,
		vendor_id,
		model_id,
		nickname: global.nickname,
		clock_source: global.clock_source,
		sample_rate: global.sample_rate,
		matrix: store.raw_coefficients(node)?,
		routes: router::raw_routes(node)?,
		mixer,
	};
	snapshot.validate(&snapshot.guid, vendor_id, model_id)?;
	Ok(snapshot)
}

pub fn save_configuration(node: &str, store: &MatrixStore, mixer: MixerState) -> Result<(), String> {
	let snapshot = capture_configuration(node, store, mixer)?;
	save_snapshot(&config_directory()?, &snapshot)
}

pub fn read_configuration(node: &str) -> Result<Option<DeviceSnapshot>, String> {
	let (_, guid, vendor, model) = dice_identity(node)?;
	let snapshot = load_snapshot(&config_directory()?, &guid)?;
	if let Some(snapshot) = &snapshot {
		snapshot.validate(&guid, vendor, model)?;
	}
	Ok(snapshot)
}

// DICE routers use a rate mode rather than an exact sample rate. Rates within
// the same mode (for example 44.1/48 kHz or 88.2/96 kHz) have compatible
// routing layouts and can be restored while an audio stream is active.
fn sample_rate_mode(rate: u32) -> Option<u8> {
	match rate {
		44_100 | 48_000 => Some(1),
		88_200 | 96_000 => Some(2),
		_ => None,
	}
}

fn sample_rates_compatible(current: u32, saved: u32) -> bool {
	current == saved || sample_rate_mode(current).is_some_and(|mode| sample_rate_mode(saved) == Some(mode))
}

fn stream_change_plan(global: &DiceGlobalState, snapshot: &DeviceSnapshot) -> Result<(bool, bool), String> {
	let change_rate = match snapshot.sample_rate {
		Some(saved_rate) if global.stream_active => {
			// current_rate is the rate actually detected on the active stream;
			// clock_config.rate may merely be the device's configured rate.
			let active_rate = global.detected_sample_rate.or(global.sample_rate);
			if let Some(current_rate) = active_rate {
				if !sample_rates_compatible(current_rate, saved_rate) {
					return Err(format!(
						"Cannot restore configuration: saved sample rate {saved_rate} Hz is incompatible with active stream rate {current_rate} Hz"
					));
				}
			}
			// Do not try to change the device clock while streaming. The rate
			// mode is compatible, so mixer/matrix/router restoration can proceed.
			false
		},
		Some(saved_rate) => super::stream_safe_change(global.sample_rate, Some(saved_rate), false, "sample rate")?,
		None => false,
	};
	let change_source = super::stream_safe_change(&global.clock_source, &snapshot.clock_source, global.stream_active, "clock source")?;
	Ok((change_rate, change_source))
}

pub fn restore_configuration(node: &str, store: &MatrixStore) -> Result<Option<MixerState>, String> {
	let Some(snapshot) = read_configuration(node)? else { return Ok(None) };
	let global = super::get_global_state(node)?;
	if !global.clock_sources.iter().any(|source| source.value == snapshot.clock_source) {
		return Err("Saved clock source is unavailable on the device".to_owned());
	}
	if snapshot.sample_rate.is_some_and(|rate| !global.sample_rates.contains(&rate)) {
		return Err("Saved sample rate is unavailable on the device".to_owned());
	}
	let (change_rate, change_source) = stream_change_plan(&global, &snapshot)?;
	let current = store.raw_coefficients(node)?;
	if current.len() != snapshot.matrix.len()
		|| current.iter().zip(&snapshot.matrix).any(|(actual, saved)| actual.len() != saved.len()) {
		return Err("Saved matrix dimensions do not match the device".to_owned());
	}
	router::validate_restorable_routes(node, &snapshot.routes, snapshot.sample_rate)?;
	if let Some(rate) = snapshot.sample_rate.filter(|_| change_rate) {
		super::set_sample_rate(node, rate).map_err(|error| format!("Could not restore sample rate: {error}"))?;
	}
	if change_source {
		super::set_clock_source(node, &snapshot.clock_source).map_err(|error| format!("Could not restore clock source: {error}"))?;
	}
	super::set_nickname(node, &snapshot.nickname).map_err(|error| format!("Could not restore nickname: {error}"))?;
	store.invalidate(node);
	router::restore_routes(node, &snapshot.routes, snapshot.sample_rate)?;
	store.restore_coefficients(node, &snapshot.matrix)?;
	Ok(Some(snapshot.mixer))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn snapshot() -> DeviceSnapshot {
		DeviceSnapshot {
			version: SNAPSHOT_VERSION,
			guid: "00130e0401408671".to_owned(),
			vendor_id: 0x00130e,
			model_id: 5,
			nickname: "Mixer".to_owned(),
			clock_source: "Internal".to_owned(),
			sample_rate: Some(48000),
			matrix: vec![vec![0, 16384]],
			routes: vec![SavedRoute {
				source: SavedSource { block: SavedSourceBlock::Mute, channel: 0 },
				destination: SavedDestination { block: SavedDestinationBlock::MixerTx0, channel: 0 },
			}],
			mixer: MixerState { buses: vec![MixerBus {
				label: None, stereo: false, master_db: Some(0.0),
				channels: vec![MixerChannel {
					label: None, stereo: false, fader_db: None, pan: 0.0,
					muted: false, solo: false, pfm: false, compensation_db: 0.0, matrix_overrides: [None, None],
				}; 2],
			}] },
		}
	}

	#[test]
	fn restore_clock_plan_skips_matching_values_and_blocks_changes_during_streaming() {
		let saved = snapshot();
		let mut global = DiceGlobalState {
			node: "/dev/fw1".to_owned(), nickname: "Mixer".to_owned(), clock_sources: vec![],
			clock_source: "Internal".to_owned(), sample_rates: vec![], sample_rate: Some(48000),
			detected_sample_rate: Some(48000), clock_locked: true, stream_active: true,
		};
		assert_eq!(stream_change_plan(&global, &saved), Ok((false, false)));
		global.sample_rate = Some(96000);
		global.detected_sample_rate = Some(96000);
		assert!(stream_change_plan(&global, &saved).expect_err("active rate change").contains("sample rate"));
		global.sample_rate = Some(48000);
		global.detected_sample_rate = Some(48000);
		global.clock_source = "Adat".to_owned();
		assert!(stream_change_plan(&global, &saved).expect_err("active source change").contains("clock source"));
		global.stream_active = false;
		global.sample_rate = Some(44100);
		assert_eq!(stream_change_plan(&global, &saved), Ok((true, true)));
	}

	#[test]
	fn active_stream_allows_sample_rates_within_the_same_dice_rate_mode() {
		let mut saved = snapshot(); // saved rate: 48 kHz
		let mut global = DiceGlobalState {
			node: "/dev/fw1".to_owned(), nickname: "Mixer".to_owned(), clock_sources: vec![],
			clock_source: "Internal".to_owned(), sample_rates: vec![], sample_rate: Some(48_000),
			detected_sample_rate: Some(44_100), clock_locked: true, stream_active: true,
		};
		assert_eq!(stream_change_plan(&global, &saved), Ok((false, false)));
		global.detected_sample_rate = Some(96_000);
		assert!(stream_change_plan(&global, &saved).unwrap_err().contains("incompatible"));

		saved.sample_rate = Some(96_000);
		global.detected_sample_rate = Some(88_200);
		assert_eq!(stream_change_plan(&global, &saved), Ok((false, false)));
		global.detected_sample_rate = Some(48_000);
		assert!(stream_change_plan(&global, &saved).unwrap_err().contains("incompatible"));
	}

	#[test]
	fn active_stream_uses_detected_rate_instead_of_configured_rate() {
		let saved = snapshot(); // saved rate: 48 kHz
		let global = DiceGlobalState {
			node: "/dev/fw1".to_owned(), nickname: "Mixer".to_owned(), clock_sources: vec![],
			clock_source: "Internal".to_owned(), sample_rates: vec![], sample_rate: Some(96_000),
			detected_sample_rate: Some(44_100), clock_locked: true, stream_active: true,
		};
		assert_eq!(stream_change_plan(&global, &saved), Ok((false, false)));
	}

	#[test]
	fn rejects_wrong_device_and_invalid_dimensions() {
		let mut saved = snapshot();
		assert!(saved.validate(&saved.guid, saved.vendor_id, 7).is_err());
		assert!(saved.validate("../invalid-path", saved.vendor_id, saved.model_id).is_err());
		saved.matrix.push(vec![0]);
		assert!(saved.validate(&saved.guid, saved.vendor_id, saved.model_id).is_err());
	}

	#[test]
	fn rejects_router_values_that_would_be_truncated() {
		let mut saved = snapshot();
		saved.routes[0].destination.channel = 16;
		assert!(saved.validate(&saved.guid, saved.vendor_id, saved.model_id).is_err());
	}

	#[test]
	fn snapshot_round_trips_without_losing_raw_coefficients() {
		let directory = std::env::temp_dir().join(format!("firemix-test-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos()));
		let saved = snapshot();
		assert_eq!(load_snapshot(&directory, &saved.guid).expect("missing file"), None);
		save_snapshot(&directory, &saved).expect("save snapshot");
		assert_eq!(load_snapshot(&directory, &saved.guid).expect("load snapshot"), Some(saved));
		fs::remove_dir_all(directory).expect("remove test directory");
	}

	#[test]
	fn session_ignores_out_of_order_mixer_updates() {
		let sessions = MixerSessions::default();
		let mut newer = snapshot().mixer;
		newer.buses[0].channels[0].fader_db = Some(-12.0);
		let older = snapshot().mixer;
		sessions.stage("/dev/fw1", 2, newer.clone(), vec![MatrixCellUpdate { row: 0, col: 0, db: Some(-12.0) }]).expect("stage newer");
		sessions.stage("/dev/fw1", 1, older, vec![]).expect("stage older");
		assert_eq!(sessions.current("/dev/fw1"), Some((2, Some(newer))));
		let entries = sessions.entries.lock().expect("session lock");
		assert_eq!(entries["/dev/fw1"].updates.len(), 1);
	}

	#[test]
	fn discarded_session_preserves_revision_and_rejects_late_updates() {
		let sessions = MixerSessions::default();
		sessions.discard("/dev/fw1", 5).expect("discard");
		sessions.stage("/dev/fw1", 4, snapshot().mixer, vec![]).expect("late update");
		assert_eq!(sessions.current("/dev/fw1"), Some((5, None)));
		sessions.stage("/dev/fw1", 6, snapshot().mixer, vec![]).expect("new update");
		assert!(sessions.current("/dev/fw1").expect("current").1.is_some());
	}

	#[test]
	#[ignore = "requires a DICE device specified by FIREMIX_TEST_NODE"]
	fn reads_full_configuration_without_hardware_writes() {
		let node = std::env::var("FIREMIX_TEST_NODE").expect("FIREMIX_TEST_NODE must name a device");
		let store = MatrixStore::default();
		let matrix = store.get_state(&node).expect("matrix state");
		let mixer = MixerState { buses: matrix.gains.iter().map(|row| MixerBus {
			label: None, stereo: false, master_db: Some(0.0),
			channels: row.iter().map(|gain| MixerChannel {
				label: None, stereo: false, fader_db: gain.map(|db| db.clamp(-59.9, 12.0)), pan: 0.0,
				muted: false, solo: false, pfm: false, compensation_db: 0.0, matrix_overrides: [None, None],
			}).collect(),
		}).collect() };
		let saved = capture_configuration(&node, &store, mixer).expect("capture snapshot");
		router::validate_restorable_routes(&node, &saved.routes, saved.sample_rate).expect("validate saved routes");
		assert_eq!(saved.mixer.buses.len(), matrix.rows);
		assert!(!saved.routes.is_empty());
		let directory = std::env::temp_dir().join(format!("firemix-hardware-test-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_nanos()));
		save_snapshot(&directory, &saved).expect("save snapshot");
		assert_eq!(load_snapshot(&directory, &saved.guid).expect("load snapshot"), Some(saved.clone()));
		fs::remove_dir_all(directory).expect("remove test directory");
		println!("GUID {}, {} matrix rows, {} router entries", saved.guid, saved.matrix.len(), saved.routes.len());
	}

	#[test]
	#[ignore = "requires a DICE device and saved snapshot specified by FIREMIX_TEST_NODE"]
	fn reads_saved_clock_state_without_hardware_writes() {
		let node = std::env::var("FIREMIX_TEST_NODE").expect("FIREMIX_TEST_NODE must name a device");
		let saved = read_configuration(&node).expect("read saved configuration").expect("saved snapshot");
		let global = super::super::get_global_state(&node).expect("read current global state");
		let rate_change = saved.sample_rate.map(|rate| super::super::stream_safe_change(global.sample_rate, Some(rate), global.stream_active, "sample rate"));
		let source_change = super::super::stream_safe_change(&global.clock_source, &saved.clock_source, global.stream_active, "clock source");
		println!("stream_active={}, sample_rate_matches={}, clock_source_matches={}", global.stream_active, global.sample_rate == saved.sample_rate, global.clock_source == saved.clock_source);
		if global.stream_active {
			assert!(!matches!(rate_change, Some(Ok(true))));
			assert_ne!(source_change, Ok(true));
		}
	}
}