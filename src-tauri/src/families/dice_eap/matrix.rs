use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use firewire_dice_protocols::focusrite::spro26::SPro26Protocol;
use firewire_dice_protocols::focusrite::spro40::SPro40Protocol;
use firewire_dice_protocols::maudio::Pfire2626Protocol;
use firewire_dice_protocols::presonus::fstudiomobile::FStudioMobileProtocol;
use firewire_dice_protocols::tcat::extension::{
    caps_section::TcatExtensionCapsSectionOperation,
    cmd_section::RateMode,
    current_config_section::CurrentRouterParams,
    mixer_section::MixerCoefficientParams,
    peak_section::PeakParams,
    router_section::RouterParams,
    DstBlk, DstBlkId, ExtensionCaps, ExtensionSections, SrcBlk, SrcBlkId,
    TcatExtensionOperation, TcatExtensionSectionParamsOperation,
    TcatExtensionSectionPartialMutableParamsOperation,
};
use firewire_dice_protocols::tcat::global_section::GlobalParameters;
use firewire_dice_protocols::tcat::tcd22xx_spec::Tcd22xxSpecification;
use serde::{Deserialize, Serialize};

use crate::families::DeviceFamily;
use crate::transport::dice::DiceTransport;
use crate::transport::discovery::{self, DeviceInfo};

