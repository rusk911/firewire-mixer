use firewire_dice_protocols::focusrite::spro40::SPro40Protocol;
use firewire_dice_protocols::tcat::extension::{ExtensionSections, TcatExtensionOperation};
use hinawa::{FwNode, FwReq};

const PRO40_NICKNAME_OFFSET: usize = 0x44;
const PRO40_NICKNAME_BYTES: usize = 16;
const TRANSACTION_TIMEOUT_MS: u32 = 200;

pub fn read_saffire_pro40_nickname(node: &FwNode, req: &FwReq) -> Result<String, String> {
	let sections = extension_sections(node, req)?;
	let mut raw = [0; PRO40_NICKNAME_BYTES];
	SPro40Protocol::read_extension(
		req,
		node,
		&sections.application,
		PRO40_NICKNAME_OFFSET,
		&mut raw,
		TRANSACTION_TIMEOUT_MS,
	)
	.map_err(|error| format!("Failed to read Focusrite nickname: {error}"))?;
	decode_nickname(&raw)
}

pub fn write_saffire_pro40_nickname(
	node: &FwNode,
	req: &FwReq,
	nickname: &str,
) -> Result<(), String> {
	if nickname.len() > PRO40_NICKNAME_BYTES {
		return Err(format!(
			"Focusrite nickname exceeds {PRO40_NICKNAME_BYTES} UTF-8 bytes"
		));
	}
	if nickname.contains('\0') {
		return Err("Nickname cannot contain a NUL character".to_owned());
	}

	let sections = extension_sections(node, req)?;
	let mut raw = [0; PRO40_NICKNAME_BYTES];
	raw[..nickname.len()].copy_from_slice(nickname.as_bytes());
	swap_quadlet_bytes(&mut raw);
	SPro40Protocol::write_extension(
		req,
		node,
		&sections.application,
		PRO40_NICKNAME_OFFSET,
		&mut raw,
		TRANSACTION_TIMEOUT_MS,
	)
	.map_err(|error| format!("Failed to write Focusrite nickname: {error}"))
}

fn extension_sections(node: &FwNode, req: &FwReq) -> Result<ExtensionSections, String> {
	let mut sections = ExtensionSections::default();
	SPro40Protocol::read_extension_sections(req, node, &mut sections, TRANSACTION_TIMEOUT_MS)
		.map_err(|error| format!("Failed to read Focusrite EAP sections: {error}"))?;
	Ok(sections)
}

fn decode_nickname(raw: &[u8; PRO40_NICKNAME_BYTES]) -> Result<String, String> {
	let mut bytes = *raw;
	swap_quadlet_bytes(&mut bytes);
	let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
	let nickname = std::str::from_utf8(&bytes[..end])
		.map_err(|error| format!("Focusrite nickname is not UTF-8: {error}"))?;
	Ok(nickname.trim().to_owned())
}

fn swap_quadlet_bytes(bytes: &mut [u8; PRO40_NICKNAME_BYTES]) {
	for quadlet in bytes.chunks_exact_mut(4) {
		quadlet.reverse();
	}
}