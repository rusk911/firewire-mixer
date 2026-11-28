use firewire_dice_protocols::focusrite::spro26::SPro26Protocol;
use firewire_dice_protocols::focusrite::spro40::SPro40Protocol;
use firewire_dice_protocols::focusrite::{OpticalOutIfaceMode, SaffireproIoParams};
use firewire_dice_protocols::maudio::Pfire2626Protocol;
use firewire_dice_protocols::presonus::fstudiomobile::FStudioMobileProtocol;
use firewire_dice_protocols::tcat::extension::{
	caps_section::{ExtensionCaps, TcatExtensionCapsSectionOperation},
	cmd_section::{Opcode, RateMode},
	current_config_section::{CurrentRouterParams, CurrentStreamFormatParams},
	FormatEntry,
	ExtensionSections, RouterEntry, RouterParams, SrcBlk, SrcBlkId, DstBlk, DstBlkId,
	TcatExtensionOperation, TcatExtensionSectionParamsOperation,
};
use firewire_dice_protocols::tcat::tcd22xx_spec::{
	Tcd22xxAvailableBlocks, Tcd22xxOperation, Tcd22xxSpecification,
};
use serde::Serialize;

use crate::families::dice_eap::matrix::device_protocol;
use crate::families::dice_eap::mixer::{SavedDestination, SavedDestinationBlock, SavedRoute, SavedSource, SavedSourceBlock};
use crate::transport::dice::DiceTransport;
use crate::transport::discovery::{self, DeviceInfo};

