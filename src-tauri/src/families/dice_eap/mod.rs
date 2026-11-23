pub mod matrix;
pub mod mixer;
pub mod router;
pub mod vendors;

use firewire_dice_protocols::tcat::global_section::{ClockRate, ClockSource, GlobalParameters};
use serde::Serialize;

use super::{DeviceFamily, DeviceFamilyDriver};
use crate::transport::dice::DiceTransport;
use crate::transport::discovery::{self, DeviceInfo};

const MAX_NICKNAME_BYTES: usize = 64;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ClockSourceOption {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DiceGlobalState {
    pub node: String,
    pub nickname: String,
    pub clock_sources: Vec<ClockSourceOption>,
    pub clock_source: String,
    pub sample_rates: Vec<u32>,
    pub sample_rate: Option<u32>,
    pub detected_sample_rate: Option<u32>,
    pub clock_locked: bool,
    pub stream_active: bool,
}

pub struct Driver;

impl DeviceFamilyDriver for Driver {
    fn family(&self) -> DeviceFamily {
        DeviceFamily::DiceEap
    }

    fn supports(&self, device: &DeviceInfo) -> bool {
        device.supported && device.protocol.as_deref() == Some("DICE")
    }
}

pub fn get_global_state(node_path: &str) -> Result<DiceGlobalState, String> {
    let device = dice_device_info(node_path)?;
    let transport = DiceTransport::open(node_path)?;
    let (_, params) = transport.read_global_section()?;
    Ok(to_state(node_path, &params, device_nickname(&device, &params)))
}

pub fn set_nickname(node_path: &str, name: &str) -> Result<DiceGlobalState, String> {
    let device = dice_device_info(node_path)?;
    let transport = DiceTransport::open(node_path)?;
    let (sections, mut params) = transport.read_global_section()?;

    if device_nickname(&device, &params) == name {
        return Ok(to_state(node_path, &params, name.to_owned()));
    }

    if is_focusrite_saffire_pro40(&device) {
        vendors::focusrite::write_saffire_pro40_nickname(
            transport.node(),
            transport.request(),
            name,
        )?;
    } else {
        validate_global_nickname(name)?;
        let mut previous = params.clone();
        params.nickname = name.to_owned();
        transport.write_global_section(&sections, &params, &mut previous)?;
    }

    Ok(to_state(node_path, &params, name.to_owned()))
}

pub fn set_clock_source(node_path: &str, source: &str) -> Result<DiceGlobalState, String> {
    let device = dice_device_info(node_path)?;
    let source = parse_clock_source(source)
        .ok_or_else(|| format!("Unsupported DICE clock source '{source}'"))?;
    let transport = DiceTransport::open(node_path)?;
    let (sections, mut params) = transport.read_global_section()?;
    if !params.avail_sources.contains(&source) && params.clock_config.src != source {
        return Err(format!("Clock source '{}' is not available on this device", source_key(source)));
    }
    if !stream_safe_change(params.clock_config.src, source, params.enable, "clock source")? {
        return Ok(to_state(node_path, &params, device_nickname(&device, &params)));
    }
    let mut previous = params.clone();
    params.clock_config.src = source;
    transport.write_global_section(&sections, &params, &mut previous)?;
    Ok(to_state(node_path, &params, device_nickname(&device, &params)))
}

pub fn set_sample_rate(node_path: &str, rate: u32) -> Result<DiceGlobalState, String> {
    let device = dice_device_info(node_path)?;
    let rate = clock_rate_from_hz(rate)
        .ok_or_else(|| format!("Unsupported sample rate {rate} Hz"))?;
    let transport = DiceTransport::open(node_path)?;
    let (sections, mut params) = transport.read_global_section()?;
    if !params.avail_rates.contains(&rate) && params.clock_config.rate != rate {
        return Err(format!("Sample rate {} Hz is not available on this device", rate_hz(rate).unwrap_or_default()));
    }
    if !stream_safe_change(params.clock_config.rate, rate, params.enable, "sample rate")? {
        return Ok(to_state(node_path, &params, device_nickname(&device, &params)));
    }
    let mut previous = params.clone();
    params.clock_config.rate = rate;
    transport.write_global_section(&sections, &params, &mut previous)?;
    Ok(to_state(node_path, &params, device_nickname(&device, &params)))
}

pub(super) fn stream_safe_change<T: PartialEq>(current: T, requested: T, stream_active: bool, parameter: &str) -> Result<bool, String> {
    if current == requested {
        return Ok(false);
    }
    if stream_active {
        return Err(format!("Cannot change {parameter} while streaming is active. Stop audio streaming (for example PipeWire) and retry; restart FireMix to retry configuration restore."));
    }
    Ok(true)
}

fn to_state(node_path: &str, params: &GlobalParameters, nickname: String) -> DiceGlobalState {
    let mut clock_sources = params
        .avail_sources
        .iter()
        .copied()
        .map(|source| ClockSourceOption {
            value: source_key(source),
            label: params
                .clock_source_labels
                .iter()
                .find(|(available, _)| *available == source)
                .map(|(_, label)| label.clone())
                .unwrap_or_else(|| source_label(source)),
        })
        .collect::<Vec<_>>();
    if !clock_sources.iter().any(|source| source.value == source_key(params.clock_config.src)) {
        clock_sources.push(ClockSourceOption {
            value: source_key(params.clock_config.src),
            label: source_label(params.clock_config.src),
        });
    }

    DiceGlobalState {
        node: node_path.to_owned(),
        nickname,
        clock_sources,
        clock_source: source_key(params.clock_config.src),
        sample_rates: params.avail_rates.iter().filter_map(|rate| rate_hz(*rate)).collect(),
        sample_rate: rate_hz(params.clock_config.rate),
        detected_sample_rate: (params.current_rate > 0).then_some(params.current_rate),
        clock_locked: params.clock_status.src_is_locked,
        stream_active: params.enable,
    }
}

fn dice_device_info(node_path: &str) -> Result<DeviceInfo, String> {
    let device = discovery::device_info_for_path(node_path)?;
    if device.family != Some(DeviceFamily::DiceEap) {
        return Err(format!("Device at {node_path} is not a DICE EAP device"));
    }
    Ok(device)
}

fn device_nickname(device: &DeviceInfo, params: &GlobalParameters) -> String {
    device
        .nickname
        .as_ref()
        .filter(|nickname| !nickname.is_empty())
        .cloned()
        .unwrap_or_else(|| params.nickname.clone())
}

fn is_focusrite_saffire_pro40(device: &DeviceInfo) -> bool {
    device.vendor_id == Some(0x00130e)
        && matches!(device.model_id, Some(0x000005 | 0x000007 | 0x0000de))
}

fn validate_global_nickname(name: &str) -> Result<(), String> {
    if name.len() > MAX_NICKNAME_BYTES {
        return Err(format!("Nickname exceeds {MAX_NICKNAME_BYTES} UTF-8 bytes"));
    }
    if name.contains('\0') {
        return Err("Nickname cannot contain a NUL character".to_owned());
    }
    Ok(())
}

fn source_key(source: ClockSource) -> String {
    match source {
        ClockSource::Aes1 => "Aes1".to_owned(),
        ClockSource::Aes2 => "Aes2".to_owned(),
        ClockSource::Aes3 => "Aes3".to_owned(),
        ClockSource::Aes4 => "Aes4".to_owned(),
        ClockSource::AesAny => "AesAny".to_owned(),
        ClockSource::Adat => "Adat".to_owned(),
        ClockSource::Tdif => "Tdif".to_owned(),
        ClockSource::WordClock => "WordClock".to_owned(),
        ClockSource::Arx1 => "Arx1".to_owned(),
        ClockSource::Arx2 => "Arx2".to_owned(),
        ClockSource::Arx3 => "Arx3".to_owned(),
        ClockSource::Arx4 => "Arx4".to_owned(),
        ClockSource::Internal => "Internal".to_owned(),
        ClockSource::Reserved(value) => format!("Reserved({value})"),
    }
}

fn source_label(source: ClockSource) -> String {
    match source {
        ClockSource::Aes1 => "S/PDIF 1".to_owned(),
        ClockSource::Aes2 => "S/PDIF 2".to_owned(),
        ClockSource::Aes3 => "S/PDIF 3".to_owned(),
        ClockSource::Aes4 => "S/PDIF 4".to_owned(),
        ClockSource::AesAny => "Any S/PDIF".to_owned(),
        ClockSource::Adat => "ADAT".to_owned(),
        ClockSource::Tdif => "TDIF".to_owned(),
        ClockSource::WordClock => "Word Clock".to_owned(),
        ClockSource::Arx1 => "Stream 1".to_owned(),
        ClockSource::Arx2 => "Stream 2".to_owned(),
        ClockSource::Arx3 => "Stream 3".to_owned(),
        ClockSource::Arx4 => "Stream 4".to_owned(),
        ClockSource::Internal => "Internal".to_owned(),
        ClockSource::Reserved(value) => format!("Reserved ({value})"),
    }
}

fn parse_clock_source(value: &str) -> Option<ClockSource> {
    Some(match value {
        "Aes1" => ClockSource::Aes1,
        "Aes2" => ClockSource::Aes2,
        "Aes3" => ClockSource::Aes3,
        "Aes4" => ClockSource::Aes4,
        "AesAny" => ClockSource::AesAny,
        "Adat" => ClockSource::Adat,
        "Tdif" => ClockSource::Tdif,
        "WordClock" => ClockSource::WordClock,
        "Arx1" => ClockSource::Arx1,
        "Arx2" => ClockSource::Arx2,
        "Arx3" => ClockSource::Arx3,
        "Arx4" => ClockSource::Arx4,
        "Internal" => ClockSource::Internal,
        _ => return None,
    })
}

fn rate_hz(rate: ClockRate) -> Option<u32> {
    Some(match rate {
        ClockRate::R32000 => 32_000,
        ClockRate::R44100 => 44_100,
        ClockRate::R48000 => 48_000,
        ClockRate::R88200 => 88_200,
        ClockRate::R96000 => 96_000,
        ClockRate::R176400 => 176_400,
        ClockRate::R192000 => 192_000,
        ClockRate::AnyLow | ClockRate::AnyMid | ClockRate::AnyHigh | ClockRate::None | ClockRate::Reserved(_) => return None,
    })
}

fn clock_rate_from_hz(rate: u32) -> Option<ClockRate> {
    Some(match rate {
        32_000 => ClockRate::R32000,
        44_100 => ClockRate::R44100,
        48_000 => ClockRate::R48000,
        88_200 => ClockRate::R88200,
        96_000 => ClockRate::R96000,
        176_400 => ClockRate::R176400,
        192_000 => ClockRate::R192000,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{stream_safe_change, ClockRate, ClockSource, GlobalParameters};
    use crate::transport::dice::DiceTransport;
    use firewire_dice_protocols::tcat::TcatSectionSerdes;

    #[test]
    fn unchanged_clock_parameters_never_require_a_write() {
        assert_eq!(stream_safe_change(ClockRate::R48000, ClockRate::R48000, true, "sample rate"), Ok(false));
        assert_eq!(stream_safe_change(ClockSource::Internal, ClockSource::Internal, true, "clock source"), Ok(false));
        assert_eq!(stream_safe_change(ClockRate::R48000, ClockRate::R48000, false, "sample rate"), Ok(false));
    }

    #[test]
    fn clock_changes_require_inactive_streaming() {
        let error = stream_safe_change(ClockRate::R48000, ClockRate::R96000, true, "sample rate").expect_err("stream active");
        assert!(error.contains("Stop audio streaming"));
        assert!(stream_safe_change(ClockSource::Internal, ClockSource::Adat, true, "clock source").is_err());
        assert_eq!(stream_safe_change(ClockRate::R48000, ClockRate::R96000, false, "sample rate"), Ok(true));
        assert_eq!(stream_safe_change(ClockSource::Internal, ClockSource::Adat, false, "clock source"), Ok(true));
    }

    #[test]
    fn clock_changes_only_modify_the_clock_quadlet() {
        let previous = GlobalParameters::default();
        let mut next = previous.clone();
        next.clock_config.rate = ClockRate::R48000;
        next.clock_config.src = ClockSource::Internal;
        let mut old = vec![0; <DiceTransport as TcatSectionSerdes<GlobalParameters>>::MIN_SIZE];
        let mut new = old.clone();
        <DiceTransport as TcatSectionSerdes<GlobalParameters>>::serialize(&previous, &mut old).expect("serialize old global state");
        <DiceTransport as TcatSectionSerdes<GlobalParameters>>::serialize(&next, &mut new).expect("serialize new global state");
        let changed: Vec<_> = old.chunks_exact(4).zip(new.chunks_exact(4)).enumerate()
            .filter_map(|(index, (old, new))| (old != new).then_some(index * 4))
            .collect();
        assert_eq!(changed, vec![76]);
    }
}