const TRANSACTION_TIMEOUT_MS: u32 = 200;
const COEFFICIENT_UNITY: f64 = 16_384.0;
const MIN_EDITABLE_DB: f32 = -60.0;
const MAX_EDITABLE_DB: f32 = 12.0;
const METER_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct MatrixState {
    pub node: String,
    pub rows: usize,
    pub cols: usize,
    pub row_labels: Vec<String>,
    pub column_labels: Vec<String>,
    pub gains: Vec<Vec<Option<f32>>>,
    pub input_peaks_db: Vec<Option<f32>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MatrixCellUpdate {
    pub row: usize,
    pub col: usize,
    pub db: Option<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MeterUpdate {
    pub node: String,
    pub input_peaks_db: Vec<Option<f32>>,
    pub destination_peaks: Vec<DestinationPeak>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DestinationPeak {
    pub port_id: String,
    pub peak_db: Option<f32>,
}

#[derive(Default)]
pub struct MatrixStore {
    matrices: Mutex<HashMap<String, CachedMatrix>>,
    pollers: Mutex<HashMap<String, Arc<AtomicBool>>>,
    meter_emitter: Mutex<Option<Arc<dyn Fn(MeterUpdate) + Send + Sync>>>,
    shutting_down: AtomicBool,
}

#[derive(Clone)]
struct CachedMatrix {
    protocol: ProtocolKind,
    sections: ExtensionSections,
    caps: ExtensionCaps,
    mixer_inputs: Vec<DstBlk>,
    coefficients: MixerCoefficientParams,
    state: MatrixState,
}

#[derive(Clone, Copy)]
pub(super) enum ProtocolKind {
    SaffirePro26,
    SaffirePro40,
    ProFire2626,
    FireStudioMobile,
}

trait MatrixProtocol:
    TcatExtensionOperation
    + Tcd22xxSpecification
    + TcatExtensionCapsSectionOperation
    + TcatExtensionSectionParamsOperation<MixerCoefficientParams>
    + TcatExtensionSectionParamsOperation<RouterParams>
    + TcatExtensionSectionParamsOperation<CurrentRouterParams>
    + TcatExtensionSectionParamsOperation<PeakParams>
    + TcatExtensionSectionPartialMutableParamsOperation<MixerCoefficientParams>
{
}

impl<T> MatrixProtocol for T where
    T: TcatExtensionOperation
        + Tcd22xxSpecification
        + TcatExtensionCapsSectionOperation
        + TcatExtensionSectionParamsOperation<MixerCoefficientParams>
        + TcatExtensionSectionParamsOperation<RouterParams>
        + TcatExtensionSectionParamsOperation<CurrentRouterParams>
        + TcatExtensionSectionParamsOperation<PeakParams>
        + TcatExtensionSectionPartialMutableParamsOperation<MixerCoefficientParams>
{
}

impl ProtocolKind {
    pub(super) fn for_device(device: &DeviceInfo) -> Result<Self, String> {
        if device.family != Some(DeviceFamily::DiceEap) {
            return Err(format!("{} is not a DICE EAP device", device.node_path));
        }

        match (device.vendor_id, device.model_id) {
            (Some(0x00130e), Some(0x000012)) => Ok(Self::SaffirePro26),
            (Some(0x00130e), Some(0x000005 | 0x000007 | 0x0000de)) => {
                Ok(Self::SaffirePro40)
            }
            (Some(0x000d6c), Some(0x000010)) => Ok(Self::ProFire2626),
            (Some(0x000a92), Some(0x000011)) => Ok(Self::FireStudioMobile),
            _ => Err(format!(
                "No DICE matrix protocol profile for {} ({}, {})",
                device.name,
                format_id(device.vendor_id),
                format_id(device.model_id)
            )),
        }
    }
}

pub(super) fn device_protocol(device: &DeviceInfo) -> Result<ProtocolKind, String> {
    ProtocolKind::for_device(device)
}

impl MatrixStore {
    pub fn set_meter_emitter<F>(&self, emitter: F)
    where
        F: Fn(MeterUpdate) + Send + Sync + 'static,
    {
        if let Ok(mut slot) = self.meter_emitter.lock() {
            *slot = Some(Arc::new(emitter));
        }
    }

    pub fn get_state(&self, node_path: &str) -> Result<MatrixState, String> {
        if let Some(cached) = self
            .matrices
            .lock()
            .map_err(|_| "Matrix cache lock poisoned".to_owned())?
            .get(node_path)
            .cloned()
        {
            return Ok(cached.state);
        }

        let device = discovery::device_info_for_path(node_path)?;
        let protocol = ProtocolKind::for_device(&device)?;
        let transport = DiceTransport::open(node_path)?;
        let (_, global) = transport.read_global_section()?;
        let cached = read_matrix(protocol, &device, node_path, &transport, &global)?;
        let state = cached.state.clone();
        self.matrices
            .lock()
            .map_err(|_| "Matrix cache lock poisoned".to_owned())?
            .insert(node_path.to_owned(), cached);
        Ok(state)
    }

    pub fn set_cell(
        &self,
        node_path: &str,
        row: usize,
        col: usize,
        db: f32,
    ) -> Result<MatrixState, String> {
        let mut matrices = self
            .matrices
            .lock()
            .map_err(|_| "Matrix cache lock poisoned".to_owned())?;
        let cached = matrices
            .get_mut(node_path)
            .ok_or_else(|| format!("Matrix state for {node_path} has not been loaded"))?;

        if row >= cached.state.rows || col >= cached.state.cols {
            return Err(format!(
                "Matrix cell ({row}, {col}) is out of bounds for {}x{} matrix",
                cached.state.rows, cached.state.cols
            ));
        }
        let coefficient = db_to_coefficient(db)?;
        let mut next = cached.coefficients.clone();
        next.0[row][col] = coefficient;

        let transport = DiceTransport::open(node_path)?;
        write_coefficients(
            cached.protocol,
            &transport,
            &cached.sections,
            &cached.caps,
            &next,
            &mut cached.coefficients,
        )?;

        cached.state.gains[row][col] = coefficient_to_db(coefficient);
        Ok(cached.state.clone())
    }

    pub fn raw_coefficients(&self, node_path: &str) -> Result<Vec<Vec<u16>>, String> {
        self.get_state(node_path)?;
        let matrices = self.matrices.lock().map_err(|_| "Matrix cache lock poisoned".to_owned())?;
        matrices.get(node_path).map(|cached| cached.coefficients.0.clone())
            .ok_or_else(|| format!("Matrix state for {node_path} has not been loaded"))
    }

    pub fn set_cells(&self, node_path: &str, updates: &[MatrixCellUpdate]) -> Result<MatrixState, String> {
        let mut matrices = self.matrices.lock().map_err(|_| "Matrix cache lock poisoned".to_owned())?;
        let cached = matrices.get_mut(node_path)
            .ok_or_else(|| format!("Matrix state for {node_path} has not been loaded"))?;
        let mut next = cached.coefficients.clone();
        for update in updates {
            if update.row >= cached.state.rows || update.col >= cached.state.cols {
                return Err(format!("Matrix cell ({}, {}) is out of bounds", update.row, update.col));
            }
            next.0[update.row][update.col] = match update.db {
                Some(db) => db_to_coefficient(db)?,
                None => 0,
            };
        }
        let transport = DiceTransport::open(node_path)?;
        if let Err(error) = write_coefficients(cached.protocol, &transport, &cached.sections, &cached.caps, &next, &mut cached.coefficients) {
            matrices.remove(node_path);
            return Err(error);
        }
        for update in updates {
            cached.state.gains[update.row][update.col] = coefficient_to_db(next.0[update.row][update.col]);
        }
        Ok(cached.state.clone())
    }

    pub fn restore_coefficients(&self, node_path: &str, coefficients: &[Vec<u16>]) -> Result<(), String> {
        self.get_state(node_path)?;
        let mut matrices = self.matrices.lock().map_err(|_| "Matrix cache lock poisoned".to_owned())?;
        let cached = matrices.get_mut(node_path)
            .ok_or_else(|| format!("Matrix state for {node_path} has not been loaded"))?;
        if coefficients.len() != cached.coefficients.0.len()
            || coefficients.iter().zip(&cached.coefficients.0).any(|(next, current)| next.len() != current.len()) {
            return Err("Saved matrix dimensions do not match the device".to_owned());
        }
        let next = MixerCoefficientParams(coefficients.to_vec());
        let mut previous = MixerCoefficientParams(coefficients.iter()
            .map(|row| row.iter().map(|value| !value).collect()).collect());
        let transport = DiceTransport::open(node_path)?;
        if let Err(error) = write_coefficients(cached.protocol, &transport, &cached.sections, &cached.caps, &next, &mut previous) {
            matrices.remove(node_path);
            return Err(error);
        }
        cached.coefficients = next;
        cached.state.gains = coefficients.iter().take(cached.state.rows)
            .map(|row| row.iter().take(cached.state.cols).map(|value| coefficient_to_db(*value)).collect())
            .collect();
        Ok(())
    }

    pub fn start_peak_polling(&self, node_path: &str) -> Result<(), String> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("Application is shutting down".to_owned());
        }
        let emitter = self
            .meter_emitter
            .lock()
            .map_err(|_| "Matrix event emitter lock poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "Matrix event emitter has not been configured".to_owned())?;
        let cached = self
            .matrices
            .lock()
            .map_err(|_| "Matrix cache lock poisoned".to_owned())?
            .get(node_path)
            .cloned()
            .ok_or_else(|| format!("Matrix state for {node_path} has not been loaded"))?;

        let mut pollers = self
            .pollers
            .lock()
            .map_err(|_| "Matrix poller lock poisoned".to_owned())?;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("Application is shutting down".to_owned());
        }
        if pollers.contains_key(node_path) {
            return Ok(());
        }

        let stop = Arc::new(AtomicBool::new(false));
        pollers.insert(node_path.to_owned(), Arc::clone(&stop));
        let node_path = node_path.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            let mut transport = None;
            let mut last_error: Option<String> = None;
            while !stop.load(Ordering::Acquire) {
                let result = poll_with_reconnect(
                    &mut transport,
                    || DiceTransport::open(&node_path),
                    |opened| read_peak_entries(cached.protocol, opened, &cached.sections, &cached.caps),
                );
                if stop.load(Ordering::Acquire) {
                    break;
                }
                match result {
                    Ok(peaks) => {
                        last_error = None;
                        emitter(MeterUpdate {
                            node: node_path.clone(),
                            input_peaks_db: peak_values(&peaks, &cached.mixer_inputs)
                                .into_iter()
                                .map(round_meter_db)
                                .collect(),
                            destination_peaks: peaks.0.0.iter().map(|entry| DestinationPeak {
                                port_id: destination_port_id(entry.dst),
                                peak_db: round_meter_db(peak_to_db(entry.peak)),
                            }).collect(),
                            error: None,
                        });
                    }
                    Err(error) => {
                        if last_error.as_deref() != Some(&error) {
                            eprintln!("Matrix peak polling failed for {node_path}: {error}");
                            emitter(MeterUpdate {
                                node: node_path.clone(),
                                input_peaks_db: vec![None; cached.mixer_inputs.len()],
                                destination_peaks: Vec::new(),
                                error: Some(error.clone()),
                            });
                            last_error = Some(error);
                        }
                    }
                }
                thread::sleep(if last_error.is_some() { Duration::from_secs(1) } else { METER_INTERVAL });
            }
        });
        Ok(())
    }

    pub fn stop_peak_polling(&self, node_path: &str) {
        if let Ok(mut pollers) = self.pollers.lock()
            && let Some(stop) = pollers.remove(node_path) {
                stop.store(true, Ordering::Release);
            }
    }

    pub fn invalidate(&self, node_path: &str) {
        self.stop_peak_polling(node_path);
        if let Ok(mut matrices) = self.matrices.lock() {
            matrices.remove(node_path);
        }
    }

    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
        let mut pollers = self.pollers.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for (_, stop) in pollers.drain() {
            stop.store(true, Ordering::Release);
        }
    }
}

