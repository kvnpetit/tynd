//! Pure-Rust PE/COFF resource section editor — embeds an ICO + RT_VERSION
//! block into the PE32+ binaries Tynd ships. Cross-host (no Win32 dep).
//!
//! Scope notes:
//! - Replaces the entire `.rsrc` section rather than merging — manifests, dialogs, etc. in the
//!   source binary's existing rsrc tree are dropped. The binaries we patch carry no useful
//!   pre-existing rsrc data.
//! - Strips the Authenticode certificate table (signature would be invalid after section rewrite
//!   anyway). Re-sign post-pack via `tynd-bundler codesign`.
//!
//! References:
//! - PE Format spec: <https://learn.microsoft.com/en-us/windows/win32/debug/pe-format>
//! - `resedit-js` (Apache-2.0) — blueprint for the rsrc tree builder.

use std::{fmt, io::Cursor};

use anyhow::{bail, Context, Result};
use error_stack::{Report, ResultExt as _};
use ico::IconDir;

/// `error-stack` context markers for the PE patcher. These are surfaced
/// at the entry points (`embed_icon_and_version`, `patch_subsystem_gui`)
/// so the chain captures which patching phase failed (resource builder
/// vs PE rebuild vs subsystem flip) plus the input sizes — far more useful
/// than the current bare `anyhow!` strings when debugging cross-host PE
/// failures from the calling `pack.rs`.
#[derive(Debug)]
pub(crate) enum PeContext {
    ParseIco,
    RebuildPe,
    PatchSubsystemFlag,
}

impl fmt::Display for PeContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParseIco => f.write_str("parse ICO directory"),
            Self::RebuildPe => f.write_str("rebuild PE with new .rsrc section"),
            Self::PatchSubsystemFlag => f.write_str("patch optional-header Subsystem flag"),
        }
    }
}

impl std::error::Error for PeContext {}

/// PE/COFF spec §3.4.2 — suppresses the console host window.
pub(crate) const SUBSYSTEM_WINDOWS_GUI: u16 = 2;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
const RT_VERSION: u32 = 16;
const RT_MANIFEST: u32 = 24;
/// `CREATEPROCESS_MANIFEST_RESOURCE_ID`: the manifest the loader applies to an `.exe`.
const PROCESS_MANIFEST_ID: u32 = 1;

/// Each ICO directory entry becomes one `RT_ICON/<id>` resource (1-indexed);
/// the `RT_GROUP_ICON` payload references them in order.
pub(crate) struct IconResources {
    pub(crate) icons: Vec<(u16, Vec<u8>)>,
    pub(crate) group: Vec<u8>,
}

impl IconResources {
    pub(crate) fn from_ico_bytes(ico_bytes: &[u8]) -> Result<Self> {
        let dir = IconDir::read(Cursor::new(ico_bytes)).context("parse ICO directory")?;
        let entries = dir.entries();
        if entries.is_empty() {
            bail!("ICO has zero entries");
        }

        let count = entries.len();
        let mut icons = Vec::with_capacity(count);
        // GRPICONDIR: reserved(u16) + type(u16) + count(u16) + count×GRPICONDIRENTRY.
        // GRPICONDIRENTRY is 14B (2 less than ICONDIRENTRY — 2-byte id replaces
        // the 4-byte offset).
        let mut group = Vec::with_capacity(6 + count * 14);
        group.extend_from_slice(&[0, 0]);
        group.extend_from_slice(&1u16.to_le_bytes()); // type: icon
        group.extend_from_slice(&(count as u16).to_le_bytes());

        for (i, entry) in entries.iter().enumerate() {
            let icon_id = (i as u16) + 1;
            let payload = entry.data().to_vec();

            // ICO encodes 256-px sides as `0` (u8); the `ico` crate returns u32,
            // so re-clamp on the way out.
            let width_u8 = u8::try_from(entry.width()).unwrap_or(0);
            let height_u8 = u8::try_from(entry.height()).unwrap_or(0);
            let bitcount = entry.bits_per_pixel();
            // num_colors: 0 for true-color (≥8bpp), 1<<bitcount otherwise.
            let num_colors = if bitcount >= 8 { 0u8 } else { 1u8 << bitcount };
            group.push(width_u8);
            group.push(height_u8);
            group.push(num_colors);
            group.push(0);
            group.extend_from_slice(&1u16.to_le_bytes()); // planes
            group.extend_from_slice(&bitcount.to_le_bytes());
            group.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            group.extend_from_slice(&icon_id.to_le_bytes());

            icons.push((icon_id, payload));
        }

        Ok(Self { icons, group })
    }
}

