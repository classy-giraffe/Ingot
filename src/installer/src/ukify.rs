//! UKI validation: parse the PE sections of a Unified Kernel Image
//! and check its kernel command line and os-release against the
//! install config.
//!
//! The UKI artifact (`ingot_<v>.efi`) is built at release time; the
//! installer never builds UKIs. What the install MUST verify is that
//! the artifact it deploys carries the command line that will boot
//! THIS layout: `usr=PARTUUID=<slot A>` and `root=PARTUUID=<state>`,
//! plus an os-release whose VERSION_ID matches the installed release.
//! A mismatch means the firmware will boot into a wrong or absent
//! partition, so this is a hard validation-phase gate.

/// The parsed UKI section contents that matter to the installer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UkiSections {
    /// `.cmdline` section (the kernel command line).
    pub cmdline: String,
    /// `.osrel` section (the os-release of the kernel release).
    pub osrel: String,
}

/// Parses the PE structure of a UKI and extracts the `.cmdline` and
/// `.osrel` section contents.
///
/// Only the PE/COFF parts a UKI is guaranteed to have are read: the
/// DOS header (`MZ`, `e_lfanew`), the COFF header (machine, section
/// count, optional-header size), and the section headers
/// (name, raw pointer, raw size). The PE optional header is skipped
/// by its declared size and never interpreted.
pub fn parse_sections(bytes: &mut [u8]) -> Result<UkiSections, String> {
    if bytes.len() < 0x40 || bytes[0] != b'M' || bytes[1] != b'Z' {
        return Err("not a PE image (missing MZ header)".into());
    }
    let pe_off = u32::from_le_bytes([bytes[0x3C], bytes[0x3D], bytes[0x3E], bytes[0x3F]]) as usize;
    if pe_off + 24 > bytes.len() {
        return Err("PE header out of bounds".into());
    }
    if &bytes[pe_off..pe_off + 4] != b"PE\0\0" {
        return Err("not a PE image (missing PE\\0\\0 signature)".into());
    }
    let machine = u16::from_le_bytes([bytes[pe_off + 4], bytes[pe_off + 5]]);
    if machine != 0x8664 {
        return Err(format!("UKI is not x86_64 (machine type 0x{machine:04x})"));
    }
    let nsec = u16::from_le_bytes([bytes[pe_off + 6], bytes[pe_off + 7]]) as usize;
    if nsec == 0 {
        return Err("PE image has no sections".into());
    }
    let opt_size = u16::from_le_bytes([bytes[pe_off + 20], bytes[pe_off + 21]]) as usize;
    let sec_off = pe_off + 24 + opt_size;
    if sec_off + 40 * nsec > bytes.len() {
        return Err("PE section table out of bounds".into());
    }

    let mut cmdline: Option<String> = None;
    let mut osrel: Option<String> = None;
    for i in 0..nsec {
        let s = sec_off + 40 * i;
        let name = section_name(&bytes[s..s + 8]);
        let raw_size =
            u32::from_le_bytes([bytes[s + 16], bytes[s + 17], bytes[s + 18], bytes[s + 19]])
                as usize;
        let raw_ptr =
            u32::from_le_bytes([bytes[s + 20], bytes[s + 21], bytes[s + 22], bytes[s + 23]])
                as usize;
        if raw_ptr + raw_size > bytes.len() {
            return Err(format!("section {name} raw data out of bounds"));
        }
        let data = &bytes[raw_ptr..raw_ptr + raw_size];
        let text = strip_trailing_nul(data);
        let text = std::str::from_utf8(text)
            .map_err(|_| format!("section {name} is not valid UTF-8"))?
            .to_string();
        match name.as_str() {
            ".cmdline" if cmdline.is_none() => cmdline = Some(text),
            ".osrel" if osrel.is_none() => osrel = Some(text),
            _ => {}
        }
    }
    match (cmdline, osrel) {
        (Some(cmdline), Some(osrel)) => Ok(UkiSections { cmdline, osrel }),
        (None, _) => Err("UKI is missing its .cmdline section".into()),
        (_, None) => Err("UKI is missing its .osrel section".into()),
    }
}

/// Validates a UKI against the install it will serve.
///
/// `slot_a_uuid` and `state_uuid` are the fixed PARTUUIDs the layout
/// assigns; the UKI's command line must point at them exactly.
/// `version` must appear as the os-release VERSION_ID.
pub fn validate_uki(
    bytes: &mut [u8],
    version: &str,
    slot_a_uuid: &str,
    state_uuid: &str,
) -> Result<UkiSections, String> {
    let sections = parse_sections(bytes)?;
    let usr = format!("usr=PARTUUID={slot_a_uuid}");
    let root = format!("root=PARTUUID={state_uuid}");
    // word-boundary check: the token must appear as a whole command
    // line word, not inside a longer token
    for (what, token) in [("usr", &usr), ("root", &root)] {
        if !sections
            .cmdline
            .split_whitespace()
            .any(|w| w == token.as_str())
        {
            return Err(format!(
                "UKI command line does not set {what} to PARTUUID {token} (this UKI will not boot this layout)"
            ));
        }
    }
    let version_id = format!("VERSION_ID=\"{version}\"");
    if !sections.osrel.contains(&version_id) {
        return Err(format!("UKI os-release does not carry {version_id}"));
    }
    Ok(sections)
}

