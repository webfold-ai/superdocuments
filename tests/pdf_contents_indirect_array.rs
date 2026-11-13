//! `/Contents` may be an indirect reference to an ARRAY of content-stream references (ISO 32000-1 7.7.3.3 allows
//! "a stream or an array of streams", and any value may be an indirect object). The Federal Register of Legislation's
//! Aspose.Pdf 10.1.0 files write `/Contents 13 0 R` with `13 0 obj [14 0 R 15 0 R 16 0 R] endobj`; the native parser
//! used to read no text from them, so webfold fell back to OCR (or timed out in it).

fn build(contents_entry: &str, objs_tail: &[(&str, Vec<u8>)]) -> Vec<u8> {
    // objects 1..=3 fixed: catalog, pages, page; then the tail objects (numbered from 4), then the font.
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents {contents_entry} /Resources << /Font << /F1 99 0 R >> >> >>").into_bytes(),
    ];
    for (_, body) in objs_tail {
        objs.push(body.clone());
    }
    while objs.len() < 98 {
        objs.push(b"null".to_vec());
    }
    objs.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in &offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn stream(text: &str) -> Vec<u8> {
    let s = format!(
        "BT /F1 12 Tf 72 {} Td ({text}) Tj ET",
        if text.starts_with("One") {
            700
        } else if text.starts_with("Two") {
            650
        } else {
            600
        }
    );
    format!("<< /Length {} >>\nstream\n{s}\nendstream", s.len()).into_bytes()
}

fn words(pdf: &[u8]) -> String {
    superdocuments::pdf::extract_markdown(pdf).expect("native extraction must not error")
}

#[test]
fn contents_as_direct_array_still_works() {
    let pdf = build(
        "[4 0 R 5 0 R]",
        &[("s1", stream("One alpha")), ("s2", stream("Two bravo"))],
    );
    let md = words(&pdf);
    assert!(
        md.contains("One alpha") && md.contains("Two bravo"),
        "{md:?}"
    );
}

#[test]
fn contents_as_single_stream_still_works() {
    let pdf = build("4 0 R", &[("s1", stream("One alpha"))]);
    assert!(words(&pdf).contains("One alpha"));
}

#[test]
fn contents_as_indirect_array_is_read() {
    // object 6 is the array; 4 and 5 are the streams (the Aspose.Pdf layout)
    let pdf = build(
        "6 0 R",
        &[
            ("s1", stream("One alpha")),
            ("s2", stream("Two bravo")),
            ("arr", b"[4 0 R 5 0 R]".to_vec()),
        ],
    );
    let md = words(&pdf);
    assert!(
        md.contains("One alpha") && md.contains("Two bravo"),
        "indirect /Contents array gave no text: {md:?}"
    );
}

#[test]
fn indirect_array_may_hold_a_reference_to_a_missing_stream() {
    let pdf = build(
        "6 0 R",
        &[
            ("s1", stream("One alpha")),
            ("s2", stream("Two bravo")),
            ("arr", b"[4 0 R 77 0 R 5 0 R]".to_vec()),
        ],
    );
    let md = words(&pdf);
    assert!(
        md.contains("One alpha") && md.contains("Two bravo"),
        "{md:?}"
    );
}