/// UTF-16LE `RT_VERSION` block (`VS_VERSION_INFO`) — single en-US (0x0409)
/// StringFileInfo with the four properties Task Manager reads.
/// `VarFileInfo` is omitted; Windows falls back to 0x0409 when missing.
pub(crate) fn build_version_block(
    app_name: &str,
    version: &str,
    file_desc: Option<&str>,
) -> Vec<u8> {
    // "1.2.3.4" → 4×u16; missing components default to 0.
    let mut parts = [0u16; 4];
    for (i, p) in version.split('.').take(4).enumerate() {
        parts[i] = p.parse().unwrap_or(0);
    }
    let file_version_ms = (u32::from(parts[0]) << 16) | u32::from(parts[1]);
    let file_version_ls = (u32::from(parts[2]) << 16) | u32::from(parts[3]);

    let strings = [
        ("FileDescription", file_desc.unwrap_or(app_name)),
        ("ProductName", app_name),
        ("InternalName", app_name),
        ("OriginalFilename", &format!("{app_name}.exe")),
        ("FileVersion", version),
        ("ProductVersion", version),
    ];

    // StringTable key "040904B0" — en-US + Unicode codepage.
    let mut strings_buf = Vec::new();
    for (k, v) in &strings {
        write_version_string(&mut strings_buf, k, v);
    }
    let mut string_table = Vec::new();
    write_version_node(
        &mut string_table,
        "040904B0",
        &strings_buf,
        /* text= */ true,
    );

    let mut string_file_info = Vec::new();
    write_version_node(
        &mut string_file_info,
        "StringFileInfo",
        &string_table,
        /* text= */ true,
    );

    // VS_FIXEDFILEINFO: 13×u32 = 52 bytes.
    let mut fixed = Vec::with_capacity(52);
    fixed.extend_from_slice(&0xFEEF_04BDu32.to_le_bytes()); // dwSignature
    fixed.extend_from_slice(&0x0001_0000u32.to_le_bytes()); // dwStrucVersion
    fixed.extend_from_slice(&file_version_ms.to_le_bytes());
    fixed.extend_from_slice(&file_version_ls.to_le_bytes());
    fixed.extend_from_slice(&file_version_ms.to_le_bytes()); // ProductVersion = FileVersion
    fixed.extend_from_slice(&file_version_ls.to_le_bytes());
    fixed.extend_from_slice(&0u32.to_le_bytes());
    fixed.extend_from_slice(&0u32.to_le_bytes());
    fixed.extend_from_slice(&0x4u32.to_le_bytes()); // VOS_NT_WINDOWS32
    fixed.extend_from_slice(&0x1u32.to_le_bytes()); // VFT_APP
    fixed.extend_from_slice(&0u32.to_le_bytes());
    fixed.extend_from_slice(&0u32.to_le_bytes());
    fixed.extend_from_slice(&0u32.to_le_bytes());

    let mut payload = fixed;
    payload.extend_from_slice(&string_file_info);
    let mut out = Vec::new();
    write_version_node(
        &mut out,
        "VS_VERSION_INFO",
        &payload,
        /* text= */ false,
    );
    out
}

/// `text=true` → type byte 1 (text); `false` → 0 (binary). For binary
/// `VS_VERSION_INFO` we pre-fill `wValueLength = sizeof(VS_FIXEDFILEINFO)`.
fn write_version_node(out: &mut Vec<u8>, key: &str, children: &[u8], text: bool) {
    let start = out.len();
    out.extend_from_slice(&[0, 0]); // wLength placeholder
    let value_length = if text {
        0u16
    } else if key == "VS_VERSION_INFO" {
        52u16
    } else {
        0u16
    };
    out.extend_from_slice(&value_length.to_le_bytes());
    out.extend_from_slice(&u16::from(text).to_le_bytes()); // wType
    write_utf16_padded(out, key);
    out.extend_from_slice(children);
    let len = (out.len() - start) as u16;
    out[start..start + 2].copy_from_slice(&len.to_le_bytes());
    pad_to_dword(out);
}