/// Reads a NUL-padded 8-byte PE section name.
fn section_name(raw: &[u8]) -> String {
    raw.iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}

/// Trims trailing NUL bytes from section data.
fn strip_trailing_nul(data: &[u8]) -> &[u8] {
    let end = data
        .iter()
        .rposition(|&b| b != 0)
        .map(|i| i + 1)
        .unwrap_or(0);
    &data[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal synthetic PE (x86_64, zero optional header)
    /// with the given sections: (name, bytes) pairs.
    fn pe(sections: &[(&str, &[u8])]) -> Vec<u8> {
        let header_end = 0x58 + 40 * sections.len(); // COFF header (24 bytes) ends at 0x58
        let mut out = vec![0u8; header_end];
        out[0] = b'M';
        out[1] = b'Z';
        let pe_off: u32 = 0x40;
        out[0x3C..0x40].copy_from_slice(&pe_off.to_le_bytes());
        out[0x40..0x44].copy_from_slice(b"PE\0\0");
        out[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
        out[0x46..0x48].copy_from_slice(&(sections.len() as u16).to_le_bytes());
        // SizeOfOptionalHeader (0x48..0x4A) stays 0
        let mut data_off = header_end;
        for (i, (name, data)) in sections.iter().enumerate() {
            let s = 0x58 + 40 * i;
            let name_bytes: Vec<u8> = name.bytes().take(8).collect();
            out[s..s + name_bytes.len()].copy_from_slice(&name_bytes);
            let size = data.len() as u32;
            out[s + 16..s + 20].copy_from_slice(&size.to_le_bytes());
            out[s + 20..s + 24].copy_from_slice(&(data_off as u32).to_le_bytes());
            data_off += data.len();
        }
        for (_, data) in sections {
            out.extend_from_slice(data);
        }
        out
    }

    const CMDLINE: &[u8] =
        b"usr=PARTUUID=0066bfe5-47f1-52dc-9a16-1bb10191a1dc root=PARTUUID=501347aa-775a-5736-8da3-2a9977c820ec";
    const OSREL: &[u8] = b"NAME=Ingot\nVERSION_ID=\"0.1.0\"\nID=ingot\n";

    #[test]
    fn parses_cmdline_and_osrel() {
        let mut img = pe(&[(".cmdline", CMDLINE), (".osrel", OSREL)]);
        let s = parse_sections(&mut img).unwrap();
        assert_eq!(s.cmdline, std::str::from_utf8(CMDLINE).unwrap());
        assert_eq!(s.osrel, std::str::from_utf8(OSREL).unwrap());
    }

    #[test]
    fn rejects_non_pe() {
        let mut junk = vec![0u8; 256];
        assert!(parse_sections(&mut junk).unwrap_err().contains("MZ"));
    }

    #[test]
    fn rejects_wrong_machine() {
        let mut img = pe(&[(".cmdline", CMDLINE), (".osrel", OSREL)]);
        img[0x44..0x46].copy_from_slice(&0x0100u16.to_le_bytes()); // i386
        assert!(parse_sections(&mut img).unwrap_err().contains("x86_64"));
    }

    #[test]
    fn rejects_missing_section() {
        let mut img = pe(&[(".cmdline", CMDLINE)]);
        assert!(parse_sections(&mut img).unwrap_err().contains(".osrel"));
        let mut img = pe(&[(".osrel", OSREL)]);
        assert!(parse_sections(&mut img).unwrap_err().contains(".cmdline"));
    }

    #[test]
    fn validates_matching_uki() {
        let mut img = pe(&[(".cmdline", CMDLINE), (".osrel", OSREL)]);
        let s = validate_uki(
            &mut img,
            "0.1.0",
            "0066bfe5-47f1-52dc-9a16-1bb10191a1dc",
            "501347aa-775a-5736-8da3-2a9977c820ec",
        )
        .unwrap();
        assert_eq!(s.osrel, std::str::from_utf8(OSREL).unwrap());
    }

    #[test]
    fn rejects_wrong_partuuid() {
        let mut img = pe(&[(".cmdline", CMDLINE), (".osrel", OSREL)]);
        let err = validate_uki(
            &mut img,
            "0.1.0",
            "deadbeef-dead-beef-dead-beefdeadbeef",
            "501347aa-775a-5736-8da3-2a9977c820ec",
        )
        .unwrap_err();
        assert!(err.contains("usr"), "{err}");

        let mut img = pe(&[(".cmdline", CMDLINE), (".osrel", OSREL)]);
        let err = validate_uki(
            &mut img,
            "0.1.0",
            "0066bfe5-47f1-52dc-9a16-1bb10191a1dc",
            "deadbeef-dead-beef-dead-beefdeadbeef",
        )
        .unwrap_err();
        assert!(err.contains("root"), "{err}");
    }

    #[test]
    fn rejects_wrong_version() {
        let mut img = pe(&[(".cmdline", CMDLINE), (".osrel", OSREL)]);
        let err = validate_uki(
            &mut img,
            "9.9.9",
            "0066bfe5-47f1-52dc-9a16-1bb10191a1dc",
            "501347aa-775a-5736-8da3-2a9977c820ec",
        )
        .unwrap_err();
        assert!(err.contains("VERSION_ID"), "{err}");
    }
}
