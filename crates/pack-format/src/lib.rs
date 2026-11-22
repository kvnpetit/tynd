#![forbid(unsafe_code)]

use std::{
    error::Error,
    fmt,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

pub const MAGIC: &[u8; 8] = b"TYNDPKG\0";
/// section_size(u64) + magic(8) = 16 bytes.
pub const TRAILER_LEN: u64 = 16;
/// Reserved pack entry carrying the project-relative WebView2 fixed-runtime path.
pub const WEBVIEW2_RUNTIME_PATH_ENTRY: &str = "webview2.runtime-path";
/// Reserved pack entry present only when the app was built with the debug
/// protocol allowed (`tynd build --debug-protocol`). Absent means disabled, so
/// packs written before the flag existed read as `false`.
pub const DEBUG_FLAG_ENTRY: &str = "debug.flag";
/// Maximum size of small boot metadata entries.
pub const MAX_BOOT_METADATA_BYTES: u64 = 4096;

#[derive(Debug)]
pub struct PackReader {
    file: File,
    remaining: usize,
    section_end: u64,
    pending_end: Option<u64>,
}

impl PackReader {
    /// Open the running executable and locate the packed section.
    ///
    /// Returns `None` when the executable carries no pack, and also when it
    /// carries a damaged one; use [`PackReader::open_checked`] to tell them apart.
    pub fn open() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        Self::open_path(&exe)
    }

    /// Open an explicit file path and locate the packed section.
    ///
    /// See [`PackReader::open_path_checked`] for the variant that reports a
    /// damaged pack instead of treating it as absent.
    pub fn open_path(exe: &Path) -> Option<Self> {
        Self::open_path_checked(exe).ok().flatten()
    }

    /// Open the running executable, distinguishing "no pack" (`Ok(None)`) from
    /// "damaged pack" (`Err`: the `TYNDPKG` marker was found but the section it
    /// describes is invalid).
    pub fn open_checked() -> io::Result<Option<Self>> {
        let Ok(exe) = std::env::current_exe() else {
            return Ok(None);
        };
        Self::open_path_checked(&exe)
    }

    /// Open an explicit file path, distinguishing "no pack" from "damaged pack".
    ///
    /// An unreadable file is reported as "no pack" (`Ok(None)`).
    pub fn open_path_checked(exe: &Path) -> io::Result<Option<Self>> {
        let Ok(mut f) = File::open(exe) else {
            return Ok(None);
        };
        let Ok(metadata) = f.metadata() else {
            return Ok(None);
        };
        let size = metadata.len();

        let Some(trailer_end) = locate_trailer_end(&mut f, size) else {
            return Ok(None);
        };

        let max_section = trailer_end - TRAILER_LEN;
        f.seek(SeekFrom::Start(max_section))?;
        let mut size_bytes = [0u8; 8];
        f.read_exact(&mut size_bytes)?;
        let section_size = u64::from_le_bytes(size_bytes);
        if section_size < 4 {
            return Err(invalid_data("pack section is smaller than its header"));
        }
        if section_size > max_section {
            return Err(invalid_data("pack section is larger than the executable"));
        }
        let section_start = max_section - section_size;
        f.seek(SeekFrom::Start(section_start))?;

        let mut cb = [0u8; 4];
        f.read_exact(&mut cb)?;
        let file_count = u32::from_le_bytes(cb) as usize;

        Ok(Some(Self {
            file: f,
            remaining: file_count,
            section_end: max_section,
            pending_end: None,
        }))
    }

    /// Number of entries still to read.
    pub fn remaining(&self) -> usize {
        self.remaining
    }

    /// Read the next entry header.
    pub fn next_header(&mut self) -> io::Result<Option<(String, u64)>> {
        if let Some(expected_end) = self.pending_end.take() {
            if self.file.stream_position()? != expected_end {
                return Err(invalid_data("previous pack entry was not consumed exactly"));
            }
        }

        if self.remaining == 0 {
            if self.file.stream_position()? != self.section_end {
                return Err(invalid_data(
                    "pack section contains trailing or unaccounted bytes",
                ));
            }
            return Ok(None);
        }

        self.require_section_bytes(2)?;
        let mut pl = [0u8; 2];
        self.file.read_exact(&mut pl)?;
        let path_len = u16::from_le_bytes(pl) as usize;
        let header_tail = u64::try_from(path_len)
            .ok()
            .and_then(|len| len.checked_add(4))
            .ok_or_else(|| invalid_data("pack entry header length overflow"))?;
        self.require_section_bytes(header_tail)?;

        let mut path_buf = vec![0u8; path_len];
        self.file.read_exact(&mut path_buf)?;
        let rel = String::from_utf8(path_buf)
            .map_err(|_| invalid_data("pack entry path is not valid UTF-8"))?;

        let mut dl = [0u8; 4];
        self.file.read_exact(&mut dl)?;
        let data_len = u32::from_le_bytes(dl) as u64;
        self.require_section_bytes(data_len)?;

        let entry_start = self.file.stream_position()?;
        self.pending_end = Some(
            entry_start
                .checked_add(data_len)
                .ok_or_else(|| invalid_data("pack entry length overflow"))?,
        );
        self.remaining -= 1;
        Ok(Some((rel, data_len)))
    }

    /// Copy the current entry to `writer`, requiring exact consumption.
    pub fn copy_entry_to<W: Write>(&mut self, data_len: u64, writer: &mut W) -> io::Result<u64> {
        self.require_current_entry_len(data_len)?;
        let copied = io::copy(&mut Read::by_ref(&mut self.file).take(data_len), writer)?;
        if copied != data_len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("pack entry ended after {copied} of {data_len} bytes"),
            ));
        }
        self.pending_end = None;
        Ok(copied)
    }

    /// Discard the next `data_len` bytes without allocating.
    pub fn skip(&mut self, data_len: u64) -> io::Result<()> {
        self.copy_entry_to(data_len, &mut io::sink())?;
        Ok(())
    }

    fn require_section_bytes(&mut self, required: u64) -> io::Result<()> {
        let position = self.file.stream_position()?;
        let available = self
            .section_end
            .checked_sub(position)
            .ok_or_else(|| invalid_data("pack reader advanced beyond section boundary"))?;
        if required > available {
            return Err(invalid_data(format!(
                "pack entry requires {required} bytes but only {available} remain in section"
            )));
        }
        Ok(())
    }

    fn require_current_entry_len(&mut self, data_len: u64) -> io::Result<()> {
        let position = self.file.stream_position()?;
        let expected_end = self
            .pending_end
            .ok_or_else(|| invalid_data("no current pack entry"))?;
        let remaining = expected_end
            .checked_sub(position)
            .ok_or_else(|| invalid_data("pack reader advanced beyond current entry"))?;
        if data_len != remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("pack entry copy requested {data_len} bytes but {remaining} bytes remain"),
            ));
        }
        Ok(())
    }
}