fn write_version_string(out: &mut Vec<u8>, key: &str, value: &str) {
    let start = out.len();
    let value_chars = value.encode_utf16().count() as u16 + 1; // +1 NUL
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&value_chars.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // wType = text
    write_utf16_padded(out, key);
    write_utf16_padded(out, value);
    let len = (out.len() - start) as u16;
    out[start..start + 2].copy_from_slice(&len.to_le_bytes());
    pad_to_dword(out);
}

fn write_utf16_padded(out: &mut Vec<u8>, s: &str) {
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]); // NUL
    pad_to_dword(out);
}

fn pad_to_dword(out: &mut Vec<u8>) {
    out.resize((out.len() + 3) & !3, 0);
}

// PE patching

/// Flip Optional Header Subsystem to GUI in place. No-op if already GUI.
pub(crate) fn patch_subsystem_gui(bytes: &mut [u8]) -> Result<()> {
    let total_len = bytes.len();
    let pe_off = read_pe_offset(bytes)
        .map_err(|e| Report::new(PeContext::PatchSubsystemFlag).attach(format!("source = {e:#}")))
        .attach_with(move || format!("pe_size = {total_len}"))
        .map_err(|report| anyhow::anyhow!("{report:?}"))?;
    // PE sig (4) + COFF header (20) = 24 → optional header starts at +24.
    let opt_off = pe_off + 24;
    // Subsystem offset is +68 in both PE32 and PE32+.
    let subsystem_off = opt_off + 68;
    let cur = u16::from_le_bytes([bytes[subsystem_off], bytes[subsystem_off + 1]]);
    if cur == SUBSYSTEM_WINDOWS_GUI {
        return Ok(());
    }
    bytes[subsystem_off..subsystem_off + 2].copy_from_slice(&SUBSYSTEM_WINDOWS_GUI.to_le_bytes());
    Ok(())
}

/// Application manifest carrying only `requestedExecutionLevel` (UAC) and the
/// Windows 10/11 `supportedOS` GUID. DPI awareness stays with the runtime, which
/// sets it programmatically.
pub(crate) fn application_manifest(level: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="{level}" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}}"/>
    </application>
  </compatibility>
</assembly>
"#
    )
    .into_bytes()
}

/// Replaces (does not merge) the `.rsrc` section. Returns the new PE bytes.
pub(crate) fn embed_icon_and_version(
    pe: Vec<u8>,
    ico_bytes: &[u8],
    version_block: Option<&[u8]>,
    manifest: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let pe_size = pe.len();
    let ico_size = ico_bytes.len();
    let version_size = version_block.map_or(0, <[u8]>::len);

    let icons = IconResources::from_ico_bytes(ico_bytes)
        .map_err(|e| Report::new(PeContext::ParseIco).attach(format!("source = {e:#}")))
        .attach_with(move || format!("ico_size = {ico_size}"))
        .map_err(|report| anyhow::anyhow!("{report:?}"))?;
    let icon_count = icons.icons.len();
    let mut builder = ResourceBuilder::new();
    for (id, data) in &icons.icons {
        builder.add(RT_ICON, u32::from(*id), data.clone());
    }
    builder.add(RT_GROUP_ICON, 1, icons.group.clone());
    if let Some(v) = version_block {
        builder.add(RT_VERSION, 1, v.to_vec());
    }
    if let Some(manifest) = manifest {
        builder.add(RT_MANIFEST, PROCESS_MANIFEST_ID, manifest.to_vec());
    }
    rebuild_pe_with_rsrc(pe, &builder)
        .map_err(|e| Report::new(PeContext::RebuildPe).attach(format!("source = {e:#}")))
        .attach_with(move || format!("pe_size = {pe_size}"))
        .attach_with(move || format!("icon_count = {icon_count}"))
        .attach_with(move || format!("version_block_size = {version_size}"))
        .map_err(|report| anyhow::anyhow!("{report:?}"))
}