fn poll_with_reconnect<T, P>(
    transport: &mut Option<T>,
    open: impl FnOnce() -> Result<T, String>,
    read: impl FnOnce(&T) -> Result<P, String>,
) -> Result<P, String> {
    if transport.is_none() {
        *transport = Some(open()?);
    }
    let result = transport.as_ref()
        .ok_or_else(|| "Meter transport is unavailable".to_owned())
        .and_then(read);
    if result.is_err() {
        *transport = None;
    }
    result
}

fn read_matrix(
    protocol: ProtocolKind,
    device: &DeviceInfo,
    node_path: &str,
    transport: &DiceTransport,
    global: &GlobalParameters,
) -> Result<CachedMatrix, String> {
    match protocol {
        ProtocolKind::SaffirePro26 => {
            read_matrix_for::<SPro26Protocol>(protocol, device, node_path, transport, global)
        }
        ProtocolKind::SaffirePro40 => {
            read_matrix_for::<SPro40Protocol>(protocol, device, node_path, transport, global)
        }
        ProtocolKind::ProFire2626 => {
            read_matrix_for::<Pfire2626Protocol>(protocol, device, node_path, transport, global)
        }
        ProtocolKind::FireStudioMobile => {
            read_matrix_for::<FStudioMobileProtocol>(protocol, device, node_path, transport, global)
        }
    }
}

