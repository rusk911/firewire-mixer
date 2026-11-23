use std::fs;
use std::path::Path;
use std::sync::Arc;

use firewire_dice_protocols::tcat::{
    global_section::{GlobalParameters, TcatGlobalSectionSpecification},
    GeneralSections, TcatOperation, TcatSectionOperation,
};
use glib::{MainContext, MainLoop};
use hinawa::prelude::{FwNodeExt, FwNodeExtManual};
use hinawa::{FwNode, FwReq};
use serde::Serialize;

const DICE_TIMEOUT_MS: u32 = 100;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DeviceInfo {
    pub node_path: String,
    pub guid: Option<String>,
    pub family: Option<crate::families::DeviceFamily>,
    pub vendor_id: Option<u32>,
    pub model_id: Option<u32>,
    pub vendor_name: Option<String>,
    pub model_name: Option<String>,
    pub nickname: Option<String>,
    pub name: String,
    pub protocol: Option<String>,
    pub supported: bool,
    pub is_host_controller: bool,
    pub scan_error: Option<String>,
}

#[derive(Clone, Copy)]
struct HardwareProfile {
    vendor_id: u32,
    model_id: u32,
    name: &'static str,
    protocol: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
struct ConfigRomInfo {
    vendor_id: u32,
    model_id: u32,
    vendor_name: Option<String>,
    model_name: Option<String>,
}

const HARDWARE_PROFILES: &[HardwareProfile] = &[
    HardwareProfile {
        vendor_id: 0x000a92,
        model_id: 0x000011,
        name: "Presonus FireStudio Mobile",
        protocol: "DICE",
    },
    HardwareProfile {
        vendor_id: 0x000d6c,
        model_id: 0x000010,
        name: "M-Audio ProFire 2626",
        protocol: "DICE",
    },
    HardwareProfile {
        vendor_id: 0x00130e,
        model_id: 0x000012,
        name: "Focusrite Saffire PRO 26",
        protocol: "DICE",
    },
    HardwareProfile {
        vendor_id: 0x00130e,
        model_id: 0x000005,
        name: "Focusrite Saffire PRO 40",
        protocol: "DICE",
    },
    HardwareProfile {
        vendor_id: 0x00130e,
        model_id: 0x000007,
        name: "Focusrite Saffire PRO 40",
        protocol: "DICE",
    },
    HardwareProfile {
        vendor_id: 0x00130e,
        model_id: 0x0000de,
        name: "Focusrite Saffire PRO 40",
        protocol: "DICE",
    },
    HardwareProfile {
        vendor_id: 0x00130e,
        model_id: 0x000003,
        name: "Focusrite Saffire PRO 26 I/O",
        protocol: "BeBoB",
    },
];

pub fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    Ok(scan_nodes()?
        .into_iter()
        .filter(|device| !device.is_host_controller && device.scan_error.is_none())
        .collect())
}

pub fn device_info_for_path(node_path: &str) -> Result<DeviceInfo, String> {
    list_devices()?
        .into_iter()
        .find(|device| device.node_path == node_path)
        .ok_or_else(|| format!("No connected FireWire device found at {node_path}"))
}

pub fn scan_nodes() -> Result<Vec<DeviceInfo>, String> {
    let entries = fs::read_dir("/dev").map_err(|error| error.to_string())?;
    let mut device_paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_firewire_device(path))
        .collect::<Vec<_>>();
    device_paths.sort();

    let mut devices = Vec::new();
    for path in device_paths {
        devices.push(inspect_device(&path));
    }
    Ok(devices)
}

fn is_firewire_device(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("fw"))
        .is_some_and(|index| !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit()))
}