// PE rebuild (rsrc replacement)

#[derive(Default)]
struct ResourceBuilder {
    /// Flat list, sorted into a tree at write time.
    entries: Vec<(u32, u32, Vec<u8>)>,
}

impl ResourceBuilder {
    fn new() -> Self {
        Self::default()
    }
    fn add(&mut self, type_id: u32, name_id: u32, data: Vec<u8>) {
        self.entries.push((type_id, name_id, data));
    }
}

fn read_pe_offset(bytes: &[u8]) -> Result<usize> {
    if bytes.len() < 0x40 {
        bail!("PE: input too small");
    }
    if &bytes[0..2] != b"MZ" {
        bail!("PE: not a DOS/PE binary (no MZ magic)");
    }
    let pe_off = u32::from_le_bytes([bytes[0x3c], bytes[0x3d], bytes[0x3e], bytes[0x3f]]) as usize;
    if pe_off + 24 > bytes.len() || &bytes[pe_off..pe_off + 4] != b"PE\0\0" {
        bail!("PE: invalid PE signature at offset {pe_off:#x}");
    }
    Ok(pe_off)
}

const SECTION_HEADER_LEN: usize = 40;

fn read_u32(bytes: &[u8], off: usize, what: &'static str) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes[off..off + 4].try_into().context(what)?,
    ))
}

/// Returns `(section_header_off, raw_off, raw_size)` of the `.rsrc` section.
fn find_rsrc_section(
    bytes: &[u8],
    num_sections: usize,
    section_table_off: usize,
) -> Option<(usize, u32, u32)> {
    let off = (0..num_sections)
        .map(|i| section_table_off + i * SECTION_HEADER_LEN)
        .find(|&off| bytes[off..off + 8].starts_with(b".rsrc"))?;
    let raw_size = u32::from_le_bytes(bytes[off + 16..off + 20].try_into().ok()?);
    let raw_off = u32::from_le_bytes(bytes[off + 20..off + 24].try_into().ok()?);
    Some((off, raw_off, raw_size))
}

/// Zero the existing `.rsrc` section header and its raw data, if present.
fn drop_rsrc_section(pe: &mut [u8], num_sections: usize, section_table_off: usize) {
    let Some((sec_off, raw_off, raw_size)) = find_rsrc_section(pe, num_sections, section_table_off)
    else {
        return;
    };
    pe[sec_off..sec_off + SECTION_HEADER_LEN].fill(0);
    if (raw_off as usize) < pe.len() {
        let end = (raw_off + raw_size).min(pe.len() as u32) as usize;
        pe[raw_off as usize..end].fill(0);
    }
}

/// First all-zero section header slot (the one `drop_rsrc_section` cleared, if any).
fn free_section_slot(pe: &[u8], num_sections: usize, section_table_off: usize) -> Option<usize> {
    (0..num_sections)
        .map(|i| section_table_off + i * SECTION_HEADER_LEN)
        .find(|&off| pe[off..off + 8].iter().all(|&b| b == 0))
}

/// 40-byte `.rsrc` section header.
fn rsrc_section_header(
    virtual_size: u32,
    virtual_addr: u32,
    raw_size: u32,
    raw_off: u32,
) -> Vec<u8> {
    let mut sh = Vec::with_capacity(SECTION_HEADER_LEN);
    sh.extend_from_slice(b".rsrc\0\0\0");
    sh.extend_from_slice(&virtual_size.to_le_bytes());
    sh.extend_from_slice(&virtual_addr.to_le_bytes());
    sh.extend_from_slice(&raw_size.to_le_bytes());
    sh.extend_from_slice(&raw_off.to_le_bytes());
    sh.extend_from_slice(&[0u8; 12]);
    // IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ
    sh.extend_from_slice(&0x4000_0040u32.to_le_bytes());
    sh
}