fn read_matrix_for<P: MatrixProtocol>(
    protocol: ProtocolKind,
    device: &DeviceInfo,
    node_path: &str,
    transport: &DiceTransport,
    global: &GlobalParameters,
) -> Result<CachedMatrix, String> {
    let mut sections = ExtensionSections::default();
    P::read_extension_sections(
        transport.request(),
        transport.node(),
        &mut sections,
        TRANSACTION_TIMEOUT_MS,
    )
    .map_err(|error| format!("Failed to read DICE extension sections: {error}"))?;

    let mut caps = ExtensionCaps::default();
    P::read_extension_caps(
        transport.request(),
        transport.node(),
        &sections,
        &mut caps,
        TRANSACTION_TIMEOUT_MS,
    )
    .map_err(|error| format!("Failed to read DICE extension capabilities: {error}"))?;
    if !caps.mixer.is_exposed {
        return Err(format!("{} does not expose a DICE EAP mixer", device.name));
    }

    let rate_mode = RateMode::from_sampling_transfer_frequency(global.current_rate);
    let mixer_inputs = P::compute_avail_mixer_blk_pair(&caps, rate_mode).1;
    let mixer_outputs = (0..caps.mixer.output_count.min(P::mixer_out_port_count(rate_mode)))
        .map(|ch| SrcBlk { id: SrcBlkId::Mixer, ch })
        .collect::<Vec<_>>();

    let mut current_router = CurrentRouterParams {
        entries: RouterParams::default(),
        rate_mode,
    };
    P::cache_extension_whole_params(
        transport.request(),
        transport.node(),
        &sections,
        &caps,
        &mut current_router,
        TRANSACTION_TIMEOUT_MS,
    )
    .map_err(|error| format!("Failed to read current DICE router: {error}"))?;
    let router = current_router.entries;

    let mut coefficients = MixerCoefficientParams::default();
    P::cache_extension_whole_params(
        transport.request(),
        transport.node(),
        &sections,
        &caps,
        &mut coefficients,
        TRANSACTION_TIMEOUT_MS,
    )
    .map_err(|error| format!("Failed to read DICE mixer coefficients: {error}"))?;

    let rows = mixer_outputs.len().min(coefficients.0.len());
    let cols = mixer_inputs
        .len()
        .min(coefficients.0.first().map_or(0, Vec::len));
    let mixer_outputs = &mixer_outputs[..rows];
    let mixer_inputs = &mixer_inputs[..cols];

    let mut state = MatrixState {
        node: node_path.to_owned(),
        rows,
        cols,
        row_labels: mixer_outputs
            .iter()
            .enumerate()
            .map(|(index, output)| output_label::<P>(output, &router, index))
            .collect(),
        column_labels: mixer_inputs
            .iter()
            .enumerate()
            .map(|(index, input)| input_label::<P>(input, &router, index))
            .collect(),
        gains: coefficients
            .0
            .iter()
            .take(rows)
            .map(|output| output.iter().take(cols).map(|&value| coefficient_to_db(value)).collect())
            .collect(),
        input_peaks_db: vec![None; cols],
    };

    if let Ok(peaks) = read_peak_entries(protocol, transport, &sections, &caps) {
        state.input_peaks_db = peak_values(&peaks, mixer_inputs);
    }

    Ok(CachedMatrix {
        protocol,
        sections,
        caps,
        mixer_inputs: mixer_inputs.to_vec(),
        coefficients,
        state,
    })
}