fn inspect_device(path: &Path) -> DeviceInfo {
    let node_path = path.to_string_lossy().into_owned();
    let node = FwNode::new();
    let mut device = DeviceInfo {
        node_path,
        guid: None,
        family: None,
        vendor_id: None,
        model_id: None,
        vendor_name: None,
        model_name: None,
        nickname: None,
        name: "Unknown FireWire device".to_owned(),
        protocol: None,
        supported: false,
        is_host_controller: false,
        scan_error: None,
    };

    if let Some(path) = path.to_str() {
        if let Err(error) = node.open(path, 0) {
            device.scan_error = Some(error.to_string());
            return device;
        }
    } else {
        device.scan_error = Some("Device path is not valid UTF-8".to_owned());
        return device;
    }

    device.is_host_controller = node.node_id() == node.local_node_id();

    let rom = match node.config_rom() {
        Ok(rom) => rom,
        Err(error) => {
            device.scan_error = Some(error.to_string());
            return device;
        }
    };
    let info = match parse_config_rom(rom) {
        Some(info) => info,
        None => {
            device.scan_error = Some("Config ROM has no valid root vendor/model IDs".to_owned());
            return device;
        }
    };
    device.guid = parse_guid(rom);
    let profile = HARDWARE_PROFILES
        .iter()
        .find(|profile| profile.vendor_id == info.vendor_id && profile.model_id == info.model_id);
    let nickname = match profile {
        Some(profile) if profile.name == "Focusrite Saffire PRO 40" => {
            with_glib_dispatcher(&node, || {
                crate::families::dice_eap::vendors::focusrite::read_saffire_pro40_nickname(
                    &node,
                    &FwReq::new(),
                )
                .ok()
                .or_else(|| read_dice_nickname(&node))
            })
            .flatten()
            .or_else(|| Some(profile.name.to_owned()))
        }
        Some(profile) if profile.protocol == "DICE" => {
            with_glib_dispatcher(&node, || read_dice_nickname(&node))
                .flatten()
                .or_else(|| Some(profile.name.to_owned()))
        }
        Some(profile) if profile.protocol == "BeBoB" => {
            info.model_name.clone().or_else(|| Some(profile.name.to_owned()))
        }
        Some(profile) => Some(profile.name.to_owned()),
        None => info.model_name.clone(),
    };

    device.vendor_id = Some(info.vendor_id);
    device.model_id = Some(info.model_id);
    device.vendor_name = info.vendor_name;
    device.model_name = info.model_name;
    device.nickname = nickname;
    device.supported = profile.is_some();
    if let Some(profile) = profile {
        device.name = profile.name.to_owned();
        device.protocol = Some(profile.protocol.to_owned());
    } else if let Some(name) = device.model_name.as_ref().or(device.vendor_name.as_ref()) {
        device.name = name.clone();
    }
    device.family = crate::families::driver_for_device(&device).map(|driver| driver.family());
    device
}

fn parse_guid(rom: &[u8]) -> Option<String> {
    if *rom.first()? < 4 {
        return None;
    }
    let guid: [u8; 8] = rom.get(12..20)?.try_into().ok()?;
    let guid = u64::from_be_bytes(guid);
    (guid != 0 && guid != u64::MAX).then(|| format!("{guid:016x}"))
}

fn parse_config_rom(rom: &[u8]) -> Option<ConfigRomInfo> {
    let bus_info_length = *rom.first()? as usize;
    let root_offset = 4usize.checked_add(bus_info_length.checked_mul(4)?)?;
    let header_end = root_offset.checked_add(4)?;
    let directory_header = rom.get(root_offset..header_end)?;
    let entry_count = u16::from_be_bytes([directory_header[0], directory_header[1]]) as usize;
    let entries_end = header_end.checked_add(entry_count.checked_mul(4)?)?;
    let entries = rom.get(header_end..entries_end)?;

    let mut vendor_id = None;
    let mut model_id = None;
    let mut descriptors = Vec::new();
    for (index, entry) in entries.chunks_exact(4).enumerate() {
        let key_type = entry[0] >> 6;
        let key_id = entry[0] & 0x3f;
        let value = u32::from_be_bytes([0, entry[1], entry[2], entry[3]]);
        if key_type == 0 {
            match key_id {
                0x03 => vendor_id = Some(value),
                0x17 => model_id = Some(value),
                _ => {}
            }
        } else if key_type == 2 && key_id == 0x01 {
            let entry_offset = header_end.checked_add(index.checked_mul(4)?)?;
            if let Some(name) = read_text_descriptor(rom, entry_offset, value as usize) {
                descriptors.push(name);
            }
        }
    }

    let vendor_name = descriptors.first().cloned();
    let model_name = descriptors.get(1).cloned().or_else(|| vendor_name.clone());

    Some(ConfigRomInfo {
        vendor_id: vendor_id?,
        model_id: model_id?,
        vendor_name,
        model_name,
    })
}

struct DiceProtocol;

impl TcatOperation for DiceProtocol {}

impl TcatGlobalSectionSpecification for DiceProtocol {}

fn with_glib_dispatcher<T>(node: &FwNode, operation: impl FnOnce() -> T) -> Option<T> {
    let context = MainContext::new();
    let source = node.create_source().ok()?;
    source.attach(Some(&context));

    let main_loop = Arc::new(MainLoop::new(Some(&context), false));
    let dispatch_loop = Arc::clone(&main_loop);
    let dispatcher = std::thread::Builder::new()
        .name("firemix-glib-dispatch".to_owned())
        .spawn(move || dispatch_loop.run())
        .ok()?;

    let result = operation();
    main_loop.quit();
    let _ = dispatcher.join();
    Some(result)
}

fn read_dice_nickname(node: &FwNode) -> Option<String> {
    read_dice_global_nickname(node)
}

