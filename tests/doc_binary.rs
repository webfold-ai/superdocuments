//! Legacy Word binary (`.doc`) files: `convert_bytes("doc", ..)` reads the main-document text from the
//! OLE compound file. Before, `doc` was an unsupported extension and the callers that fell back to
//! reading the bytes as text (webfold's `convert`) failed with "stream did not contain valid UTF-8".
//!
//! The synthetic files are written here byte by byte (an OLE container with a `WordDocument` and a
//! `0Table` stream, a Word 97 FIB, a piece table with 8-bit and UTF-16 pieces), so every
//! construct the reader handles has a test that names it. The Register's own file is in
//! `tests/fixtures/au-register/`.

use superdocuments::document::convert_bytes;

// ── a minimal OLE compound file writer (512-byte sectors, every stream >= 4096 bytes) ──────────

fn ole(streams: &[(&str, Vec<u8>)]) -> Vec<u8> {
    const SECTOR: usize = 512;
    // layout: sector 0 = FAT, sector 1.. = directory, then the streams
    let dir_entries = 1 + streams.len();
    let dir_sectors = dir_entries.div_ceil(4);
    let mut next = 1 + dir_sectors; // first stream sector
    let mut fat: Vec<u32> = vec![0xFFFF_FFFD]; // sector 0 holds the FAT itself
    // directory chain
    for k in 0..dir_sectors {
        fat.push(if k + 1 == dir_sectors { 0xFFFF_FFFE } else { (2 + k) as u32 });
    }
    let mut starts = Vec::new();
    let mut padded: Vec<Vec<u8>> = Vec::new();
    for (_, data) in streams {
        let mut d = data.clone();
        while d.len() < 4096 || d.len() % SECTOR != 0 {
            d.push(0);
        }
        let sectors = d.len() / SECTOR;
        starts.push(next as u32);
        for k in 0..sectors {
            fat.push(if k + 1 == sectors { 0xFFFF_FFFE } else { (next + k + 1) as u32 });
        }
        next += sectors;
        padded.push(d);
    }
    assert!(fat.len() <= 128, "test writer: one FAT sector only");
    let mut out = vec![0u8; SECTOR]; // header
    out[..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    out[0x18..0x1A].copy_from_slice(&0x003Eu16.to_le_bytes());
    out[0x1A..0x1C].copy_from_slice(&3u16.to_le_bytes());
    out[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
    out[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes()); // sector shift
    out[0x20..0x22].copy_from_slice(&6u16.to_le_bytes()); // mini sector shift
    out[0x2C..0x30].copy_from_slice(&1u32.to_le_bytes()); // one FAT sector
    out[0x30..0x34].copy_from_slice(&1u32.to_le_bytes()); // directory starts at sector 1
    out[0x38..0x3C].copy_from_slice(&4096u32.to_le_bytes()); // mini stream cutoff
    out[0x3C..0x40].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes()); // no mini FAT
    out[0x44..0x48].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes()); // no DIFAT
    for k in 0..109 {
        let v: u32 = if k == 0 { 0 } else { 0xFFFF_FFFF };
        out[0x4C + 4 * k..0x50 + 4 * k].copy_from_slice(&v.to_le_bytes());
    }
    // FAT sector
    let mut fat_sector = vec![0xFFu8; SECTOR];
    for (k, v) in fat.iter().enumerate() {
        fat_sector[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&fat_sector);
    // directory
    let mut dir = vec![0u8; dir_sectors * SECTOR];
    let mut entry = |k: usize, name: &str, kind: u8, start: u32, size: u64| {
        let e = &mut dir[k * 128..(k + 1) * 128];
        let units: Vec<u16> = name.encode_utf16().collect();
        for (i, u) in units.iter().enumerate() {
            e[2 * i..2 * i + 2].copy_from_slice(&u.to_le_bytes());
        }
        e[0x40..0x42].copy_from_slice(&((units.len() as u16 + 1) * 2).to_le_bytes());
        e[0x42] = kind;
        e[0x44..0x48].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        e[0x48..0x4C].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        e[0x4C..0x50].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        e[0x74..0x78].copy_from_slice(&start.to_le_bytes());
        e[0x78..0x80].copy_from_slice(&size.to_le_bytes());
    };
    entry(0, "Root Entry", 5, 0xFFFF_FFFE, 0);
    for (k, (((name, _), start), data)) in streams.iter().zip(&starts).zip(&padded).enumerate() {
        entry(k + 1, name, 2, *start, data.len() as u64);
    }
    out.extend_from_slice(&dir);
    for d in padded {
        out.extend_from_slice(&d);
    }
    out
}

// ── a minimal Word 97 document ────────────────────────────────────────────────────────────────

/// One piece of text: the characters and whether they are stored as 8-bit (cp1252) or UTF-16.
enum Piece<'a> {
    Latin(&'a str),
    Wide(&'a str),
}

fn cp1252(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            '\u{2019}' => 0x92,
            '\u{201C}' => 0x93,
            '\u{201D}' => 0x94,
            c if (c as u32) < 256 => c as u8,
            _ => b'?',
        })
        .collect()
}