fn write_coefficients(
    protocol: ProtocolKind,
    transport: &DiceTransport,
    sections: &ExtensionSections,
    caps: &ExtensionCaps,
    next: &MixerCoefficientParams,
    previous: &mut MixerCoefficientParams,
) -> Result<(), String> {
    match protocol {
        ProtocolKind::SaffirePro26 => write_coefficients_for::<SPro26Protocol>(transport, sections, caps, next, previous),
        ProtocolKind::SaffirePro40 => write_coefficients_for::<SPro40Protocol>(transport, sections, caps, next, previous),
        ProtocolKind::ProFire2626 => write_coefficients_for::<Pfire2626Protocol>(transport, sections, caps, next, previous),
        ProtocolKind::FireStudioMobile => write_coefficients_for::<FStudioMobileProtocol>(transport, sections, caps, next, previous),
    }
}

fn write_coefficients_for<P: MatrixProtocol>(
    transport: &DiceTransport,
    sections: &ExtensionSections,
    caps: &ExtensionCaps,
    next: &MixerCoefficientParams,
    previous: &mut MixerCoefficientParams,
) -> Result<(), String> {
    P::update_extension_partial_params(
        transport.request(),
        transport.node(),
        sections,
        caps,
        next,
        previous,
        TRANSACTION_TIMEOUT_MS,
    )
    .map_err(|error| format!("Failed to write DICE mixer coefficient: {error}"))
}

fn read_peak_entries(
    protocol: ProtocolKind,
    transport: &DiceTransport,
    sections: &ExtensionSections,
    caps: &ExtensionCaps,
) -> Result<PeakParams, String> {
    match protocol {
        ProtocolKind::SaffirePro26 => read_peak_entries_for::<SPro26Protocol>(transport, sections, caps),
        ProtocolKind::SaffirePro40 => read_peak_entries_for::<SPro40Protocol>(transport, sections, caps),
        ProtocolKind::ProFire2626 => read_peak_entries_for::<Pfire2626Protocol>(transport, sections, caps),
        ProtocolKind::FireStudioMobile => read_peak_entries_for::<FStudioMobileProtocol>(transport, sections, caps),
    }
}

fn read_peak_entries_for<P: MatrixProtocol>(
    transport: &DiceTransport,
    sections: &ExtensionSections,
    caps: &ExtensionCaps,
) -> Result<PeakParams, String> {
    if !caps.general.peak_avail {
        return Ok(PeakParams::default());
    }
    let mut peaks = PeakParams::default();
    P::cache_extension_whole_params(
        transport.request(),
        transport.node(),
        sections,
        caps,
        &mut peaks,
        TRANSACTION_TIMEOUT_MS,
    )
    .map_err(|error| format!("Failed to read DICE peak meters: {error}"))?;
    Ok(peaks)
}

fn round_meter_db(value: Option<f32>) -> Option<f32> {
    value.map(f32::round)
}

fn peak_values(peaks: &PeakParams, destinations: &[DstBlk]) -> Vec<Option<f32>> {
    destinations
        .iter()
        .map(|destination| peak_for_destination(peaks, *destination))
        .collect()
}

