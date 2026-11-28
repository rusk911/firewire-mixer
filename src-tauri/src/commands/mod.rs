use crate::families::dice_eap::{self, DiceGlobalState};
use crate::families::dice_eap::matrix::{MatrixCellUpdate, MatrixState, MatrixStore};
use crate::families::dice_eap::mixer::{self, MixerSessions, MixerState, RestoreStatus};
use crate::families::dice_eap::router::{self, RouterState};
use crate::transport::discovery::{self, DeviceInfo};
use std::sync::Arc;
use tauri::State;

#[derive(serde::Serialize)]
pub struct SavedMixerState {
    mixer: Option<MixerState>,
    revision: u64,
}

#[tauri::command]
pub fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    discovery::list_devices()
}

#[tauri::command]
pub fn get_dice_global_state(node: String) -> Result<DiceGlobalState, String> {
    dice_eap::get_global_state(&node)
}

#[tauri::command]
pub fn set_dice_nickname(node: String, name: String) -> Result<DiceGlobalState, String> {
    dice_eap::set_nickname(&node, &name)
}

#[tauri::command]
pub fn set_dice_clock_source(node: String, source: String) -> Result<DiceGlobalState, String> {
    dice_eap::set_clock_source(&node, &source)
}

#[tauri::command]
pub fn set_dice_sample_rate(node: String, rate: u32) -> Result<DiceGlobalState, String> {
    dice_eap::set_sample_rate(&node, rate)
}

#[tauri::command]
pub async fn get_matrix_state(
    matrix_store: State<'_, Arc<MatrixStore>>,
    node: String,
) -> Result<MatrixState, String> {
    let matrix_store = matrix_store.inner().clone();
    let loader = Arc::clone(&matrix_store);
    let matrix_node = node.clone();
    let state = tauri::async_runtime::spawn_blocking(move || loader.get_state(&matrix_node))
        .await
        .map_err(|error| format!("Matrix load task failed: {error}"))??;
    Ok(state)
}

#[tauri::command]
pub async fn set_matrix_cell(
    matrix_store: State<'_, Arc<MatrixStore>>,
    node: String,
    row: usize,
    col: usize,
    db: f32,
) -> Result<MatrixState, String> {
    let matrix_store = matrix_store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || matrix_store.set_cell(&node, row, col, db))
        .await
        .map_err(|error| format!("Matrix update task failed: {error}"))?
}

#[tauri::command]
pub async fn set_matrix_cells(
    matrix_store: State<'_, Arc<MatrixStore>>,
    node: String,
    updates: Vec<MatrixCellUpdate>,
) -> Result<MatrixState, String> {
    let matrix_store = matrix_store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || matrix_store.set_cells(&node, &updates))
        .await
        .map_err(|error| format!("Matrix update task failed: {error}"))?
}

#[tauri::command]
pub async fn start_meter_polling(
    matrix_store: State<'_, Arc<MatrixStore>>,
    node: String,
) -> Result<(), String> {
    let matrix_store = matrix_store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        matrix_store.get_state(&node)?;
        matrix_store.start_peak_polling(&node)
    })
    .await
    .map_err(|error| format!("Meter start task failed: {error}"))?
}

#[tauri::command]
pub fn stop_meter_polling(matrix_store: State<'_, Arc<MatrixStore>>, node: String) {
    matrix_store.stop_peak_polling(&node);
}

#[tauri::command]
pub async fn get_router_state(node: String) -> Result<RouterState, String> {
    tauri::async_runtime::spawn_blocking(move || router::get_router_state(&node))
        .await
        .map_err(|error| format!("Router load task failed: {error}"))?
}

#[tauri::command]
pub async fn get_router_routes(
    router_lock: State<'_, Arc<tokio::sync::Mutex<()>>>,
    node: String,
) -> Result<router::RouterRoutesSnapshot, String> {
    let _guard = router_lock.lock().await;
    tauri::async_runtime::spawn_blocking(move || router::get_router_routes(&node))
        .await
        .map_err(|error| format!("Router route refresh task failed: {error}"))?
}

#[tauri::command]
pub async fn set_route(
    router_lock: State<'_, Arc<tokio::sync::Mutex<()>>>,
    node: String,
    destination_id: String,
    source_id: Option<String>,
) -> Result<RouterState, String> {
    let _guard = router_lock.lock().await;
    tauri::async_runtime::spawn_blocking(move || {
        router::set_route(&node, &destination_id, source_id.as_deref())
    })
    .await
    .map_err(|error| format!("Router update task failed: {error}"))?
}

#[tauri::command]
pub async fn get_saved_mixer_state(status: State<'_, Arc<RestoreStatus>>, sessions: State<'_, Arc<MixerSessions>>, node: String) -> Result<SavedMixerState, String> {
    status.check(&node)?;
    let (revision, current) = sessions.current(&node).unwrap_or((0, None));
    if current.is_some() {
        return Ok(SavedMixerState { mixer: current, revision });
    }
    tauri::async_runtime::spawn_blocking(move || {
        mixer::read_configuration(&node).map(|snapshot| SavedMixerState { mixer: snapshot.map(|snapshot| snapshot.mixer), revision })
    })
    .await
    .map_err(|error| format!("Mixer configuration load task failed: {error}"))?
}

#[tauri::command]
pub async fn save_mixer_configuration(
    matrix_store: State<'_, Arc<MatrixStore>>,
    status: State<'_, Arc<RestoreStatus>>,
    sessions: State<'_, Arc<MixerSessions>>,
    node: String,
    mixer_state: MixerState,
    revision: u64,
) -> Result<(), String> {
    status.check(&node)?;
    let matrix_store = matrix_store.inner().clone();
    let status = status.inner().clone();
    let sessions = sessions.inner().clone();
    tauri::async_runtime::spawn_blocking(move || sessions.save(&node, &matrix_store, &status, revision, mixer_state))
        .await
        .map_err(|error| format!("Mixer configuration save task failed: {error}"))?
}

#[tauri::command]
pub fn stage_mixer_configuration(
    status: State<'_, Arc<RestoreStatus>>,
    sessions: State<'_, Arc<MixerSessions>>,
    node: String,
    revision: u64,
    mixer_state: MixerState,
    updates: Vec<MatrixCellUpdate>,
) -> Result<(), String> {
    status.check(&node)?;
    sessions.stage(&node, revision, mixer_state, updates)
}

#[tauri::command]
pub fn discard_mixer_configuration(sessions: State<'_, Arc<MixerSessions>>, node: String, revision: u64) -> Result<(), String> {
    sessions.discard(&node, revision)
}
