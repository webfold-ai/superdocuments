//! Tests for the cross-reference loader, object streams and stream filters (G18). Each test drives
//! the production entry points (`Document::parse`, `Document::resolve`, `Document::stream_bytes`,
//! `crate::pdf::extract_markdown`); the inputs are small PDFs built here, one per defect.

use super::{startxref, Loader};
use crate::pdf::parser::{Document, Object};
use std::time::{Duration, Instant};

/// Adler-32 of `data` (the zlib trailer).
fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// A zlib stream of stored (uncompressed) deflate blocks: a valid FlateDecode payload.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = if data.is_empty() {
        vec![&[]]
    } else {
        data.chunks(65535).collect()
    };
    for (i, c) in chunks.iter().enumerate() {
        let last = i + 1 == chunks.len();
        out.push(if last { 1 } else { 0 });
        let len = c.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Encode rows with PNG filter type `tag` (0 None, 1 Sub, 2 Up), one byte per pixel (bpp 1). The
/// reference encoder is independent of the decoder under test.
fn png_encode(rows: &[Vec<u8>], tag: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut prev = vec![0u8; rows.first().map_or(0, |r| r.len())];
    for row in rows {
        out.push(tag);
        for (j, &x) in row.iter().enumerate() {
            let a = if j >= 1 { row[j - 1] } else { 0 };
            let b = prev[j];
            let pred = match tag {
                0 => 0,
                1 => a,
                _ => b,
            };
            out.push(x.wrapping_sub(pred));
        }
        prev = row.clone();
    }
    out
}

/// Build a classic PDF: each object is written at its own offset, followed by an `xref` table and
/// a trailer that names `root`. `free` lists object numbers marked free in the table.
fn classic_pdf(objs: &[(u32, Vec<u8>)], root: u32, free: &[u32]) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets: Vec<(u32, usize)> = Vec::new();
    for (num, body) in objs {
        offsets.push((*num, out.len()));
        out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let size = offsets.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for num in 1..size {
        match offsets.iter().find(|(n, _)| *n == num) {
            Some((_, off)) if !free.contains(&num) => {
                out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes())
            }
            _ => out.extend_from_slice(b"0000000000 65535 f \n"),
        }
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root {root} 0 R >>\nstartxref\n{xref_at}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

/// The objects of a one-page document with `text` on its page.
fn page_objects(text: &str) -> Vec<(u32, Vec<u8>)> {
    let content = format!("BT /F1 12 Tf 72 700 Td ({text}) Tj ET");
    vec![
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
        (
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
              /Resources << /Font << /F1 5 0 R >> >> >>"
                .to_vec(),
        ),
        (
            4,
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
            .into_bytes(),
        ),
        (
            5,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ),
    ]
}

/// The bytes of a `/W` row with field widths `w` and values `fields`, big-endian.
fn row_bytes(w: [usize; 3], fields: [u64; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    for k in 0..3 {
        for b in (0..w[k]).rev() {
            out.push((fields[k] >> (8 * b)) as u8);
        }
    }
    out
}

/// A classic PDF whose catalog, pages and page are compressed in one object stream (object 6),
/// with an xref stream (object 7) that names them. `filter` wraps the xref stream's data.
fn xref_stream_pdf(predict: bool) -> Vec<u8> {
    let content = "BT /F1 12 Tf 72 700 Td (Compressed catalog) Tj ET";
    // Object stream 6: catalog 1, pages 2, page 3 (the page refers to the content in object 4).
    let members: [(u32, &str); 3] = [
        (1, "<< /Type /Catalog /Pages 2 0 R >>"),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (
            3,
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
             /Resources << /Font << /F1 5 0 R >> >> >>",
        ),
    ];
    let mut header = String::new();
    let mut body = String::new();
    for (num, text) in members {
        header.push_str(&format!("{num} {} ", body.len()));
        body.push_str(text);
        body.push(' ');
    }
    let first = header.len();
    let objstm = format!("{header}{body}").into_bytes();

    let mut out = b"%PDF-1.5\n".to_vec();
    let content_obj = out.len();
    out.extend_from_slice(
        format!(
            "4 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj\n",
            content.len()
        )
        .as_bytes(),
    );
    let font_obj = out.len();
    out.extend_from_slice(
        b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
    );
    let objstm_at = out.len();
    let objstm_dict = format!(
        "6 0 obj\n<< /Type /ObjStm /N 3 /First {first} /Length {} >>\nstream\n",
        objstm.len()
    );
    out.extend_from_slice(objstm_dict.as_bytes());
    out.extend_from_slice(&objstm);
    out.extend_from_slice(b"\nendstream\nendobj\n");

    // Xref stream 7: rows for objects 0..=7 with W [1 4 2].
    let w = [1usize, 4, 2];
    let mut rows: Vec<Vec<u8>> = vec![
        row_bytes(w, [0, 0, 255]),
        row_bytes(w, [2, 6, 0]),
        row_bytes(w, [2, 6, 1]),
        row_bytes(w, [2, 6, 2]),
        row_bytes(w, [1, content_obj as u64, 0]),
        row_bytes(w, [1, font_obj as u64, 0]),
        row_bytes(w, [1, objstm_at as u64, 0]),
    ];
    let xref_obj = out.len();
    rows.push(row_bytes(w, [1, xref_obj as u64, 0]));
    let raw_rows: Vec<u8> = rows.concat();
    let (data, extra) = if predict {
        // PNG Up (tag 2) per row of 7 bytes (w1+w2+w3 = 7 columns).
        let cols = 7;
        let mut enc = Vec::new();
        let mut prev = vec![0u8; cols];
        for chunk in raw_rows.chunks(cols) {
            enc.push(2u8);
            for (j, &x) in chunk.iter().enumerate() {
                enc.push(x.wrapping_sub(prev[j]));
            }
            prev = chunk.to_vec();
        }
        (
            enc,
            " /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 7 >>",
        )
    } else {
        (raw_rows, "")
    };
    let data = if predict { zlib_stored(&data) } else { data };
    out.extend_from_slice(
        format!(
            "7 0 obj\n<< /Type /XRef /Size 8 /W [1 4 2] /Root 1 0 R{extra} /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    out.extend_from_slice(format!("startxref\n{xref_obj}\n%%EOF\n").as_bytes());
    out
}

fn text_of(bytes: &[u8]) -> Option<String> {
    crate::pdf::extract_markdown(bytes).ok()
}

#[test]
fn classic_table_loads_catalog_and_text() {
    let pdf = classic_pdf(&page_objects("Hello classic xref"), 1, &[]);
    let md = text_of(&pdf).expect("classic table PDF extracts");
    assert!(md.contains("Hello classic xref"), "markdown: {md:?}");
}

#[test]
fn xref_stream_with_object_stream_loads_compressed_catalog() {
    // Regression for "PDF has no readable catalog": the catalog lives only in an object stream.
    let pdf = xref_stream_pdf(false);
    let doc = Document::parse(&pdf).expect("parse");
    assert_eq!(doc.root(), 1, "catalog read from the object stream");
    assert!(matches!(doc.object(1), Some(Object::Dict(_))));
    let md = text_of(&pdf).expect("compressed catalog PDF extracts");
    assert!(md.contains("Compressed catalog"), "markdown: {md:?}");
}

#[test]
fn flate_xref_stream_with_png_predictor_is_undone() {
    // The xref stream is FlateDecode with /Predictor 12: rows must be reconstructed to be read.
    let pdf = xref_stream_pdf(true);
    let doc = Document::parse(&pdf).expect("parse");
    assert_eq!(doc.root(), 1);
    let md = text_of(&pdf).expect("predicted xref stream PDF extracts");
    assert!(md.contains("Compressed catalog"), "markdown: {md:?}");
}

#[test]
fn generation_mismatch_resolves_to_null() {
    let pdf = classic_pdf(&page_objects("gen"), 1, &[]);
    let doc = Document::parse(&pdf).expect("parse");
    assert!(doc.resolve(&Object::Ref(2, 0)).is_some());
    assert!(
        doc.resolve(&Object::Ref(2, 1)).is_none(),
        "gen 1 of an object read as gen 0"
    );
}

#[test]
fn free_entry_resolves_to_null() {
    // Object 5 is in the body but marked free in the table: the newest section governs.
    let pdf = classic_pdf(&page_objects("free"), 1, &[5]);
    let doc = Document::parse(&pdf).expect("parse");
    assert!(doc.object(5).is_none(), "a free entry is null");
    assert!(doc.object(4).is_some());
}

#[test]
fn prev_pointing_at_itself_terminates() {
    // /Prev names the section's own offset: the chain must stop after one read.
    let mut pdf = classic_pdf(&page_objects("loop"), 1, &[]);
    let at: usize = {
        let s = String::from_utf8_lossy(&pdf);
        let i = s.rfind("startxref\n").unwrap() + "startxref\n".len();
        s[i..].split_whitespace().next().unwrap().parse().unwrap()
    };
    let needle = b"trailer\n<< /Size";
    let pos = pdf.windows(needle.len()).position(|w| w == needle).unwrap() + "trailer\n<<".len();
    pdf.splice(pos..pos, format!(" /Prev {at}").into_bytes());
    let doc = Document::parse(&pdf).expect("parse");
    assert_eq!(doc.root(), 1);
}

#[test]
fn stream_length_is_used_not_the_first_endstream() {
    // The stream's binary data contains "endstream"; /Length must decide the extent.
    let data: &[u8] = b"AB endstream CD";
    let mut objs = page_objects("x");
    let body = [
        format!("<< /Length {} >>\nstream\n", data.len()).into_bytes(),
        data.to_vec(),
        b"\nendstream".to_vec(),
    ]
    .concat();
    objs[3] = (4, body);
    let pdf = classic_pdf(&objs, 1, &[]);
    let doc = Document::parse(&pdf).expect("parse");
    let stream = doc.object(4).expect("content stream");
    assert_eq!(doc.stream_bytes(stream).as_deref(), Some(data));
}

#[test]
fn corrupt_startxref_recovers_by_header_scan() {
    let mut pdf = classic_pdf(&page_objects("recovered"), 1, &[]);
    let s = String::from_utf8_lossy(&pdf).into_owned();
    let i = s.rfind("startxref\n").unwrap() + "startxref\n".len();
    pdf.splice(i..i, b"999999999".to_vec());
    let md = text_of(&pdf).expect("recovered PDF extracts");
    assert!(md.contains("recovered"), "markdown: {md:?}");
}

#[test]
fn compressed_index_past_n_is_refused() {
    let mut pdf = xref_stream_pdf(false);
    // Point object 1 (compressed, index 0) at index 7 by rewriting its row: row 1 is [2 6 0] in
    // the raw xref stream; find it and change the index byte. The stream is uncompressed here.
    let needle = [2u8, 0, 0, 0, 6, 0, 0];
    let pos = pdf.windows(needle.len()).position(|w| w == needle).unwrap();
    pdf[pos + 6] = 7;
    let doc = Document::parse(&pdf).expect("parse");
    // The row's index 7 is past /N 3, so the entry itself is refused. Header recovery may still
    // find object 1 among the members; whatever is found must be the catalog, never another object.
    assert!(matches!(doc.object(1), None | Some(Object::Dict(_))));
    if let Some(Object::Dict(d)) = doc.object(1) {
        assert!(matches!(d.get(&"Type".to_string()), Some(Object::Name(t)) if t == "Catalog"));
    }
}

#[test]
fn unknown_filter_refuses_the_stream() {
    let mut objs = page_objects("x");
    objs[3] = (
        4,
        b"<< /Length 4 /Filter /LZWDecode >>\nstream\n41424344\nendstream".to_vec(),
    );
    let pdf = classic_pdf(&objs, 1, &[]);
    let doc = Document::parse(&pdf).expect("parse");
    let stream = doc.object(4).expect("content stream");
    assert!(
        doc.stream_bytes(stream).is_none(),
        "an unsupported filter is not handed on"
    );
}

#[test]
fn png_row_predictors_reconstruct_every_filter_type() {
    // Each filter tag (None, Sub, Up) reconstructs to the original rows.
    let rows: Vec<Vec<u8>> = (0..5u8)
        .map(|r| {
            (0..6u8)
                .map(|c| r.wrapping_mul(37).wrapping_add(c.wrapping_mul(91)))
                .collect()
        })
        .collect();
    for tag in [0u8, 1, 2] {
        let encoded = png_encode(&rows, tag);
        let raw = zlib_stored(&encoded);
        let mut dict = crate::pdf::parser::Dict::new();
        dict.put(
            "Filter".to_string(),
            Object::Name("FlateDecode".to_string()),
        );
        let mut parms = crate::pdf::parser::Dict::new();
        parms.put("Predictor".to_string(), Object::Int(12));
        parms.put("Columns".to_string(), Object::Int(6));
        dict.put("DecodeParms".to_string(), Object::Dict(parms));
        let stream = Object::Stream(dict, raw);
        let doc = Document::default();
        let got = doc.stream_bytes(&stream).expect("predicted stream decodes");
        let want: Vec<u8> = rows.concat();
        assert_eq!(got, want, "tag {tag}");
    }
}

#[test]
fn bit_flips_never_panic_or_hang() {
    // Every single-bit flip of a small PDF with each structure (classic table, xref stream with
    // object stream, predicted xref stream) parses without panicking, and quickly. The parser is
    // called directly: a panic fails the test, and nothing here relies on `catch_unwind`.
    let bases = [
        classic_pdf(&page_objects("flip"), 1, &[]),
        xref_stream_pdf(false),
        xref_stream_pdf(true),
    ];
    let started = Instant::now();
    let mut runs = 0usize;
    for base in &bases {
        for pos in 0..base.len() {
            for bit in 0..8 {
                let mut pdf = base.clone();
                pdf[pos] ^= 1 << bit;
                if let Ok(doc) = Document::parse(&pdf) {
                    let _ = doc.markdown();
                }
                runs += 1;
            }
        }
    }
    assert!(runs > 1000);
    assert!(
        started.elapsed().as_secs() < 120,
        "bit-flip sweep took {:?}",
        started.elapsed()
    );
}

#[test]
fn garbage_never_panics() {
    for seed in 0..64u32 {
        let mut x = seed.wrapping_mul(2654435761).wrapping_add(1);
        let mut buf = b"%PDF-1.7\n".to_vec();
        for _ in 0..4096 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            buf.push(
                b"<<>>[]/ 0123456789 obj endobj stream endstream xref trailer startxref R\n"
                    [(x as usize) % 72],
            );
        }
        if let Ok(doc) = Document::parse(&buf) {
            let _ = doc.markdown();
        }
    }
}

#[test]
fn ascii85_and_asciihex_stages_decode_before_flate() {
    // "Hello World!" in ASCII85, with the `~>` end marker.
    let text = b"87cURD]i,\"Ebo80~>";
    let mut dict = crate::pdf::parser::Dict::new();
    dict.put(
        "Filter".to_string(),
        Object::Name("ASCII85Decode".to_string()),
    );
    assert_eq!(
        Document::default()
            .stream_bytes(&Object::Stream(dict, text.to_vec()))
            .as_deref(),
        Some(&b"Hello World!"[..])
    );
    // ASCIIHex with white space and a `>` end marker, then RunLength with a repeat and a literal.
    let mut hex = crate::pdf::parser::Dict::new();
    hex.put(
        "Filter".to_string(),
        Object::Name("ASCIIHexDecode".to_string()),
    );
    assert_eq!(
        Document::default()
            .stream_bytes(&Object::Stream(hex, b"4 8 6 5 6C6C6F>".to_vec()))
            .as_deref(),
        Some(&b"Hello"[..])
    );
    let mut rl = crate::pdf::parser::Dict::new();
    rl.put(
        "Filter".to_string(),
        Object::Name("RunLengthDecode".to_string()),
    );
    assert_eq!(
        Document::default()
            .stream_bytes(&Object::Stream(rl, vec![255, b'a', 0, b'z', 128]))
            .as_deref(),
        Some(&b"aaz"[..])
    );
}

#[test]
fn huge_text_coordinates_do_not_size_allocations() {
    // Text placed at x = 1e11 would size the column projection by the page's coordinate span. The
    // span is refused before any buffer is sized from it: the document still extracts, no abort.
    let mut objs = page_objects("x");
    let content = "BT 0 0 Td (Left) Tj 100000000000 0 Td (Far) Tj ET \
                   BT 0 -20 Td (Under) Tj 100000000000 -20 Td (Edge) Tj ET";
    objs[3] = (
        4,
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        )
        .into_bytes(),
    );
    let pdf = classic_pdf(&objs, 1, &[]);
    let md = text_of(&pdf).expect("a malformed coordinate extracts without aborting");
    assert!(md.contains("Left"), "markdown: {md:?}");
}

/// A deterministic generator for the mutation tests (xorshift64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// The bytes of a fixture under `tests/fixtures`.
fn fixture(rel: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

/// Every `.pdf` directly under `tests/fixtures` and under `tests/fixtures/regression`, by name.
fn fixture_pdfs() -> Vec<String> {
    let mut names = Vec::new();
    for dir in ["", "regression"] {
        let path = format!("{}/tests/fixtures/{dir}", env!("CARGO_MANIFEST_DIR"));
        for entry in std::fs::read_dir(&path).expect("fixtures directory") {
            let name = entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned();
            if name.ends_with(".pdf") {
                names.push(if dir.is_empty() {
                    name
                } else {
                    format!("{dir}/{name}")
                });
            }
        }
    }
    names.sort();
    names
}

#[test]
fn literal_string_cut_off_by_end_of_file_does_not_panic() {
    // The smoke.pdf fuzz shape: an object whose last token is a literal string ending in a
    // backslash at the end of the input. The lexer stepped past the end and the next slice panicked.
    let pdf = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Title (cut\\".to_vec();
    let doc = Document::parse(&pdf).expect("a truncated object still parses");
    let _ = doc.markdown();
    let hex = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Title <41\\".to_vec();
    let _ = Document::parse(&hex)
        .expect("a truncated hex string still parses")
        .markdown();
}

#[test]
fn regression_set_loads_through_the_xref_chain() {
    // DoD 2: these files used to fail with "PDF has no readable catalog". Each must read through
    // startxref, the /Prev chain and the xref sections, with the catalog loaded from the chain
    // (not from header recovery), and extract text.
    for name in [
        "doc.pdf",
        "paper-html2pdf.pdf",
        "paper-syndesi.pdf",
        "research-agent-loop.pdf",
        "webfold-benchmarks.pdf",
    ] {
        let bytes = fixture(&format!("regression/{name}"));
        let mut l = Loader::new(&bytes);
        assert!(
            startxref(&bytes).is_some_and(|off| l.load_chain(off)),
            "{name}: xref chain"
        );
        let root = l
            .root_num()
            .unwrap_or_else(|| panic!("{name}: trailer names /Root"));
        let mut doc = Document::default();
        l.load_all(&mut doc);
        assert!(
            matches!(doc.object(root), Some(Object::Dict(d)) if matches!(d.get(&"Type".to_string()), Some(Object::Name(t)) if t == "Catalog")),
            "{name}: the catalog {root} loads from the chain"
        );
        let md = Document::parse(&bytes).expect("parses").markdown();
        assert!(!md.trim().is_empty(), "{name}: non-empty text");
    }
    let bytes = fixture("regression/doc.pdf");
    let mut l = Loader::new(&bytes);
    assert!(startxref(&bytes).is_some_and(|off| l.load_chain(off)));
    let mut doc = Document::default();
    l.load_all(&mut doc);
    assert!(
        !l.streams.is_empty(),
        "doc.pdf's objects are read from its /ObjStm"
    );
}

#[test]
fn smoke_pdf_yields_the_poppler_line_set() {
    // DoD 3: the text lines of smoke.pdf match `pdftotext` (smoke.expected.txt), ignoring the
    // markdown heading marker.
    let bytes = fixture("regression/smoke.pdf");
    let want = String::from_utf8(fixture("regression/smoke.expected.txt")).expect("utf8");
    let got = Document::parse(&bytes).expect("parses").markdown();
    let lines = |s: &str| -> Vec<String> {
        s.lines()
            .map(|l| l.trim().trim_start_matches('#').trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    };
    assert_eq!(lines(&got), lines(&want), "markdown: {got:?}");
}

/// A PDF whose xref stream (object 3) has `count` type-2 rows that all name object stream 2. Stream
/// 2 is FlateDecode of 4 MiB of spaces with `/First 9999`, past its decoded length, so its header
/// is refused.
fn refused_objstm_pdf(count: usize) -> Vec<u8> {
    let mut out = b"%PDF-1.5\n".to_vec();
    out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog >>\nendobj\n");
    let data = vec![b' '; 4 << 20];
    let z = zlib_stored(&data);
    let off2 = out.len();
    out.extend_from_slice(
        format!(
            "2 0 obj\n<< /Type /ObjStm /N 2 /First 9999 /Filter /FlateDecode /Length {} >>\nstream\n",
            z.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&z);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    let off3 = out.len();
    let size = count + 4;
    let mut rows = Vec::new();
    rows.extend(row_bytes([1, 4, 2], [0, 0, 0]));
    rows.extend(row_bytes([1, 4, 2], [1, 9, 0]));
    rows.extend(row_bytes([1, 4, 2], [1, off2 as u64, 0]));
    rows.extend(row_bytes([1, 4, 2], [1, off3 as u64, 0]));
    for i in 0..count {
        rows.extend(row_bytes([1, 4, 2], [2, 2, (i % 2) as u64]));
    }
    out.extend_from_slice(
        format!(
            "3 0 obj\n<< /Type /XRef /Size {size} /W [1 4 2] /Index [0 {size}] /Root 1 0 R /Length {} >>\nstream\n",
            rows.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&rows);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{off3}\n%%EOF\n").as_bytes());
    out
}

#[test]
fn refused_object_stream_is_decoded_once() {
    // Many type-2 entries name one refused object stream: its decode and its refusal are cached,
    // so the 4 MiB stream is inflated once, not once per entry.
    let pdf = refused_objstm_pdf(2000);
    let started = Instant::now();
    let mut l = Loader::new(&pdf);
    assert!(startxref(&pdf).is_some_and(|off| l.load_chain(off)));
    let mut doc = Document::default();
    l.load_all(&mut doc);
    assert_eq!(
        l.decode_attempts, 2,
        "the xref stream and the object stream, each once"
    );
    assert!(doc.object(1).is_some(), "the direct catalog still loads");
    assert!(
        doc.object(4).is_none(),
        "an object from the refused stream does not load"
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "took {:?}",
        started.elapsed()
    );
}

#[test]
fn document_decode_budget_bounds_inflation() {
    // With a 1 MiB budget, the 4 MiB object stream's inflate is refused and charged its cap, so the
    // budget is spent and a later reference does not inflate again.
    let pdf = refused_objstm_pdf(3);
    let mut l = Loader::new(&pdf);
    l.decode_budget = 1 << 20;
    assert!(startxref(&pdf).is_some_and(|off| l.load_chain(off)));
    let mut doc = Document::default();
    l.load_all(&mut doc);
    assert_eq!(l.decode_budget, 0, "the failed inflate is charged its cap");
    assert!(l.streams.is_empty(), "no object stream was produced");
    let before = l.decode_attempts;
    assert!(
        l.load_compressed(4, 2, 0).is_none(),
        "a refused stream stays refused"
    );
    assert_eq!(l.decode_attempts, before, "and it is not decoded again");
}

#[test]
fn stale_generation_contents_and_kids_contribute_no_text() {
    // The file holds object 4 at generation 0 and object 3 at generation 0. A reference that names
    // generation 1 (a stale pointer left over from an earlier revision) must resolve to null, so
    // neither its content stream nor its page-tree node contributes text (7.3.10).
    let mut stale_contents = page_objects("STALE CONTENTS");
    stale_contents[2].1 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
        /Contents 4 1 R /Resources << /Font << /F1 5 0 R >> >> >>"
        .to_vec();
    let md = text_of(&classic_pdf(&stale_contents, 1, &[])).unwrap_or_default();
    assert!(
        !md.contains("STALE CONTENTS"),
        "stale /Contents read: {md:?}"
    );

    let mut stale_kids = page_objects("STALE KIDS");
    stale_kids[1].1 = b"<< /Type /Pages /Kids [3 1 R] /Count 1 >>".to_vec();
    let md = text_of(&classic_pdf(&stale_kids, 1, &[])).unwrap_or_default();
    assert!(!md.contains("STALE KIDS"), "stale /Kids read: {md:?}");

    // Control: the same document with generation 0 references does yield its text.
    let fresh = text_of(&classic_pdf(&page_objects("FRESH TEXT"), 1, &[])).expect("fresh PDF");
    assert!(fresh.contains("FRESH TEXT"), "control markdown: {fresh:?}");
}

#[test]
fn indirect_decode_parms_are_resolved_or_refused() {
    // A FlateDecode content stream whose /DecodeParms is an indirect reference. Resolved to a
    // dictionary, the parameters apply and the text is read. Resolved to a non-dictionary, the
    // stream refuses: its predictor is unknown, so no predicted bytes are passed on.
    let content = b"BT /F1 12 Tf 72 700 Td (PARMS TEXT) Tj ET";
    let flate = zlib_stored(content);
    let stream = |parms: &str| {
        let mut body = format!(
            "<< /Length {} /Filter /FlateDecode /DecodeParms {parms} >>\nstream\n",
            flate.len()
        )
        .into_bytes();
        body.extend_from_slice(&flate);
        body.extend_from_slice(b"\nendstream");
        body
    };
    let objects = |parms_obj: Vec<u8>| {
        let mut objs = page_objects("unused");
        objs[2].1 = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
            /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_vec();
        objs[3].1 = stream("6 0 R");
        objs.push((6, parms_obj));
        objs
    };
    let direct = text_of(&classic_pdf(
        &objects(b"<< /Predictor 1 >>".to_vec()),
        1,
        &[],
    ))
    .unwrap_or_default();
    assert!(direct.contains("PARMS TEXT"), "resolved dict: {direct:?}");
    let refused = text_of(&classic_pdf(&objects(b"42".to_vec()), 1, &[])).unwrap_or_default();
    assert!(
        !refused.contains("PARMS TEXT"),
        "non-dict parms decoded: {refused:?}"
    );
}

/// The files the mutation sweep starts from: every repo fixture and regression file, then, when
/// `SUPERDOCUMENTS_FUZZ_CORPUS` names a directory, every `.pdf` directly inside it (the full
/// comparison corpus, which is too large to commit). Sorted, so a run is reproducible.
fn fuzz_corpus() -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = fixture_pdfs()
        .into_iter()
        .map(|name| {
            let bytes = fixture(&name);
            (name, bytes)
        })
        .collect();
    if let Ok(dir) = std::env::var("SUPERDOCUMENTS_FUZZ_CORPUS") {
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .expect("SUPERDOCUMENTS_FUZZ_CORPUS directory")
            .map(|e| e.expect("corpus entry").path())
            .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
            .collect();
        paths.sort();
        for path in paths {
            let bytes = std::fs::read(&path).expect("corpus file");
            out.push((path.display().to_string(), bytes));
        }
    }
    out
}

#[test]
fn corpus_mutations_never_panic_or_hang() {
    // DoD 4: seeded bit flips, byte overwrites, truncations and splices over every fixture and the
    // regression set (and the `SUPERDOCUMENTS_FUZZ_CORPUS` directory when set). No `catch_unwind`:
    // a panic or a slow case fails the test directly.
    let started = Instant::now();
    let mut rng = Rng(777);
    let mut runs = 0usize;
    for (name, base) in fuzz_corpus() {
        for _ in 0..60 {
            let mut pdf = base.clone();
            match rng.below(4) {
                0 => {
                    let pos = rng.below(pdf.len());
                    pdf[pos] ^= 1 << rng.below(8);
                }
                1 => {
                    let pos = rng.below(pdf.len());
                    pdf[pos] = rng.next() as u8;
                }
                2 => pdf.truncate(rng.below(pdf.len())),
                _ => {
                    let from = rng.below(pdf.len());
                    let len = rng.below(pdf.len() - from).min(256);
                    let to = rng.below(pdf.len());
                    let piece = pdf[from..from + len].to_vec();
                    let end = (to + piece.len()).min(pdf.len());
                    pdf[to..end].copy_from_slice(&piece[..end - to]);
                }
            }
            let case = Instant::now();
            if let Ok(doc) = Document::parse(&pdf) {
                let _ = doc.markdown();
            }
            assert!(
                case.elapsed() < Duration::from_secs(10),
                "{name} run {runs} took {:?}",
                case.elapsed()
            );
            runs += 1;
        }
    }
    assert!(runs >= 500, "ran {runs} mutations");
    assert!(
        started.elapsed() < Duration::from_secs(300),
        "sweep took {:?}",
        started.elapsed()
    );
}