const ROUTER_TIMEOUT_MS: u32 = 200;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RouterPort {
	pub id: String,
	pub label: String,
	pub group: String,
	pub stereo_partner_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RouterDestination {
	pub port: RouterPort,
	pub source_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RouterRoute {
	pub destination_id: String,
	pub source_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RouterRoutesSnapshot {
	pub sample_rate: u32,
	pub routes: Vec<RouterRoute>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RouterState {
	pub node: String,
	pub sample_rate: u32,
	pub sources: Vec<RouterPort>,
	pub destinations: Vec<RouterDestination>,
	pub matrix_row_labels: Vec<String>,
	pub matrix_column_labels: Vec<String>,
	pub matrix_input_port_ids: Vec<String>,
	pub matrix_output_port_ids: Vec<String>,
}

trait RouterProtocol:
	Tcd22xxOperation
	+ TcatExtensionOperation
	+ TcatExtensionCapsSectionOperation
	+ TcatExtensionSectionParamsOperation<CurrentRouterParams>
	+ TcatExtensionSectionParamsOperation<CurrentStreamFormatParams>
{
}

impl<T> RouterProtocol for T where
	T: Tcd22xxOperation
		+ TcatExtensionOperation
		+ TcatExtensionCapsSectionOperation
		+ TcatExtensionSectionParamsOperation<CurrentRouterParams>
		+ TcatExtensionSectionParamsOperation<CurrentStreamFormatParams>
	{
}

struct RouterSnapshot {
	state: RouterState,
	sections: ExtensionSections,
	caps: ExtensionCaps,
	available: Tcd22xxAvailableBlocks,
	rate_mode: RateMode,
	current: CurrentRouterParams,
}

pub fn get_router_state(node_path: &str) -> Result<RouterState, String> {
	read_router_snapshot(node_path).map(|snapshot| snapshot.state)
}

pub fn get_router_routes(node_path: &str) -> Result<RouterRoutesSnapshot, String> {
	let device = discovery::device_info_for_path(node_path)?;
	let protocol = device_protocol(&device)?;
	let transport = DiceTransport::open(node_path)?;
	match protocol {
		crate::families::dice_eap::matrix::ProtocolKind::SaffirePro26 => read_router_routes_for::<SPro26Protocol>(&device, &transport),
		crate::families::dice_eap::matrix::ProtocolKind::SaffirePro40 => read_router_routes_for::<SPro40Protocol>(&device, &transport),
		crate::families::dice_eap::matrix::ProtocolKind::ProFire2626 => read_router_routes_for::<Pfire2626Protocol>(&device, &transport),
		crate::families::dice_eap::matrix::ProtocolKind::FireStudioMobile => read_router_routes_for::<FStudioMobileProtocol>(&device, &transport),
	}
}

fn read_router_routes_for<P: RouterProtocol>(device: &DeviceInfo, transport: &DiceTransport) -> Result<RouterRoutesSnapshot, String> {
	let (_, global) = transport.read_global_section()?;
	let rate_mode = RateMode::from_sampling_transfer_frequency(global.current_rate);
	let mut sections = ExtensionSections::default();
	P::read_extension_sections(transport.request(), transport.node(), &mut sections, ROUTER_TIMEOUT_MS)
		.map_err(|error| format!("Failed to read DICE extension sections: {error}"))?;
	let mut caps = ExtensionCaps::default();
	P::read_extension_caps(transport.request(), transport.node(), &sections, &mut caps, ROUTER_TIMEOUT_MS)
		.map_err(|error| format!("Failed to read DICE capabilities: {error}"))?;
	if !caps.router.is_exposed {
		return Err(format!("{} does not expose a DICE router", device.name));
	}
	let mut available = Tcd22xxAvailableBlocks::default();
	P::detect_available_blocks(transport.request(), transport.node(), &sections, &caps, rate_mode, &mut available, ROUTER_TIMEOUT_MS)
		.map_err(|error| format!("Failed to detect DICE route ports: {error}"))?;
	let mut current = CurrentRouterParams { entries: RouterParams::default(), rate_mode };
	P::cache_extension_whole_params(transport.request(), transport.node(), &sections, &caps, &mut current, ROUTER_TIMEOUT_MS)
		.map_err(|error| format!("Failed to read current DICE router: {error}"))?;
	let routes = available.1.iter().copied().map(|destination| {
		let source_id = current.entries.0.iter().find(|entry| entry.dst == destination)
			.and_then(|entry| (entry.src.id != SrcBlkId::Mute).then(|| source_port_id(entry.src)));
		RouterRoute { destination_id: destination_port_id(destination), source_id }
	}).collect();
	Ok(RouterRoutesSnapshot { sample_rate: global.current_rate, routes })
}

pub fn raw_routes(node_path: &str) -> Result<Vec<SavedRoute>, String> {
	let snapshot = read_router_snapshot(node_path)?;
	Ok(snapshot.current.entries.0.iter().map(save_route).collect())
}

pub fn validate_restorable_routes(node_path: &str, routes: &[SavedRoute], sample_rate: Option<u32>) -> Result<(), String> {
	let device = discovery::device_info_for_path(node_path)?;
	let protocol = device_protocol(&device)?;
	let transport = DiceTransport::open(node_path)?;
	let snapshot = read_router_snapshot_with_transport(node_path, &device, protocol, &transport)?;
	let rate_mode = sample_rate.map(RateMode::from_sampling_transfer_frequency).unwrap_or(snapshot.rate_mode);
	match protocol {
		crate::families::dice_eap::matrix::ProtocolKind::SaffirePro26 => validate_routes_for::<SPro26Protocol>(&transport, &snapshot, routes, rate_mode),
		crate::families::dice_eap::matrix::ProtocolKind::SaffirePro40 => validate_routes_for::<SPro40Protocol>(&transport, &snapshot, routes, rate_mode),
		crate::families::dice_eap::matrix::ProtocolKind::ProFire2626 => validate_routes_for::<Pfire2626Protocol>(&transport, &snapshot, routes, rate_mode),
		crate::families::dice_eap::matrix::ProtocolKind::FireStudioMobile => validate_routes_for::<FStudioMobileProtocol>(&transport, &snapshot, routes, rate_mode),
	}
}

fn validate_routes_for<P: RouterProtocol>(transport: &DiceTransport, snapshot: &RouterSnapshot, routes: &[SavedRoute], rate_mode: RateMode) -> Result<(), String> {
	if routes.len() > snapshot.caps.router.maximum_entry_count as usize {
		return Err("Saved router exceeds device capacity".to_owned());
	}
	let mut available = Tcd22xxAvailableBlocks::default();
	P::detect_available_blocks(transport.request(), transport.node(), &snapshot.sections, &snapshot.caps, rate_mode, &mut available, ROUTER_TIMEOUT_MS)
		.map_err(|error| format!("Failed to validate saved router ports: {error}"))?;
	let mut current = CurrentRouterParams { entries: RouterParams::default(), rate_mode };
	P::cache_extension_whole_params(transport.request(), transport.node(), &snapshot.sections, &snapshot.caps, &mut current, ROUTER_TIMEOUT_MS)
		.map_err(|error| format!("Failed to validate saved router configuration: {error}"))?;
	for route in routes {
		let entry = load_route(route);
		let known_source = available.0.contains(&entry.src)
			|| entry.src.id == SrcBlkId::Mute
			|| matches!(entry.src.id, SrcBlkId::Reserved(_))
			|| current.entries.0.iter().any(|existing| existing.src == entry.src);
		let known_destination = available.1.contains(&entry.dst)
			|| matches!(entry.dst.id, DstBlkId::Reserved(_))
			|| current.entries.0.iter().any(|existing| existing.dst == entry.dst);
		if !known_source || !known_destination {
			return Err(format!("Saved router port is unavailable: {:?} -> {:?}", entry.src, entry.dst));
		}
	}
	Ok(())
}

pub fn restore_routes(node_path: &str, routes: &[SavedRoute], sample_rate: Option<u32>) -> Result<(), String> {
	let device = discovery::device_info_for_path(node_path)?;
	let protocol = device_protocol(&device)?;
	let transport = DiceTransport::open(node_path)?;
	let mut snapshot = read_router_snapshot_with_transport(node_path, &device, protocol, &transport)?;
	if let Some(rate) = sample_rate {
		snapshot.rate_mode = RateMode::from_sampling_transfer_frequency(rate);
	}
	if routes.len() > snapshot.caps.router.maximum_entry_count as usize {
		return Err("Saved router exceeds device capacity".to_owned());
	}
	let params = RouterParams(routes.iter().map(load_route).collect());
	match protocol {
		crate::families::dice_eap::matrix::ProtocolKind::SaffirePro26 => restore_router_for::<SPro26Protocol>(&transport, &snapshot, &params),
		crate::families::dice_eap::matrix::ProtocolKind::SaffirePro40 => restore_router_for::<SPro40Protocol>(&transport, &snapshot, &params),
		crate::families::dice_eap::matrix::ProtocolKind::ProFire2626 => restore_router_for::<Pfire2626Protocol>(&transport, &snapshot, &params),
		crate::families::dice_eap::matrix::ProtocolKind::FireStudioMobile => restore_router_for::<FStudioMobileProtocol>(&transport, &snapshot, &params),
	}
}

fn restore_router_for<P: RouterProtocol>(transport: &DiceTransport, snapshot: &RouterSnapshot, params: &RouterParams) -> Result<(), String> {
	P::update_extension_whole_params(transport.request(), transport.node(), &snapshot.sections, &snapshot.caps, params, ROUTER_TIMEOUT_MS)
		.map_err(|error| format!("Failed to restore DICE router table: {error}"))?;
	P::initiate(transport.request(), transport.node(), &snapshot.sections, &snapshot.caps, Opcode::LoadRouter(snapshot.rate_mode), ROUTER_TIMEOUT_MS)
		.map_err(|error| format!("Failed to activate restored DICE router: {error}"))?;
	Ok(())
}

fn save_route(entry: &RouterEntry) -> SavedRoute {
	let source = match entry.src.id {
		SrcBlkId::Aes => SavedSourceBlock::Aes, SrcBlkId::Adat => SavedSourceBlock::Adat,
		SrcBlkId::Mixer => SavedSourceBlock::Mixer, SrcBlkId::Ins0 => SavedSourceBlock::Ins0,
		SrcBlkId::Ins1 => SavedSourceBlock::Ins1, SrcBlkId::ArmAprAudio => SavedSourceBlock::ArmAprAudio,
		SrcBlkId::Avs0 => SavedSourceBlock::Avs0, SrcBlkId::Avs1 => SavedSourceBlock::Avs1,
		SrcBlkId::Mute => SavedSourceBlock::Mute, SrcBlkId::Reserved(value) => SavedSourceBlock::Reserved(value),
	};
	let destination = match entry.dst.id {
		DstBlkId::Aes => SavedDestinationBlock::Aes, DstBlkId::Adat => SavedDestinationBlock::Adat,
		DstBlkId::MixerTx0 => SavedDestinationBlock::MixerTx0, DstBlkId::MixerTx1 => SavedDestinationBlock::MixerTx1,
		DstBlkId::Ins0 => SavedDestinationBlock::Ins0, DstBlkId::Ins1 => SavedDestinationBlock::Ins1,
		DstBlkId::ArmApbAudio => SavedDestinationBlock::ArmApbAudio, DstBlkId::Avs0 => SavedDestinationBlock::Avs0,
		DstBlkId::Avs1 => SavedDestinationBlock::Avs1, DstBlkId::Reserved(value) => SavedDestinationBlock::Reserved(value),
	};
	SavedRoute { source: SavedSource { block: source, channel: entry.src.ch }, destination: SavedDestination { block: destination, channel: entry.dst.ch } }
}

fn load_route(route: &SavedRoute) -> RouterEntry {
	let source = match route.source.block {
		SavedSourceBlock::Aes => SrcBlkId::Aes, SavedSourceBlock::Adat => SrcBlkId::Adat,
		SavedSourceBlock::Mixer => SrcBlkId::Mixer, SavedSourceBlock::Ins0 => SrcBlkId::Ins0,
		SavedSourceBlock::Ins1 => SrcBlkId::Ins1, SavedSourceBlock::ArmAprAudio => SrcBlkId::ArmAprAudio,
		SavedSourceBlock::Avs0 => SrcBlkId::Avs0, SavedSourceBlock::Avs1 => SrcBlkId::Avs1,
		SavedSourceBlock::Mute => SrcBlkId::Mute, SavedSourceBlock::Reserved(value) => SrcBlkId::Reserved(value),
	};
	let destination = match route.destination.block {
		SavedDestinationBlock::Aes => DstBlkId::Aes, SavedDestinationBlock::Adat => DstBlkId::Adat,
		SavedDestinationBlock::MixerTx0 => DstBlkId::MixerTx0, SavedDestinationBlock::MixerTx1 => DstBlkId::MixerTx1,
		SavedDestinationBlock::Ins0 => DstBlkId::Ins0, SavedDestinationBlock::Ins1 => DstBlkId::Ins1,
		SavedDestinationBlock::ArmApbAudio => DstBlkId::ArmApbAudio, SavedDestinationBlock::Avs0 => DstBlkId::Avs0,
		SavedDestinationBlock::Avs1 => DstBlkId::Avs1, SavedDestinationBlock::Reserved(value) => DstBlkId::Reserved(value),
	};
	RouterEntry { src: SrcBlk { id: source, ch: route.source.channel }, dst: DstBlk { id: destination, ch: route.destination.channel }, ..Default::default() }
}

pub fn set_route(
	node_path: &str,
	destination_id: &str,
	source_id: Option<&str>,
) -> Result<RouterState, String> {
	let device = discovery::device_info_for_path(node_path)?;
	let protocol = device_protocol(&device)?;
	let transport = DiceTransport::open(node_path)?;
	let mut snapshot = read_router_snapshot_with_transport(node_path, &device, protocol, &transport)?;

	let _destination = snapshot
		.state
		.destinations
		.iter()
		.find(|port| port.port.id == destination_id)
		.ok_or_else(|| format!("Unknown router destination '{destination_id}'"))?
		.port
		.id
		.clone();
	let destination_block = snapshot
		.available
		.1
		.iter()
		.find(|block| destination_port_id(**block) == destination_id)
		.copied()
		.ok_or_else(|| format!("Router destination '{destination_id}' is unavailable"))?;

	let source_block = match source_id {
		Some(source_id) => Some(
			snapshot
				.state
				.sources
				.iter()
				.find(|port| port.id == source_id)
				.ok_or_else(|| format!("Unknown router source '{source_id}'"))?
				.id
				.as_str(),
		),
		None => None,
	};
	let source_block = match source_block {
		Some(source_id) => snapshot
			.available
			.0
			.iter()
			.find(|block| source_port_id(**block) == source_id)
			.copied()
			.ok_or_else(|| format!("Router source '{source_id}' is unavailable"))?,
		None => SrcBlk {
			id: SrcBlkId::Mute,
			ch: 0,
		},
	};

	if let Some(entry) = snapshot.current.entries.0.iter_mut().find(|entry| entry.dst == destination_block) {
		entry.src = source_block;
	} else {
		snapshot.current.entries.0.push(RouterEntry {
			dst: destination_block,
			src: source_block,
			..Default::default()
		});
	}
	include_muted_sources(&snapshot.current.entries, &mut snapshot.available);

	update_router(
		protocol,
		&transport,
		&snapshot.sections,
		&snapshot.caps,
		snapshot.rate_mode,
		&snapshot.available,
		&snapshot.current.entries,
	)?;

	read_router_snapshot_with_transport(node_path, &device, protocol, &transport)
		.map(|snapshot| snapshot.state)
}

fn read_router_snapshot(node_path: &str) -> Result<RouterSnapshot, String> {
	let device = discovery::device_info_for_path(node_path)?;
	let protocol = device_protocol(&device)?;
	let transport = DiceTransport::open(node_path)?;
	read_router_snapshot_with_transport(node_path, &device, protocol, &transport)
}

fn read_router_snapshot_with_transport(
	node_path: &str,
	device: &DeviceInfo,
	protocol: crate::families::dice_eap::matrix::ProtocolKind,
	transport: &DiceTransport,
) -> Result<RouterSnapshot, String> {
	match protocol {
		crate::families::dice_eap::matrix::ProtocolKind::SaffirePro26 => {
			read_router_snapshot_for::<SPro26Protocol>(node_path, device, transport)
		}
		crate::families::dice_eap::matrix::ProtocolKind::SaffirePro40 => {
			read_router_snapshot_for::<SPro40Protocol>(node_path, device, transport)
		}
		crate::families::dice_eap::matrix::ProtocolKind::ProFire2626 => {
			read_router_snapshot_for::<Pfire2626Protocol>(node_path, device, transport)
		}
		crate::families::dice_eap::matrix::ProtocolKind::FireStudioMobile => {
			read_router_snapshot_for::<FStudioMobileProtocol>(node_path, device, transport)
		}
	}
}

fn read_router_snapshot_for<P: RouterProtocol>(
	node_path: &str,
	device: &DeviceInfo,
	transport: &DiceTransport,
) -> Result<RouterSnapshot, String> {
	let (_, global) = transport.read_global_section()?;
	let rate_mode = RateMode::from_sampling_transfer_frequency(global.current_rate);
	let mut sections = ExtensionSections::default();
	P::read_extension_sections(
		transport.request(),
		transport.node(),
		&mut sections,
		ROUTER_TIMEOUT_MS,
	)
	.map_err(|error| format!("Failed to read DICE extension sections: {error}"))?;

	let mut caps = ExtensionCaps::default();
	P::read_extension_caps(
		transport.request(),
		transport.node(),
		&sections,
		&mut caps,
		ROUTER_TIMEOUT_MS,
	)
	.map_err(|error| format!("Failed to read DICE capabilities: {error}"))?;
	if !caps.router.is_exposed {
		return Err(format!("{} does not expose a DICE router", device.name));
	}

	let mut available = Tcd22xxAvailableBlocks::default();
	P::detect_available_blocks(
		transport.request(),
		transport.node(),
		&sections,
		&caps,
		rate_mode,
		&mut available,
		ROUTER_TIMEOUT_MS,
	)
	.map_err(|error| format!("Failed to detect DICE route ports: {error}"))?;

	let optical_mode = if matches!((device.vendor_id, device.model_id),
		(Some(0x00130e), Some(0x000005 | 0x000007 | 0x0000de))) {
		let mut params = SaffireproIoParams::default();
		SPro40Protocol::cache_extension_whole_params(
			transport.request(), transport.node(), &sections, &caps, &mut params, ROUTER_TIMEOUT_MS,
		)
		.map_err(|error| format!("Failed to read Focusrite optical mode: {error}"))?;
		Some(params.opt_out_iface_mode)
	} else {
		None
	};

	let mut stream_params = CurrentStreamFormatParams {
		pair: Default::default(),
		rate_mode,
	};
	P::cache_extension_whole_params(
		transport.request(),
		transport.node(),
		&sections,
		&caps,
		&mut stream_params,
		ROUTER_TIMEOUT_MS,
	)
	.map_err(|error| format!("Failed to read DICE stream formats: {error}"))?;

	let mut current = CurrentRouterParams {
		entries: RouterParams::default(),
		rate_mode,
	};
	P::cache_extension_whole_params(
		transport.request(),
		transport.node(),
		&sections,
		&caps,
		&mut current,
		ROUTER_TIMEOUT_MS,
	)
	.map_err(|error| format!("Failed to read current DICE router: {error}"))?;

	let sources = available
		.0
		.iter()
		.copied()
		.filter(|source| source.id != SrcBlkId::Mute)
		.filter(|source| optical_source_visible(*source, optical_mode))
		.map(|source| RouterPort {
			id: source_port_id(source),
			label: source_label::<P>(source, &stream_params),
			group: source_group(source.id).to_owned(),
			stereo_partner_id: source_partner::<P>(source, &available, &stream_params).map(source_port_id),
		})
		.collect::<Vec<_>>();

	let destinations = available
		.1
		.iter()
		.copied()
		.filter(|destination| optical_destination_visible(*destination, optical_mode))
		.map(|destination| {
			let route = current
				.entries
				.0
				.iter()
				.find(|entry| entry.dst == destination);
			let source_id = route.and_then(|entry| {
				(entry.src.id != SrcBlkId::Mute).then(|| source_port_id(entry.src))
			});
			RouterDestination {
				port: RouterPort {
					id: destination_port_id(destination),
					label: destination_label::<P>(destination, &stream_params),
					group: destination_group(destination.id).to_owned(),
					stereo_partner_id: None,
				},
				source_id,
			}
		})
		.collect::<Vec<_>>();

	let mixer_pair = P::compute_avail_mixer_blk_pair(&caps, rate_mode);
	let matrix_row_labels = mixer_pair
		.0
		.iter()
		.map(|source| matrix_output_label::<P>(*source, &current.entries, &stream_params))
		.collect();
	let matrix_column_labels = mixer_pair
		.1
		.iter()
		.enumerate()
		.map(|(index, destination)| {
			current
				.entries
				.0
				.iter()
				.find(|entry| entry.dst == *destination)
				.map(|entry| source_label::<P>(entry.src, &stream_params))
				.unwrap_or_else(|| format!("Mix In {}", index + 1))
		})
		.collect();

	Ok(RouterSnapshot {
		state: RouterState {
			node: node_path.to_owned(),
			sample_rate: global.current_rate,
			sources,
			destinations,
			matrix_row_labels,
			matrix_column_labels,
			matrix_input_port_ids: mixer_pair.1.iter().map(|destination| destination_port_id(*destination)).collect(),
			matrix_output_port_ids: mixer_pair.0.iter().map(|source| source_port_id(*source)).collect(),
		},
		sections,
		caps,
		available,
		rate_mode,
		current,
	})
}

fn update_router(
	protocol: crate::families::dice_eap::matrix::ProtocolKind,
	transport: &DiceTransport,
	sections: &ExtensionSections,
	caps: &ExtensionCaps,
	rate_mode: RateMode,
	available: &Tcd22xxAvailableBlocks,
	current: &RouterParams,
) -> Result<(), String> {
	match protocol {
		crate::families::dice_eap::matrix::ProtocolKind::SaffirePro26 => {
			update_router_for::<SPro26Protocol>(transport, sections, caps, rate_mode, available, current)
		}
		crate::families::dice_eap::matrix::ProtocolKind::SaffirePro40 => {
			update_router_for::<SPro40Protocol>(transport, sections, caps, rate_mode, available, current)
		}
		crate::families::dice_eap::matrix::ProtocolKind::ProFire2626 => {
			update_router_for::<Pfire2626Protocol>(transport, sections, caps, rate_mode, available, current)
		}
		crate::families::dice_eap::matrix::ProtocolKind::FireStudioMobile => {
			update_router_for::<FStudioMobileProtocol>(transport, sections, caps, rate_mode, available, current)
		}
	}
}

fn update_router_for<P: RouterProtocol>(
	transport: &DiceTransport,
	sections: &ExtensionSections,
	caps: &ExtensionCaps,
	rate_mode: RateMode,
	available: &Tcd22xxAvailableBlocks,
	current: &RouterParams,
) -> Result<(), String> {
	let mut updated = current.clone();
	P::update_router_entries(
		transport.request(),
		transport.node(),
		sections,
		caps,
		rate_mode,
		available,
		&mut updated,
		ROUTER_TIMEOUT_MS,
	)
	.map_err(|error| format!("Failed to write DICE router: {error}"))
}

fn source_label<P: Tcd22xxSpecification>(block: SrcBlk, streams: &CurrentStreamFormatParams) -> String {
	match block.id {
		SrcBlkId::Avs0 | SrcBlkId::Avs1 => {
			let stream_index = usize::from(block.id == SrcBlkId::Avs1);
			stream_label(&streams.pair.rx_entries, stream_index, block.ch as usize)
		}
		_ => crate::families::dice_eap::matrix::source_block_label::<P>(block),
	}
}

fn destination_label<P: Tcd22xxSpecification>(
	block: DstBlk,
	streams: &CurrentStreamFormatParams,
) -> String {
	match block.id {
		DstBlkId::Avs0 | DstBlkId::Avs1 => {
			let stream_index = usize::from(block.id == DstBlkId::Avs1);
			stream_label(&streams.pair.tx_entries, stream_index, block.ch as usize)
		}
		_ => crate::families::dice_eap::matrix::destination_block_label::<P>(block),
	}
}

fn matrix_output_label<P: Tcd22xxSpecification>(source: SrcBlk, routes: &RouterParams, streams: &CurrentStreamFormatParams) -> String {
	let labels = routes.0.iter()
		.filter(|entry| entry.src == source && !matches!(entry.dst.id, DstBlkId::Reserved(_)))
		.map(|entry| destination_label::<P>(entry.dst, streams))
		.collect::<Vec<_>>()
		.join(" / ");
	if labels.is_empty() { format!("Mix Out {}", source.ch as usize + 1) } else { labels }
}

fn stream_label(
	entries: &[FormatEntry],
	stream_index: usize,
	channel: usize,
) -> String {
	let preceding_channels: usize = entries.iter()
		.take(stream_index)
		.map(|entry| entry.pcm_count as usize)
		.sum();
	format!("DAW {}", preceding_channels + channel + 1)
}

fn source_port_id(block: SrcBlk) -> String {
	format!("{:?}:{}", block.id, block.ch)
}

fn source_partner<P: Tcd22xxSpecification>(source: SrcBlk, available: &Tcd22xxAvailableBlocks, streams: &CurrentStreamFormatParams) -> Option<SrcBlk> {
	let partner = match source.id {
		SrcBlkId::Avs0 | SrcBlkId::Avs1 => {
			let stream_index = usize::from(source.id == SrcBlkId::Avs1);
			let preceding: usize = streams.pair.rx_entries.iter().take(stream_index).map(|entry| entry.pcm_count as usize).sum();
			if !(preceding + source.ch as usize).is_multiple_of(2) { return None; }
			let current = streams.pair.rx_entries.get(stream_index)?;
			if source.ch.checked_add(1)? < current.pcm_count {
				SrcBlk { id: source.id, ch: source.ch + 1 }
			} else if stream_index == 0 && streams.pair.rx_entries.get(1).is_some_and(|entry| entry.pcm_count > 0) {
				SrcBlk { id: SrcBlkId::Avs1, ch: 0 }
			} else {
				return None;
			}
		}
		_ => {
			let descriptor = P::INPUTS.iter().find(|port| port.id == source.id && source.ch >= port.offset && source.ch < port.offset + port.count);
			let offset = descriptor.map_or(0, |port| port.offset);
			if !(source.ch - offset).is_multiple_of(2) { return None; }
			let channel = source.ch.checked_add(1)?;
			if descriptor.is_some_and(|port| channel >= port.offset + port.count) { return None; }
			SrcBlk { id: source.id, ch: channel }
		}
	};
	available.0.contains(&partner).then_some(partner)
}

fn optical_source_visible(block: SrcBlk, mode: Option<OpticalOutIfaceMode>) -> bool {
	match mode {
		Some(OpticalOutIfaceMode::Adat) => !(block.id == SrcBlkId::Aes && (4..6).contains(&block.ch)),
		Some(OpticalOutIfaceMode::Spdif) => block.id != SrcBlkId::Adat,
		_ => true,
	}
}

fn optical_destination_visible(block: DstBlk, mode: Option<OpticalOutIfaceMode>) -> bool {
	match mode {
		Some(OpticalOutIfaceMode::Adat) => !(block.id == DstBlkId::Aes && (4..6).contains(&block.ch)),
		Some(OpticalOutIfaceMode::Spdif) => block.id != DstBlkId::Adat,
		_ => true,
	}
}

fn include_muted_sources(entries: &RouterParams, available: &mut Tcd22xxAvailableBlocks) {
	for entry in &entries.0 {
		if entry.src.id == SrcBlkId::Mute && !available.0.contains(&entry.src) {
			available.0.push(entry.src);
		}
	}
}

fn destination_port_id(block: DstBlk) -> String {
	format!("{:?}:{}", block.id, block.ch)
}

fn source_group(id: SrcBlkId) -> &'static str {
	match id {
		SrcBlkId::Ins0 | SrcBlkId::Ins1 | SrcBlkId::Aes | SrcBlkId::Adat => "Physical Inputs",
		SrcBlkId::Avs0 | SrcBlkId::Avs1 => "DAW Playback",
		SrcBlkId::Mixer => "Mixer Outputs",
		SrcBlkId::ArmAprAudio => "System Audio",
		SrcBlkId::Mute => "Disconnected",
		SrcBlkId::Reserved(_) => "Other",
	}
}

fn destination_group(id: DstBlkId) -> &'static str {
	match id {
		DstBlkId::Ins0 | DstBlkId::Ins1 | DstBlkId::Aes | DstBlkId::Adat => "Physical Outputs",
		DstBlkId::Avs0 | DstBlkId::Avs1 => "DAW / FireWire Inputs",
		DstBlkId::MixerTx0 | DstBlkId::MixerTx1 => "Mixer Inputs",
		DstBlkId::ArmApbAudio => "System Audio",
		DstBlkId::Reserved(_) => "Other",
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn matrix_output_labels_ignore_reserved_destinations() {
		let source = SrcBlk { id: SrcBlkId::Mixer, ch: 0 };
		let routes = RouterParams(vec![
			RouterEntry { src: source, dst: DstBlk { id: DstBlkId::Ins0, ch: 0 }, ..Default::default() },
			RouterEntry { src: source, dst: DstBlk { id: DstBlkId::Reserved(15), ch: 0 }, ..Default::default() },
		]);
		assert_eq!(matrix_output_label::<SPro40Protocol>(source, &routes, &CurrentStreamFormatParams::default()), "Line Out 1");
	}

	#[test]
	fn stereo_pairs_follow_daw_numbering_across_stream_boundaries() {
		let mut streams = CurrentStreamFormatParams { pair: Default::default(), rate_mode: RateMode::Low };
		streams.pair.rx_entries = vec![FormatEntry { pcm_count: 3, ..Default::default() }, FormatEntry { pcm_count: 3, ..Default::default() }];
		let mut available = Tcd22xxAvailableBlocks::default();
		available.0 = vec![SrcBlk { id: SrcBlkId::Avs0, ch: 2 }, SrcBlk { id: SrcBlkId::Avs1, ch: 0 }, SrcBlk { id: SrcBlkId::Avs1, ch: 1 }, SrcBlk { id: SrcBlkId::Avs1, ch: 2 }];
		assert_eq!(source_partner::<SPro40Protocol>(available.0[0], &available, &streams), Some(available.0[1]));
		assert_eq!(source_partner::<SPro40Protocol>(available.0[1], &available, &streams), None);
		assert_eq!(source_partner::<SPro40Protocol>(available.0[2], &available, &streams), Some(available.0[3]));
	}

	#[test]
	fn saved_routes_preserve_unexposed_blocks() {
		let entries = [
			RouterEntry { src: SrcBlk { id: SrcBlkId::Reserved(15), ch: 3 }, dst: DstBlk { id: DstBlkId::Reserved(255), ch: 255 }, ..Default::default() },
			RouterEntry { src: SrcBlk { id: SrcBlkId::Mute, ch: 0 }, dst: DstBlk { id: DstBlkId::MixerTx1, ch: 1 }, ..Default::default() },
		];
		for entry in entries {
			assert_eq!(load_route(&save_route(&entry)), entry);
		}
	}

	#[test]
	fn daw_channels_are_numbered_across_streams_independently_per_direction() {
		let mut streams = CurrentStreamFormatParams { pair: Default::default(), rate_mode: RateMode::Low };
		streams.pair.rx_entries = vec![
			FormatEntry { pcm_count: 12, labels: vec!["device label".to_owned()], ..Default::default() },
			FormatEntry { pcm_count: 8, ..Default::default() },
		];
		streams.pair.tx_entries = vec![
			FormatEntry { pcm_count: 10, ..Default::default() },
			FormatEntry { pcm_count: 8, ..Default::default() },
		];
		assert_eq!(source_label::<SPro40Protocol>(SrcBlk { id: SrcBlkId::Avs0, ch: 0 }, &streams), "DAW 1");
		assert_eq!(source_label::<SPro40Protocol>(SrcBlk { id: SrcBlkId::Avs1, ch: 0 }, &streams), "DAW 13");
		assert_eq!(source_label::<SPro40Protocol>(SrcBlk { id: SrcBlkId::Avs1, ch: 7 }, &streams), "DAW 20");
		assert_eq!(destination_label::<SPro40Protocol>(DstBlk { id: DstBlkId::Avs1, ch: 0 }, &streams), "DAW 11");
		assert_eq!(destination_label::<SPro40Protocol>(DstBlk { id: DstBlkId::Avs1, ch: 7 }, &streams), "DAW 18");
	}

	#[test]
	fn optical_modes_filter_only_the_inactive_optical_ports() {
		for mode in [OpticalOutIfaceMode::Adat, OpticalOutIfaceMode::Spdif] {
			let is_adat = mode == OpticalOutIfaceMode::Adat;
			for channel in 0..2 {
				assert!(optical_source_visible(SrcBlk { id: SrcBlkId::Aes, ch: channel }, Some(mode)));
				assert!(optical_destination_visible(DstBlk { id: DstBlkId::Aes, ch: channel }, Some(mode)));
				assert_eq!(optical_source_visible(SrcBlk { id: SrcBlkId::Aes, ch: channel + 4 }, Some(mode)), !is_adat);
				assert_eq!(optical_destination_visible(DstBlk { id: DstBlkId::Aes, ch: channel + 4 }, Some(mode)), !is_adat);
				assert_eq!(optical_source_visible(SrcBlk { id: SrcBlkId::Adat, ch: channel }, Some(mode)), is_adat);
				assert_eq!(optical_destination_visible(DstBlk { id: DstBlkId::Adat, ch: channel }, Some(mode)), is_adat);
			}
		}
	}

	#[test]
	fn mixer_destinations_are_numbered_across_blocks() {
		use crate::families::dice_eap::matrix::destination_block_label;
		assert_eq!(destination_block_label::<SPro40Protocol>(DstBlk { id: DstBlkId::MixerTx0, ch: 0 }), "Mixer In 1");
		assert_eq!(destination_block_label::<SPro40Protocol>(DstBlk { id: DstBlkId::MixerTx0, ch: 15 }), "Mixer In 16");
		assert_eq!(destination_block_label::<SPro40Protocol>(DstBlk { id: DstBlkId::MixerTx1, ch: 0 }), "Mixer In 17");
		assert_eq!(destination_block_label::<SPro40Protocol>(DstBlk { id: DstBlkId::MixerTx1, ch: 1 }), "Mixer In 18");
	}

	#[test]
	fn saffire_pro_40_analog_outputs_are_numbered_across_blocks() {
		use crate::families::dice_eap::matrix::destination_block_label;
		let outputs = SPro40Protocol::compute_avail_real_blk_pair(RateMode::Low).1;
		let labels: Vec<_> = outputs.iter()
			.filter(|block| matches!(block.id, DstBlkId::Ins0 | DstBlkId::Ins1))
			.map(|block| destination_block_label::<SPro40Protocol>(*block))
			.collect();
		assert_eq!(labels, (1..=10).map(|number| format!("Line Out {number}")).collect::<Vec<_>>());
	}

	#[test]
	fn muted_routes_survive_protocol_refinement() {
		let source = SrcBlk { id: SrcBlkId::Mute, ch: 0 };
		let destination = DstBlk { id: DstBlkId::Ins0, ch: 0 };
		let mut entries = RouterParams(vec![RouterEntry { src: source, dst: destination, ..Default::default() }]);
		let mut available = Tcd22xxAvailableBlocks::default();
		available.1.push(destination);
		include_muted_sources(&entries, &mut available);
		SPro40Protocol::refine_router_entries(&mut entries.0, &available);
		assert!(entries.0.iter().any(|entry| entry.src == source && entry.dst == destination));
	}

	#[test]
	#[ignore = "requires a DICE device specified by FIREMIX_TEST_NODE"]
	fn reads_router_from_hardware() {
		let node = std::env::var("FIREMIX_TEST_NODE").expect("FIREMIX_TEST_NODE must name a device");
		let state = get_router_state(&node).expect("router state should be readable");
		assert!(!state.sources.is_empty());
		assert!(!state.destinations.is_empty());
		assert!(!(state.sources.iter().any(|port| port.id.starts_with("Adat:"))
			&& state.sources.iter().any(|port| port.id == "Aes:4" || port.id == "Aes:5")));
		assert!(!(state.destinations.iter().any(|entry| entry.port.id.starts_with("Adat:"))
			&& state.destinations.iter().any(|entry| entry.port.id == "Aes:4" || entry.port.id == "Aes:5")));
		assert!(state.destinations.iter().any(|entry| entry.port.label == "Mixer In 1"));
		assert!(state.destinations.iter().any(|entry| entry.port.label == "Mixer In 18"));
		assert!(state.matrix_row_labels.iter().all(|label| !label.contains("Output ")));
		let analog_outputs: Vec<_> = state.destinations.iter()
			.filter(|entry| entry.port.id.starts_with("Ins0:") || entry.port.id.starts_with("Ins1:"))
			.map(|entry| entry.port.label.clone())
			.collect();
		assert_eq!(analog_outputs, (1..=10).map(|number| format!("Line Out {number}")).collect::<Vec<_>>());
		let daw_sources: Vec<_> = state.sources.iter()
			.filter(|port| port.id.starts_with("Avs0:") || port.id.starts_with("Avs1:"))
			.map(|port| port.label.clone()).collect();
		let daw_destinations: Vec<_> = state.destinations.iter()
			.filter(|entry| entry.port.id.starts_with("Avs0:") || entry.port.id.starts_with("Avs1:"))
			.map(|entry| entry.port.label.clone()).collect();
		assert_eq!(daw_sources, (1..=daw_sources.len()).map(|number| format!("DAW {number}")).collect::<Vec<_>>());
		assert_eq!(daw_destinations, (1..=daw_destinations.len()).map(|number| format!("DAW {number}")).collect::<Vec<_>>());
		println!("DAW: {} playback, {} capture", daw_sources.len(), daw_destinations.len());
		println!("{} sources, {} destinations", state.sources.len(), state.destinations.len());
	}
}