fn read_dice_global_nickname(node: &FwNode) -> Option<String> {
    let req = FwReq::new();
    let mut sections = GeneralSections::default();
    DiceProtocol::read_general_sections(&req, node, &mut sections, DICE_TIMEOUT_MS).ok()?;

    let mut params = GlobalParameters::default();
    <DiceProtocol as TcatSectionOperation<GlobalParameters>>::whole_cache(
        &req,
        node,
        &sections.global,
        &mut params,
        DICE_TIMEOUT_MS,
    )
    .ok()?;

    let nickname = params.nickname.trim().to_owned();
    (!nickname.is_empty()).then_some(nickname)
}

fn read_text_descriptor(rom: &[u8], entry_offset: usize, relative_offset: usize) -> Option<String> {
    let block_offset = entry_offset.checked_add(relative_offset.checked_mul(4)?)?;
    let header_end = block_offset.checked_add(4)?;
    let block_header = rom.get(block_offset..header_end)?;
    let block_length = (u16::from_be_bytes([block_header[0], block_header[1]]) as usize)
        .checked_mul(4)?;
    let block_end = header_end.checked_add(block_length)?;
    let descriptor = rom.get(header_end..block_end)?;
    if descriptor.len() < 8 || descriptor[0] != 0 {
        return None;
    }

    let text_bytes = descriptor.get(8..)?;
    let text_end = text_bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(text_bytes.len());
    let text = std::str::from_utf8(&text_bytes[..text_end]).ok()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{parse_config_rom, parse_guid, HARDWARE_PROFILES};

    #[test]
    fn reads_guid_from_bus_information_block() {
        let mut rom = vec![0; 20];
        rom[0] = 4;
        rom[12..20].copy_from_slice(&0x0013_0e04_0140_8671u64.to_be_bytes());
        assert_eq!(parse_guid(&rom).as_deref(), Some("00130e0401408671"));
        rom[0] = 3;
        assert_eq!(parse_guid(&rom), None);
        assert_eq!(parse_guid(&[4]), None);
    }

    #[test]
    fn parses_vendor_and_model_from_root_directory() {
        let rom = [
            0x04, 0x04, 0x00, 0x00, // ROM header: four bus-info quadlets
            0, 0, 0, 0, // Bus-info block
            0, 0, 0, 0,
            0, 0, 0, 0,
            0, 0, 0, 0,
            0x00, 0x02, 0x00, 0x00, // Root directory: two entries
            0x03, 0x00, 0x0a, 0x92, // Vendor ID
            0x17, 0x00, 0x00, 0x11, // Model ID
        ];

        let info = parse_config_rom(&rom).expect("valid Config ROM");
        assert_eq!(info.vendor_id, 0x000a92);
        assert_eq!(info.model_id, 0x000011);
    }

    #[test]
    fn rejects_incomplete_or_non_immediate_identifiers() {
        assert_eq!(parse_config_rom(&[]), None);

        let mut rom = vec![0; 36];
        rom[0] = 4;
        rom[20..24].copy_from_slice(&[0x00, 0x02, 0x00, 0x00]);
        rom[24..28].copy_from_slice(&[0x43, 0x00, 0x0a, 0x92]);
        rom[28..32].copy_from_slice(&[0x17, 0x00, 0x00, 0x11]);

        assert_eq!(parse_config_rom(&rom), None);
    }

    #[test]
    fn reads_textual_model_name_from_root_descriptor() {
        let mut rom = vec![0; 80];
        rom[0] = 4;
        rom[20..24].copy_from_slice(&[0x00, 0x03, 0x00, 0x00]);
        rom[24..28].copy_from_slice(&[0x03, 0x00, 0x0a, 0x92]);
        rom[28..32].copy_from_slice(&[0x17, 0x00, 0x00, 0x11]);
        rom[32..36].copy_from_slice(&[0x81, 0x00, 0x00, 0x03]);
        rom[44..48].copy_from_slice(&[0x00, 0x07, 0x00, 0x00]);
        rom[48..56].copy_from_slice(&[0; 8]);
        rom[56..75].copy_from_slice(b"FireStudio Mobile\0\0");

        let info = parse_config_rom(&rom).expect("valid Config ROM");
        assert_eq!(info.model_name.as_deref(), Some("FireStudio Mobile"));
    }

    #[test]
    fn profile_table_has_unique_vendor_model_pairs() {
        for (index, profile) in HARDWARE_PROFILES.iter().enumerate() {
            assert!(!HARDWARE_PROFILES[index + 1..].iter().any(|other| {
                profile.vendor_id == other.vendor_id && profile.model_id == other.model_id
            }));
        }
    }

    #[test]
    fn matches_saffire_pro_40_config_rom_observed_on_hardware() {
        let profile = HARDWARE_PROFILES
            .iter()
            .find(|profile| profile.vendor_id == 0x00130e && profile.model_id == 0x000005);

        assert_eq!(profile.map(|profile| profile.name), Some("Focusrite Saffire PRO 40"));
    }
}