/// Find the offset one past the `TYNDPKG` magic, i.e. where the pack trailer ends.
///
/// An unsigned executable ends with the trailer, so the check at EOF is the
/// common path. Code signing breaks that: signing happens after packing (the
/// host binary must be signed in its final form, pack included), and both
/// formats put the signature at the very end of the file, after the pack:
///
/// - PE / Authenticode: the certificate table is appended at EOF and located
///   by the security data directory. The pack is part of the signed overlay, so
///   it stays valid and the trailer sits just before the table (the table
///   starts on an 8-byte boundary, so up to 7 padding bytes may separate them).
/// - Mach-O / codesign: the signature blob is located by `LC_CODE_SIGNATURE`
///   and the trailer sits just before it (16-byte alignment padding).
///
/// Only these structured locations are probed; the file is never scanned
/// blindly for the magic, so a stray `TYNDPKG` string cannot be mistaken for a pack.
fn locate_trailer_end(f: &mut File, size: u64) -> Option<u64> {
    if magic_ends_at(f, size) {
        return Some(size);
    }
    let signature_start = pe_certificate_table_start(f, size)
        .map(|start| (start, 8))
        .or_else(|| macho_code_signature_start(f, size).map(|start| (start, 16)))?;
    let (start, alignment) = signature_start;
    (0..alignment)
        .filter_map(|padding| start.checked_sub(padding))
        .find(|&end| magic_ends_at(f, end))
}

fn read_exact_at(f: &mut File, offset: u64, buf: &mut [u8]) -> Option<()> {
    f.seek(SeekFrom::Start(offset)).ok()?;
    f.read_exact(buf).ok()
}