fn peak_for_destination(peaks: &PeakParams, destination: DstBlk) -> Option<f32> {
    peaks
        .0
        .0
        .iter()
        .find(|entry| entry.dst == destination)
        .and_then(|entry| peak_to_db(entry.peak))
}

pub(super) fn destination_port_id(destination: DstBlk) -> String {
    format!("{:?}:{}", destination.id, destination.ch)
}

pub(super) fn input_label<P: Tcd22xxSpecification>(input: &DstBlk, router: &RouterParams, index: usize) -> String {
    router
        .0
        .iter()
        .find(|entry| entry.dst == *input)
        .map(|entry| source_block_label::<P>(entry.src))
        .unwrap_or_else(|| format!("Mix In {}", index + 1))
}

pub(super) fn output_label<P: Tcd22xxSpecification>(output: &SrcBlk, router: &RouterParams, index: usize) -> String {
    let routes = router
        .0
        .iter()
        .filter(|entry| entry.src == *output)
        .filter(|entry| !matches!(entry.dst.id, DstBlkId::Reserved(_)))
        .map(|entry| destination_block_label::<P>(entry.dst))
        .filter(|label| !label.is_empty())
        .collect::<Vec<_>>();
    if routes.is_empty() {
        format!("Mix Out {}", index + 1)
    } else {
        routes.join(" / ")
    }
}

pub(super) fn source_block_label<P: Tcd22xxSpecification>(block: SrcBlk) -> String {
    if block.id == SrcBlkId::Mute {
        return "Muted".to_owned();
    }
    let descriptor = P::INPUTS.iter().find(|port| {
        port.id == block.id && block.ch >= port.offset && block.ch < port.offset + port.count
    });
    let (base, offset) = descriptor
        .map(|port| (port.label.unwrap_or_else(|| default_source_name(block.id)), port.offset))
        .unwrap_or_else(|| (default_source_name(block.id), 0));
    format!("{base} {}", block.ch.saturating_sub(offset) as usize + 1)
}

pub(super) fn destination_block_label<P: Tcd22xxSpecification>(block: DstBlk) -> String {
    match block.id {
        DstBlkId::MixerTx0 => return format!("Mixer In {}", block.ch as usize + 1),
        DstBlkId::MixerTx1 => {
            let first_block_count = P::MIXER_IN_PORTS.iter()
                .find(|(id, _)| *id == DstBlkId::MixerTx0)
                .map_or(0, |(_, count)| *count as usize);
            return format!("Mixer In {}", first_block_count + block.ch as usize + 1);
        }
        _ => {}
    }
    let descriptor = P::OUTPUTS.iter().find(|port| {
        port.id == block.id && block.ch >= port.offset && block.ch < port.offset + port.count
    });
    if let Some(port) = descriptor
        && port.label.is_none() && matches!(port.id, DstBlkId::Ins0 | DstBlkId::Ins1) {
            let previous_count: usize = P::OUTPUTS.iter()
                .take_while(|candidate| !std::ptr::eq(*candidate, port))
                .filter(|candidate| candidate.label.is_none() && matches!(candidate.id, DstBlkId::Ins0 | DstBlkId::Ins1))
                .map(|candidate| candidate.count as usize)
                .sum();
            return format!("Line Out {}", previous_count + (block.ch - port.offset) as usize + 1);
        }
    let (base, offset) = descriptor
        .map(|port| (port.label.unwrap_or_else(|| default_destination_name(block.id)), port.offset))
        .unwrap_or_else(|| (default_destination_name(block.id), 0));
    format!("{base} {}", block.ch.saturating_sub(offset) as usize + 1)
}

fn default_source_name(id: SrcBlkId) -> &'static str {
    match id {
        SrcBlkId::Aes => "S/PDIF In",
        SrcBlkId::Adat => "ADAT In",
        SrcBlkId::Mixer => "Mix Out",
        SrcBlkId::Ins0 | SrcBlkId::Ins1 => "Mic/Line In",
        SrcBlkId::ArmAprAudio => "APB Audio In",
        SrcBlkId::Avs0 => "DAW A",
        SrcBlkId::Avs1 => "DAW B",
        SrcBlkId::Mute => "Mute",
        SrcBlkId::Reserved(_) => "Source",
    }
}