/// New section is appended at file end so the section table doesn't move.
fn rebuild_pe_with_rsrc(mut pe: Vec<u8>, builder: &ResourceBuilder) -> Result<Vec<u8>> {
    let pe_off = read_pe_offset(&pe)?;
    let coff_off = pe_off + 4;
    let num_sections = u16::from_le_bytes([pe[coff_off + 2], pe[coff_off + 3]]) as usize;
    let opt_size = u16::from_le_bytes([pe[coff_off + 16], pe[coff_off + 17]]) as usize;
    let opt_off = coff_off + 20;
    let section_table_off = opt_off + opt_size;

    // PE32 vs PE32+ determines the data-directory offset.
    let opt_magic = u16::from_le_bytes([pe[opt_off], pe[opt_off + 1]]);
    let is_pe32_plus = opt_magic == 0x20b;
    let dd_off = if is_pe32_plus {
        opt_off + 112
    } else {
        opt_off + 96
    };

    let section_alignment = read_u32(&pe, opt_off + 32, "section_alignment")?;
    let file_alignment = read_u32(&pe, opt_off + 36, "file_alignment")?;
    if file_alignment == 0 || section_alignment == 0 {
        bail!("PE: zero alignment");
    }

    // Append: new VA = align_up(image_size, section_alignment),
    // raw offset = align_up(file_size, file_alignment).
    let size_of_image_off = opt_off + 56;
    let size_of_image = read_u32(&pe, size_of_image_off, "SizeOfImage")?;

    drop_rsrc_section(&mut pe, num_sections, section_table_off);

    // Strip cert table (data directory 4) — Authenticode would no longer match.
    let cert_off = dd_off + 4 * 8;
    if cert_off + 8 <= pe.len() {
        pe[cert_off..cert_off + 8].fill(0);
    }

    // Compute final VA up front so DataEntry RVAs are correct.
    let new_section_va = align_up(size_of_image, section_alignment);
    let mut rsrc = build_rsrc_blob(builder, new_section_va);
    let rsrc_virtual_size = rsrc.len() as u32;
    let rsrc_raw_size = align_up(rsrc_virtual_size, file_alignment);
    rsrc.resize(rsrc_raw_size as usize, 0);

    let pad_to = align_up(pe.len() as u32, file_alignment) as usize;
    pe.resize(pad_to, 0);
    let new_raw_off = pe.len() as u32;
    pe.extend_from_slice(&rsrc);

    let Some(sec_off) = free_section_slot(&pe, num_sections, section_table_off) else {
        // Bumping NumberOfSections + SizeOfHeaders is complex and unneeded —
        // the binaries we patch always have a free slot.
        bail!(
            "PE: no free section table slot to write .rsrc (NumberOfSections={num_sections}). \
             Re-link with reserved space or rebuild with the existing .rsrc kept."
        );
    };
    let header = rsrc_section_header(
        rsrc_virtual_size,
        new_section_va,
        rsrc_raw_size,
        new_raw_off,
    );
    pe[sec_off..sec_off + SECTION_HEADER_LEN].copy_from_slice(&header);

    let new_size_of_image = align_up(new_section_va + rsrc_virtual_size, section_alignment);
    pe[size_of_image_off..size_of_image_off + 4].copy_from_slice(&new_size_of_image.to_le_bytes());

    // Resource data directory (index 2): VA + size.
    let res_dd_off = dd_off + 2 * 8;
    pe[res_dd_off..res_dd_off + 4].copy_from_slice(&new_section_va.to_le_bytes());
    pe[res_dd_off + 4..res_dd_off + 8].copy_from_slice(&rsrc_virtual_size.to_le_bytes());

    Ok(pe)
}

fn align_up(v: u32, align: u32) -> u32 {
    if align == 0 {
        return v;
    }
    let r = v % align;
    if r == 0 {
        v
    } else {
        v + (align - r)
    }
}

/// Characteristics(u32) + TimeDateStamp(u32) + MajorVersion(u16) +
/// MinorVersion(u16) + n_named(u16) + n_id(u16). All other bytes stay 0.
fn write_dir_hdr(buf: &mut [u8], off: usize, n_named: u16, n_id: u16) {
    buf[off + 12..off + 14].copy_from_slice(&n_named.to_le_bytes());
    buf[off + 14..off + 16].copy_from_slice(&n_id.to_le_bytes());
}

