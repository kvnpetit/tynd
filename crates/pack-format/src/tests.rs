use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct TempFile(std::path::PathBuf);

impl TempFile {
    fn with_bytes(bytes: &[u8]) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "tynd-pack-format-test-{}-{n}.bin",
            std::process::id()
        ));
        std::fs::write(&path, bytes).expect("write temp file");
        Self(path)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn pack(prefix: &[u8], entries: &[PackEntry<'_>]) -> Vec<u8> {
    let mut buf = prefix.to_vec();
    write_section_and_trailer(&mut buf, entries.iter().copied()).expect("write pack");
    buf
}

fn read_all(file: &TempFile) -> Vec<(String, Vec<u8>)> {
    let mut reader = PackReader::open_path(&file.0).expect("pack detected");
    let mut out = Vec::new();
    while let Some((rel, len)) = reader.next_header().expect("header") {
        let mut data = Vec::new();
        reader.copy_entry_to(len, &mut data).expect("copy");
        out.push((rel, data));
    }
    assert_eq!(reader.remaining(), 0);
    out
}

fn raw_pack(section: &[u8]) -> Vec<u8> {
    let mut buf = section.to_vec();
    buf.extend_from_slice(&(section.len() as u64).to_le_bytes());
    buf.extend_from_slice(MAGIC);
    buf
}

#[test]
fn round_trip_preserves_entries_and_host_prefix() {
    let entries = [
        PackEntry::new("frontend/index.html", b"<html></html>"),
        PackEntry::new("backend.js", b""),
        PackEntry::new("sidecar/bin/\u{e9}.exe", &[0, 1, 2, 255]),
    ];
    let file = TempFile::with_bytes(&pack(b"MZ-host-binary-bytes", &entries));
    let got = read_all(&file);
    assert_eq!(got.len(), 3);
    for (entry, (rel, data)) in entries.iter().zip(&got) {
        assert_eq!(entry.rel, rel);
        assert_eq!(entry.data, data.as_slice());
    }
}

#[test]
fn round_trip_empty_pack() {
    let file = TempFile::with_bytes(&pack(b"", &[]));
    assert_eq!(read_all(&file), Vec::new());
}

#[test]
fn writer_reports_section_size_matching_layout() {
    let mut buf = Vec::new();
    let size = write_section_and_trailer(&mut buf, [PackEntry::new("a", b"xyz")]).unwrap();
    // count(4) + path_len(2) + path(1) + data_len(4) + data(3)
    assert_eq!(size, 14);
    assert_eq!(buf.len() as u64, size + TRAILER_LEN);
    assert_eq!(&buf[buf.len() - 8..], MAGIC);
}

#[test]
fn skip_consumes_entry_and_allows_next() {
    let entries = [PackEntry::new("a", b"1234"), PackEntry::new("b", b"56")];
    let file = TempFile::with_bytes(&pack(b"host", &entries));
    let mut reader = PackReader::open_path(&file.0).unwrap();
    let (rel, len) = reader.next_header().unwrap().unwrap();
    assert_eq!((rel.as_str(), len), ("a", 4));
    reader.skip(len).unwrap();
    let (rel, len) = reader.next_header().unwrap().unwrap();
    assert_eq!((rel.as_str(), len), ("b", 2));
    reader.skip(len).unwrap();
    assert!(reader.next_header().unwrap().is_none());
}

#[test]
fn unconsumed_entry_is_rejected() {
    let file = TempFile::with_bytes(&pack(b"", &[PackEntry::new("a", b"1234")]));
    let mut reader = PackReader::open_path(&file.0).unwrap();
    reader.next_header().unwrap().unwrap();
    assert!(reader.next_header().is_err());
}

#[test]
fn copy_with_wrong_length_is_rejected() {
    let file = TempFile::with_bytes(&pack(b"", &[PackEntry::new("a", b"1234")]));
    let mut reader = PackReader::open_path(&file.0).unwrap();
    reader.next_header().unwrap().unwrap();
    let err = reader.copy_entry_to(2, &mut Vec::new()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn copy_without_header_is_rejected() {
    let file = TempFile::with_bytes(&pack(b"", &[PackEntry::new("a", b"1")]));
    let mut reader = PackReader::open_path(&file.0).unwrap();
    assert!(reader.copy_entry_to(1, &mut Vec::new()).is_err());
}

#[test]
fn open_rejects_non_pack_files() {
    assert!(PackReader::open_path(&TempFile::with_bytes(b"short").0).is_none());
    assert!(PackReader::open_path(&TempFile::with_bytes(&[0u8; 64]).0).is_none());
    let missing = std::env::temp_dir().join("tynd-pack-format-does-not-exist.bin");
    assert!(PackReader::open_path(&missing).is_none());
}

#[test]
fn open_rejects_oversized_or_tiny_section_size() {
    let mut oversized = vec![0u8; 8];
    oversized.extend_from_slice(&1000u64.to_le_bytes());
    oversized.extend_from_slice(MAGIC);
    assert!(PackReader::open_path(&TempFile::with_bytes(&oversized).0).is_none());

    let mut tiny = vec![0u8; 8];
    tiny.extend_from_slice(&3u64.to_le_bytes());
    tiny.extend_from_slice(MAGIC);
    assert!(PackReader::open_path(&TempFile::with_bytes(&tiny).0).is_none());
}

#[test]
fn truncated_entry_data_is_rejected() {
    // Section claims one entry with 100 data bytes but only provides 2.
    let mut section = Vec::new();
    section.extend_from_slice(&1u32.to_le_bytes());
    section.extend_from_slice(&1u16.to_le_bytes());
    section.push(b'a');
    section.extend_from_slice(&100u32.to_le_bytes());
    section.extend_from_slice(b"xy");
    let file = TempFile::with_bytes(&raw_pack(&section));
    let mut reader = PackReader::open_path(&file.0).unwrap();
    assert!(reader.next_header().is_err());
}

#[test]
fn trailing_bytes_in_section_are_rejected() {
    let mut section = Vec::new();
    section.extend_from_slice(&0u32.to_le_bytes());
    section.extend_from_slice(b"junk");
    let file = TempFile::with_bytes(&raw_pack(&section));
    let mut reader = PackReader::open_path(&file.0).unwrap();
    assert!(reader.next_header().is_err());
}

#[test]
fn validate_entries_rejects_long_paths() {
    let long = "a".repeat(u16::MAX as usize + 1);
    let err = validate_entries(&[PackEntry::new(&long, b"")]).unwrap_err();
    assert!(matches!(err, PackWriteError::PathTooLong { .. }));
    assert!(err.to_string().contains("path too long"));

    let ok = "a".repeat(u16::MAX as usize);
    assert!(validate_entries(&[PackEntry::new(&ok, b"")]).is_ok());
}

#[test]
fn write_fails_before_emitting_bytes_on_invalid_input() {
    let long = "a".repeat(u16::MAX as usize + 1);
    let mut buf = Vec::new();
    let result = write_section_and_trailer(&mut buf, [PackEntry::new(&long, b"")]);
    assert!(result.is_err());
    assert_eq!(buf, [] as [u8; 0]);
}

#[test]
fn validate_subpath_accepts_relative_paths() {
    for ok in [
        "index.html",
        "assets/app.js",
        "a\\b\\c.txt",
        "dir/.hidden",
        "a..b",
    ] {
        assert_eq!(validate_subpath(ok), Some(ok), "{ok}");
    }
}

#[test]
fn validate_subpath_rejects_traversal_and_absolute() {
    for bad in [
        "",
        "/etc/passwd",
        "\\windows",
        "//server/share",
        "\\\\server\\share",
        "..",
        ".",
        "a/../b",
        "a/./b",
        "a//b",
        "a/",
        "C:/x",
        "c:\\x",
        "a/D:/x",
    ] {
        assert_eq!(validate_subpath(bad), None, "{bad}");
    }
}

#[test]
fn validate_subpath_rejects_windows_hostile_names() {
    for bad in [
        "a.txt:stream",
        "dir/file:Zone.Identifier",
        "nul",
        "NUL.txt",
        "dir/con",
        "Com1",
        "lpt9.log",
        "aux.",
        "name.",
        "name ",
        "dir./x",
    ] {
        assert_eq!(validate_subpath(bad), None, "{bad}");
    }
    for ok in ["com10", "console.js", "com0", "lpt", "a.nul", "nulls.txt"] {
        assert_eq!(validate_subpath(ok), Some(ok), "{ok}");
    }
}

#[test]
fn damaged_pack_is_an_error_not_absent() {
    let mut oversized = vec![0u8; 8];
    oversized.extend_from_slice(&1000u64.to_le_bytes());
    oversized.extend_from_slice(MAGIC);
    let file = TempFile::with_bytes(&oversized);
    assert!(PackReader::open_path_checked(&file.0).is_err());

    let plain = TempFile::with_bytes(&[0u8; 64]);
    assert!(matches!(PackReader::open_path_checked(&plain.0), Ok(None)));
}

/// Minimal PE32+ image: DOS header, PE header, 16 data directories, then `body`.
/// Returns the image and the offset of the security directory entry.
fn fake_pe(body: &[u8]) -> (Vec<u8>, usize) {
    let pe_offset = 0x40usize;
    let optional = pe_offset + 4 + 20;
    let directories = optional + 112;
    let mut image = vec![0u8; directories + 16 * 8];
    image[..2].copy_from_slice(b"MZ");
    image[0x3c..0x40].copy_from_slice(&(pe_offset as u32).to_le_bytes());
    image[pe_offset..pe_offset + 4].copy_from_slice(&[b'P', b'E', 0, 0]);
    image[optional..optional + 2].copy_from_slice(&0x20bu16.to_le_bytes());
    image[optional + 108..optional + 112].copy_from_slice(&16u32.to_le_bytes());
    image.extend_from_slice(body);
    (image, directories + 4 * 8)
}

/// Append a fake Authenticode certificate table the way signing tools do:
/// pad to 8 bytes, record the table's file offset/size, append the blob.
fn sign_fake_pe(mut image: Vec<u8>, security_entry: usize, certificate: &[u8]) -> Vec<u8> {
    while !image.len().is_multiple_of(8) {
        image.push(0);
    }
    let offset = image.len() as u32;
    image[security_entry..security_entry + 4].copy_from_slice(&offset.to_le_bytes());
    image[security_entry + 4..security_entry + 8]
        .copy_from_slice(&(certificate.len() as u32).to_le_bytes());
    image.extend_from_slice(certificate);
    image
}

#[test]
fn pack_is_found_before_an_authenticode_certificate_table() {
    let entries = [PackEntry::new("bundle.js", b"console.log(1)")];
    // Odd prefix length so the signer's 8-byte padding is exercised.
    for prefix_len in 0..8 {
        let mut body = vec![7u8; prefix_len];
        write_section_and_trailer(&mut body, entries.iter().copied()).unwrap();
        let (image, security) = fake_pe(&body);
        let signed = sign_fake_pe(image, security, &[0xAB; 1234]);
        let file = TempFile::with_bytes(&signed);
        let got = read_all(&file);
        assert_eq!(
            got,
            vec![("bundle.js".to_string(), b"console.log(1)".to_vec())]
        );
    }
}

#[test]
fn unsigned_pe_without_pack_is_not_a_pack() {
    let (image, security) = fake_pe(&[1, 2, 3, 4]);
    let signed = sign_fake_pe(image, security, &[0xCD; 64]);
    let file = TempFile::with_bytes(&signed);
    assert!(matches!(PackReader::open_path_checked(&file.0), Ok(None)));
}

#[test]
fn pe_security_directory_outside_the_file_is_ignored() {
    let mut body = Vec::new();
    write_section_and_trailer(&mut body, [PackEntry::new("a", b"1")]).unwrap();
    let (mut image, security) = fake_pe(&body);
    image.extend_from_slice(&[9u8; 32]);
    image[security..security + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    image[security + 4..security + 8].copy_from_slice(&64u32.to_le_bytes());
    let file = TempFile::with_bytes(&image);
    assert!(PackReader::open_path(&file.0).is_none());
}

#[test]
fn pack_is_found_before_a_macho_code_signature() {
    let mut body = Vec::new();
    write_section_and_trailer(&mut body, [PackEntry::new("bundle.js", b"x")]).unwrap();
    // 64-bit Mach-O header + one LC_CODE_SIGNATURE command.
    let mut image = vec![0u8; 32 + 16];
    image[..4].copy_from_slice(&0xfeed_facfu32.to_le_bytes());
    image[16..20].copy_from_slice(&1u32.to_le_bytes());
    image[20..24].copy_from_slice(&16u32.to_le_bytes());
    image.extend_from_slice(&body);
    while !image.len().is_multiple_of(16) {
        image.push(0);
    }
    let signature = [0xEE; 300];
    image[32..36].copy_from_slice(&0x1du32.to_le_bytes());
    image[36..40].copy_from_slice(&16u32.to_le_bytes());
    let signature_offset = image.len() as u32;
    image[40..44].copy_from_slice(&signature_offset.to_le_bytes());
    image[44..48].copy_from_slice(&(signature.len() as u32).to_le_bytes());
    image.extend_from_slice(&signature);
    let file = TempFile::with_bytes(&image);
    assert_eq!(
        read_all(&file),
        vec![("bundle.js".to_string(), b"x".to_vec())]
    );
}

fn packed_debug_flag(enabled: bool) -> bool {
    let mut entries = vec![PackEntry::new("bundle.js", b"export {}")];
    entries.extend(debug_flag_entry(enabled));
    entries.push(PackEntry::new("frontend/index.html", b"<html></html>"));
    let file = TempFile::with_bytes(&pack(b"host", &entries));
    read_all(&file)
        .iter()
        .find(|(rel, _)| rel == DEBUG_FLAG_ENTRY)
        .is_some_and(|(_, data)| parse_debug_flag(data))
}

#[test]
fn debug_flag_round_trips_through_a_pack() {
    assert!(packed_debug_flag(true));
    assert!(!packed_debug_flag(false));
}

#[test]
fn pack_without_the_debug_entry_reads_as_off() {
    // A pack written before the flag existed carries no `debug.flag` entry.
    let file = TempFile::with_bytes(&pack(b"host", &[PackEntry::new("bundle.js", b"x")]));
    assert!(read_all(&file)
        .iter()
        .all(|(rel, _)| rel != DEBUG_FLAG_ENTRY));
    assert!(debug_flag_entry(false).is_none());
}

#[test]
fn damaged_debug_flag_fails_closed() {
    assert!(parse_debug_flag(b"1"));
    assert!(parse_debug_flag(b" 1\n"));
    assert!(!parse_debug_flag(b""));
    assert!(!parse_debug_flag(b"0"));
    assert!(!parse_debug_flag(b"true"));
}