fn default_destination_name(id: DstBlkId) -> &'static str {
    match id {
        DstBlkId::Aes => "S/PDIF Out",
        DstBlkId::Adat => "ADAT Out",
        DstBlkId::MixerTx0 => "Mixer A In",
        DstBlkId::MixerTx1 => "Mixer B In",
        DstBlkId::Ins0 | DstBlkId::Ins1 => "Line Out",
        DstBlkId::ArmApbAudio => "APB Audio Out",
        DstBlkId::Avs0 => "DAW A Out",
        DstBlkId::Avs1 => "DAW B Out",
        DstBlkId::Reserved(_) => "Output",
    }
}

fn coefficient_to_db(coefficient: u16) -> Option<f32> {
    if coefficient == 0 {
        None
    } else {
        Some((20.0 * ((coefficient as f64) / COEFFICIENT_UNITY).log10()) as f32)
    }
}

fn db_to_coefficient(db: f32) -> Result<u16, String> {
    if db == f32::NEG_INFINITY || db <= MIN_EDITABLE_DB {
        return Ok(0);
    }
    if !db.is_finite() {
        return Err("Gain must be finite dB or -inf".to_owned());
    }
    let linear = COEFFICIENT_UNITY * 10_f64.powf(db.clamp(MIN_EDITABLE_DB, MAX_EDITABLE_DB) as f64 / 20.0);
    Ok(linear.round().clamp(1.0, u16::MAX as f64) as u16)
}

pub(super) fn peak_to_db(peak: u16) -> Option<f32> {
    (peak != 0).then(|| 20.0 * (peak.min(0x0fff) as f32 / 0x0fff as f32).log10())
}

fn format_id(id: Option<u32>) -> String {
    id.map(|id| format!("0x{id:06x}"))
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{coefficient_to_db, db_to_coefficient, poll_with_reconnect, MatrixStore};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn shutdown_stops_all_pollers_and_rejects_new_ones() {
        let store = MatrixStore::default();
        let first = Arc::new(AtomicBool::new(false));
        let second = Arc::new(AtomicBool::new(false));
        {
            let mut pollers = store.pollers.lock().expect("poller lock");
            pollers.insert("/dev/fw1".to_owned(), Arc::clone(&first));
            pollers.insert("/dev/fw2".to_owned(), Arc::clone(&second));
        }
        store.shutdown();
        assert!(first.load(Ordering::Acquire));
        assert!(second.load(Ordering::Acquire));
        assert!(store.pollers.lock().expect("poller lock").is_empty());
        assert_eq!(store.start_peak_polling("/dev/fw1"), Err("Application is shutting down".to_owned()));
        store.shutdown();
    }

    #[test]
    fn meter_polling_reopens_transport_after_failures() {
        let mut transport = None;
        let mut opens = 0;
        let failure = poll_with_reconnect(&mut transport, || Err::<u32, _>("open failed".to_owned()), |_| Ok(0));
        assert_eq!(failure, Err("open failed".to_owned()));
        assert_eq!(transport, None);

        let failure = poll_with_reconnect(&mut transport, || {
            opens += 1;
            Ok(opens)
        }, |_| Err::<u32, _>("read failed".to_owned()));
        assert_eq!(failure, Err("read failed".to_owned()));
        assert_eq!(transport, None);

        let value = poll_with_reconnect(&mut transport, || {
            opens += 1;
            Ok(opens)
        }, |opened| Ok(*opened));
        assert_eq!(value, Ok(2));
        let value = poll_with_reconnect(&mut transport, || Err("unexpected reopen".to_owned()), |opened| Ok(*opened));
        assert_eq!(value, Ok(2));
    }

    #[test]
    fn maps_dice_fixed_point_coefficients_to_db() {
        assert_eq!(coefficient_to_db(0), None);
        assert_eq!(coefficient_to_db(16_384), Some(0.0));
        let maximum_db = coefficient_to_db(u16::MAX).expect("nonzero coefficient");
        assert!((maximum_db - 12.04).abs() < 0.02);
    }

    #[test]
    fn maps_db_values_to_dice_fixed_point_coefficients() {
        assert_eq!(db_to_coefficient(f32::NEG_INFINITY), Ok(0));
        assert_eq!(db_to_coefficient(-60.0), Ok(0));
        assert_eq!(db_to_coefficient(0.0), Ok(16_384));
        assert!(db_to_coefficient(12.0).unwrap_or_default() > 60_000);
        assert!(db_to_coefficient(f32::NAN).is_err());
    }
}