/// Whether a complete trailer (`section_size` + magic) ends exactly at `end`.
fn magic_ends_at(f: &mut File, end: u64) -> bool {
    if end < TRAILER_LEN {
        return false;
    }
    let mut magic = [0u8; 8];
    read_exact_at(f, end - 8, &mut magic).is_some() && &magic == MAGIC
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// File offset of the Authenticode certificate table of a PE image, if any.
fn pe_certificate_table_start(f: &mut File, size: u64) -> Option<u64> {
    let mut dos = [0u8; 0x40];
    read_exact_at(f, 0, &mut dos)?;
    if &dos[..2] != b"MZ" {
        return None;
    }
    let pe_offset = u64::from(u32_at(&dos, 0x3c)?);
    // PE signature (4) + COFF header (20) + optional header up to the data directories.
    let mut header = [0u8; 4 + 20 + 112 + 8 * 5];
    read_exact_at(f, pe_offset, &mut header)?;
    if header[..4] != [b'P', b'E', 0, 0] {
        return None;
    }
    let optional = 4 + 20;
    let (directory_count_at, directories_at) = match u16_at(&header, optional)? {
        0x10b => (optional + 92, optional + 96),
        0x20b => (optional + 108, optional + 112),
        _ => return None,
    };
    // Security is data directory #4.
    if u32_at(&header, directory_count_at)? < 5 {
        return None;
    }
    let entry = directories_at + 4 * 8;
    let offset = u64::from(u32_at(&header, entry)?);
    let length = u64::from(u32_at(&header, entry + 4)?);
    // The security directory holds a *file offset*, not an RVA.
    (offset >= TRAILER_LEN && length > 0 && offset.checked_add(length)? <= size).then_some(offset)
}

/// File offset of the `LC_CODE_SIGNATURE` blob of a thin 64-bit little-endian Mach-O.
fn macho_code_signature_start(f: &mut File, size: u64) -> Option<u64> {
    const MH_MAGIC_64: u32 = 0xfeed_facf;
    const LC_CODE_SIGNATURE: u32 = 0x1d;
    let mut header = [0u8; 32];
    read_exact_at(f, 0, &mut header)?;
    if u32_at(&header, 0)? != MH_MAGIC_64 {
        return None;
    }
    let command_count = u32_at(&header, 16)?;
    let commands_size = u64::from(u32_at(&header, 20)?);
    if commands_size > 1 << 20 {
        return None;
    }
    let mut commands = vec![0u8; usize::try_from(commands_size).ok()?];
    read_exact_at(f, 32, &mut commands)?;
    let mut at = 0usize;
    for _ in 0..command_count {
        let command = u32_at(&commands, at)?;
        let length = usize::try_from(u32_at(&commands, at + 4)?).ok()?;
        if length < 8 {
            return None;
        }
        if command == LC_CODE_SIGNATURE {
            let offset = u64::from(u32_at(&commands, at + 8)?);
            let length = u64::from(u32_at(&commands, at + 12)?);
            return (offset >= TRAILER_LEN && offset.checked_add(length)? <= size)
                .then_some(offset);
        }
        at = at.checked_add(length)?;
    }
    None
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Debug, Clone, Copy)]
pub struct PackEntry<'a> {
    pub rel: &'a str,
    pub data: &'a [u8],
}

impl<'a> PackEntry<'a> {
    pub const fn new(rel: &'a str, data: &'a [u8]) -> Self {
        Self { rel, data }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackWriteError {
    TooManyEntries { len: usize },
    PathTooLong { len: usize, path: String },
    DataTooLarge { len: u64, path: String },
    Io(String),
}

impl fmt::Display for PackWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyEntries { len } => write!(f, "too many pack entries: {len}"),
            Self::PathTooLong { len, path } => write!(f, "path too long ({len} bytes): {path}"),
            Self::DataTooLarge { len, path } => write!(f, "asset too large ({len} bytes): {path}"),
            Self::Io(err) => f.write_str(err),
        }
    }
}

impl Error for PackWriteError {}