/// Directory entry: `id` + subdirectory offset (high bit set → subdirectory).
fn write_subdir_entry(buf: &mut [u8], off: usize, id: u32, subdir_off: usize) {
    buf[off..off + 4].copy_from_slice(&id.to_le_bytes());
    let sub_off = subdir_off as u32 | 0x8000_0000;
    buf[off + 4..off + 8].copy_from_slice(&sub_off.to_le_bytes());
}

/// `base_rva` is the section's load address — each
/// `IMAGE_RESOURCE_DATA_ENTRY.OffsetToData` is RVA-relative.
///
/// Layout:
/// ```text
///   [root dir hdr 16]   [root entries 8 × n_types]
///   for each type:
///     [type dir hdr 16] [name entries 8 × n_names]
///     for each name:
///       [name dir hdr 16] [lang entry 8]
///   [data entries 16 × total_names]
///   [data blobs (4-byte aligned)]
/// ```
fn build_rsrc_blob(builder: &ResourceBuilder, base_rva: u32) -> Vec<u8> {
    use std::collections::BTreeMap;
    let mut tree: BTreeMap<u32, BTreeMap<u32, Vec<u8>>> = BTreeMap::new();
    for (t, n, d) in &builder.entries {
        tree.entry(*t).or_default().insert(*n, d.clone());
    }

    let n_types = tree.len();
    let total_names: usize = tree.values().map(BTreeMap::len).sum();
    let entries_off_root = 16usize;
    let mut cursor = entries_off_root + 8 * n_types;
    let mut type_dir_offs: Vec<usize> = Vec::with_capacity(n_types);
    let mut name_dir_offs: Vec<Vec<usize>> = Vec::with_capacity(n_types);
    for names in tree.values() {
        type_dir_offs.push(cursor);
        cursor += 16 + 8 * names.len();
        let mut row = Vec::with_capacity(names.len());
        for _ in names {
            row.push(cursor);
            cursor += 16 + 8; // name dir hdr + lang entry
        }
        name_dir_offs.push(row);
    }
    let data_entries_off = cursor;
    cursor += 16 * total_names;
    let mut blob_offs: Vec<usize> = Vec::with_capacity(total_names);
    for blob in tree.values().flat_map(BTreeMap::values) {
        blob_offs.push(cursor);
        cursor = (cursor + blob.len()).next_multiple_of(4);
    }
    let mut buf = vec![0u8; cursor];

    write_dir_hdr(&mut buf, 0, 0, n_types as u16);
    let mut data_idx = 0usize;
    for (i, (type_id, names)) in tree.iter().enumerate() {
        write_subdir_entry(
            &mut buf,
            entries_off_root + i * 8,
            *type_id,
            type_dir_offs[i],
        );

        write_dir_hdr(&mut buf, type_dir_offs[i], 0, names.len() as u16);
        let names_entries_off = type_dir_offs[i] + 16;
        for (j, (name_id, blob)) in names.iter().enumerate() {
            let name_dir_off = name_dir_offs[i][j];
            write_subdir_entry(&mut buf, names_entries_off + j * 8, *name_id, name_dir_off);

            write_dir_hdr(&mut buf, name_dir_off, 0, 1);
            // Lang id 0x0409 (en-US).
            let lang_off = name_dir_off + 16;
            buf[lang_off..lang_off + 4].copy_from_slice(&0x0409u32.to_le_bytes());
            let de_off = data_entries_off + data_idx * 16;
            buf[lang_off + 4..lang_off + 8].copy_from_slice(&(de_off as u32).to_le_bytes());

            // Data entry: OffsetToData(RVA) + Size + CodePage(0) + Reserved(0).
            let blob_off = blob_offs[data_idx];
            let rva = base_rva + blob_off as u32;
            buf[de_off..de_off + 4].copy_from_slice(&rva.to_le_bytes());
            buf[de_off + 4..de_off + 8].copy_from_slice(&(blob.len() as u32).to_le_bytes());

            buf[blob_off..blob_off + blob.len()].copy_from_slice(blob);
            data_idx += 1;
        }
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_manifest_requests_the_level() {
        let manifest = String::from_utf8(application_manifest("requireAdministrator")).unwrap();
        assert!(manifest.contains(r#"<requestedExecutionLevel level="requireAdministrator""#));
        assert!(manifest.contains("{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"));
        assert!(manifest.starts_with("<?xml"));
    }
}