fn word97(pieces: &[Piece], flags: u16, n_fib: u16) -> Vec<u8> {
    let mut word = vec![0u8; 0x800];
    let mut cps = vec![0u32];
    let mut pcds: Vec<(u32, u32)> = Vec::new(); // (fc with compression bit, length)
    for p in pieces {
        let (bytes, compressed, chars) = match p {
            Piece::Latin(s) => (cp1252(s), true, s.chars().count()),
            Piece::Wide(s) => (
                s.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>(),
                false,
                s.encode_utf16().count(),
            ),
        };
        let at = word.len() as u32;
        word.extend_from_slice(&bytes);
        let fc = if compressed { (at * 2) | 0x4000_0000 } else { at };
        pcds.push((fc, chars as u32));
        cps.push(cps.last().unwrap() + chars as u32);
    }
    let ccp_text = *cps.last().unwrap();
    word[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
    word[2..4].copy_from_slice(&n_fib.to_le_bytes());
    word[0x0A..0x0C].copy_from_slice(&flags.to_le_bytes());
    word[32..34].copy_from_slice(&14u16.to_le_bytes()); // csw
    word[62..64].copy_from_slice(&22u16.to_le_bytes()); // cslw
    word[64 + 12..64 + 16].copy_from_slice(&ccp_text.to_le_bytes()); // ccpText
    word[152..154].copy_from_slice(&93u16.to_le_bytes()); // cbRgFcLcb
    // Clx: Pcdt (0x02, lcb, PlcPcd)
    let n = pcds.len();
    let mut plc = Vec::new();
    for cp in &cps {
        plc.extend_from_slice(&cp.to_le_bytes());
    }
    for (fc, _) in &pcds {
        plc.extend_from_slice(&0u16.to_le_bytes());
        plc.extend_from_slice(&fc.to_le_bytes());
        plc.extend_from_slice(&0u16.to_le_bytes());
    }
    assert_eq!(plc.len(), 4 * (n + 1) + 8 * n);
    let mut clx = vec![0x01u8, 0x02, 0x00, 0xAA, 0xBB]; // a Prc to be skipped: clxt 1, cb 2, 2 bytes
    clx.push(0x02);
    clx.extend_from_slice(&(plc.len() as u32).to_le_bytes());
    clx.extend_from_slice(&plc);
    let table_name = if flags & 0x0200 != 0 { "1Table" } else { "0Table" };
    let fc_clx_at = 154 + 33 * 8;
    word[fc_clx_at..fc_clx_at + 4].copy_from_slice(&0u32.to_le_bytes());
    word[fc_clx_at + 4..fc_clx_at + 8].copy_from_slice(&(clx.len() as u32).to_le_bytes());
    ole(&[("WordDocument", word), (table_name, clx)])
}

fn convert(doc: &[u8]) -> Result<String, String> {
    convert_bytes("doc", doc)
}

// ── what the reader returns ────────────────────────────────────────────────────────────────────

#[test]
fn paragraphs_from_8bit_and_utf16_pieces() {
    let doc = word97(
        &[
            Piece::Latin("Census \u{2019}61 caf\u{e9}\rSecond paragraph\r"),
            Piece::Wide("Third: \u{4e2d}\u{6587} \u{1F600}\r"),
        ],
        0,
        0x00C1,
    );
    let md = convert(&doc).expect("a Word 97 document converts");
    assert_eq!(
        md,
        "Census \u{2019}61 caf\u{e9}\n\nSecond paragraph\n\nThird: \u{4e2d}\u{6587} \u{1F600}"
    );
}

#[test]
fn second_table_stream_is_used_when_the_fib_says_so() {
    let doc = word97(&[Piece::Latin("from 1Table\r")], 0x0200, 0x0101);
    assert_eq!(convert(&doc).unwrap(), "from 1Table");
}

#[test]
fn field_instructions_are_dropped_and_results_kept() {
    let doc = word97(
        &[Piece::Latin("See \u{13} HYPERLINK \"http://x\" \u{14}the schedule\u{15}, page \u{13} PAGE \u{15}.\r")],
        0,
        0x00C1,
    );
    assert_eq!(convert(&doc).unwrap(), "See the schedule, page .");
}

#[test]
fn table_cells_become_a_markdown_table() {
    let doc = word97(
        &[Piece::Latin("Before\rName\u{7}Value\u{7}\u{7}Alpha\u{7}1\u{7}\u{7}After\r")],
        0,
        0x00C1,
    );
    assert_eq!(
        convert(&doc).unwrap(),
        "Before\n\n| Name | Value |\n| --- | --- |\n| Alpha | 1 |\n\nAfter"
    );
}

#[test]
fn only_the_main_document_story_is_read() {
    // ccpText counts the main story; characters after it (headers, footnotes) are not part of it
    let mut doc_pieces = vec![Piece::Latin("Body\r")];
    doc_pieces.push(Piece::Latin("Footnote text\r"));
    let mut doc = word97(&doc_pieces, 0, 0x00C1);
    // rewrite ccpText to the first piece only, inside the (uncompressed, page-padded) WordDocument stream
    let needle = 0xA5ECu16.to_le_bytes();
    let at = doc.windows(2).position(|w| w == needle).unwrap();
    doc[at + 64 + 12..at + 64 + 16].copy_from_slice(&5u32.to_le_bytes());
    assert_eq!(convert(&doc).unwrap(), "Body");
}

// ── what it refuses, by name ───────────────────────────────────────────────────────────────────

#[test]
fn encrypted_documents_are_refused_by_name() {
    let doc = word97(&[Piece::Latin("secret\r")], 0x0100, 0x00C1);
    let err = convert(&doc).unwrap_err();
    assert!(err.contains("encrypted") && err.contains(".doc"), "{err}");
}

#[test]
fn foreign_content_under_a_doc_name_is_named_not_misread() {
    let err = convert(b"just some text").unwrap_err();
    assert!(err.contains("legacy Word (.doc)"), "{err}");
    // an OLE container that is not a Word document (here: no WordDocument stream)
    let other = ole(&[("Workbook", vec![1, 2, 3])]);
    let err = convert(&other).unwrap_err();
    assert!(err.contains("WordDocument"), "{err}");
}

#[test]
fn rtf_and_docx_saved_as_doc_are_read_by_content() {
    let rtf = br"{\rtf1\ansi Hello from RTF\par}";
    assert!(convert(rtf).unwrap().contains("Hello from RTF"));
    // not a zip with a document in it: the OOXML reader's own error, not an extension error
    let err = convert(b"PK\x03\x04garbage").unwrap_err();
    assert!(!err.contains("unsupported document extension"), "{err}");
}

#[test]
fn truncated_and_corrupted_files_error_without_panicking() {
    let doc = word97(
        &[Piece::Latin("Body text of some length for the truncation sweep\r")],
        0,
        0x00C1,
    );
    for cut in (0..doc.len()).step_by(97) {
        let _ = convert(&doc[..cut]); // must return, Ok or Err
    }
    let mut broken = doc.clone();
    for (k, b) in broken.iter_mut().enumerate().skip(0x200).step_by(61) {
        *b = b.wrapping_add(k as u8 | 1);
    }
    let _ = convert(&broken);
    // a FAT that loops back on itself
    let mut looped = doc.clone();
    looped[512 + 4 * 4..512 + 4 * 4 + 4].copy_from_slice(&3u32.to_le_bytes());
    let _ = convert(&looped);
}

// ── the Register's own file ─────────────────────────────────────────────────────────────────────

#[test]
fn register_doc_fixture_reads() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/au-register/census-regulations-1909-05-26.doc");
    let md = superdocuments::document::convert_path(&path).expect("the Register's .doc converts");
    assert!(md.contains("Census"), "{md:.300}");
    assert!(md.len() > 500, "only {} characters", md.len());
}