/// Validate every size encoded by the TYNDPKG section header.
///
/// Callers writing to a filesystem can run this before creating the output,
/// preserving transactional behavior for invalid input.
pub fn validate_entries(entries: &[PackEntry<'_>]) -> Result<(), PackWriteError> {
    if entries.len() > u32::MAX as usize {
        return Err(PackWriteError::TooManyEntries { len: entries.len() });
    }

    for entry in entries {
        let path_len = entry.rel.len();
        if path_len > u16::MAX as usize {
            return Err(PackWriteError::PathTooLong {
                len: path_len,
                path: entry.rel.to_string(),
            });
        }
        if entry.data.len() > u32::MAX as usize {
            return Err(PackWriteError::DataTooLarge {
                len: entry.data.len() as u64,
                path: entry.rel.to_string(),
            });
        }
    }
    Ok(())
}

/// Write a TYNDPKG section and trailer to `writer`.
///
/// The caller is responsible for writing the host binary bytes first.
pub fn write_section_and_trailer<'a, W, I>(
    writer: &mut W,
    entries: I,
) -> Result<u64, PackWriteError>
where
    W: Write,
    I: IntoIterator<Item = PackEntry<'a>>,
{
    let entries: Vec<PackEntry<'a>> = entries.into_iter().collect();
    validate_entries(&entries)?;

    let mut section_size: u64 = 0;
    writer
        .write_all(&(entries.len() as u32).to_le_bytes())
        .map_err(|e| PackWriteError::Io(format!("write file_count: {e}")))?;
    section_size += 4;

    for entry in &entries {
        let path_bytes = entry.rel.as_bytes();
        writer
            .write_all(&(path_bytes.len() as u16).to_le_bytes())
            .map_err(|e| PackWriteError::Io(format!("write path_len: {e}")))?;
        writer
            .write_all(path_bytes)
            .map_err(|e| PackWriteError::Io(format!("write path: {e}")))?;
        writer
            .write_all(&(entry.data.len() as u32).to_le_bytes())
            .map_err(|e| PackWriteError::Io(format!("write data_len: {e}")))?;
        writer
            .write_all(entry.data)
            .map_err(|e| PackWriteError::Io(format!("write data: {e}")))?;
        section_size += 2 + path_bytes.len() as u64 + 4 + entry.data.len() as u64;
    }

    writer
        .write_all(&section_size.to_le_bytes())
        .map_err(|e| PackWriteError::Io(format!("write section_size: {e}")))?;
    writer
        .write_all(MAGIC)
        .map_err(|e| PackWriteError::Io(format!("write magic: {e}")))?;

    Ok(section_size)
}

/// Payload of the [`DEBUG_FLAG_ENTRY`] entry when the flag is on.
pub const DEBUG_FLAG_ON: &[u8] = b"1";

/// The pack entry to append when the debug flag is on; `None` when it is off
/// (the flag is never written as an explicit `0`).
pub fn debug_flag_entry(enabled: bool) -> Option<PackEntry<'static>> {
    enabled.then_some(PackEntry::new(DEBUG_FLAG_ENTRY, DEBUG_FLAG_ON))
}

/// Interpret the payload of a [`DEBUG_FLAG_ENTRY`] entry. Anything but `1`
/// (surrounding whitespace ignored) reads as off, so a damaged flag fails closed.
pub fn parse_debug_flag(data: &[u8]) -> bool {
    data.trim_ascii() == DEBUG_FLAG_ON
}

/// Validate a path segment carried after `frontend/` or `sidecar/`.
pub fn validate_subpath(rest: &str) -> Option<&str> {
    // A leading separator also rejects UNC (`\\server`) and `//` prefixes.
    if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') {
        return None;
    }
    for comp in rest.split(['/', '\\']) {
        if comp.is_empty() || comp == ".." || comp == "." {
            return None;
        }
        // Rejected on every platform so a pack extracts identically everywhere:
        // `:` covers drive prefixes and NTFS alternate data streams, trailing
        // dots/spaces are silently stripped by Win32, and device names resolve
        // to the device instead of a file.
        if comp.contains(':') || comp.ends_with('.') || comp.ends_with(' ') {
            return None;
        }
        if is_windows_device_name(comp) {
            return None;
        }
    }
    Some(rest)
}

/// `CON`, `NUL`, `COM1`... with any extension (`nul.txt` is still the device).
fn is_windows_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).trim_end();
    let stem = stem.to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|digit| matches!(digit.as_bytes(), [b'1'..=b'9']))
}

#[cfg(test)]
mod tests;
