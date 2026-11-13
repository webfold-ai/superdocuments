//! Shared by the PDF routing tests: a tiny in-memory PDF writer.
#![allow(dead_code)]

pub const STAMP: &str = "Federal Register of Legislative Instruments F2006B00784";

/// One page of a test PDF.
pub struct Page {
    /// Page size in points; `None` inherits the A4 box set on the `/Pages` node.
    pub size: Option<(u32, u32)>,
    /// Painted image size in points (`w h`), placed at the origin.
    pub image: Option<(u32, u32)>,
    pub text: Option<String>,
}

pub fn page(image: Option<(u32, u32)>, text: Option<&str>) -> Page {
    Page {
        size: None,
        image,
        text: text.map(str::to_string),
    }
}

pub fn build(pages: &[Page]) -> Vec<u8> {
    // 1 catalog, 2 pages, 3 font, 4 image; page i is object 10 + 2i, its content 11 + 2i.
    let mut objs: Vec<(u32, Vec<u8>)> = vec![
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (
            3,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ),
    ];
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", 10 + 2 * i))
        .collect();
    objs.push((
        2,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} /MediaBox [0 0 595 842] >>",
            kids.join(" "),
            pages.len()
        )
        .into_bytes(),
    ));
    let sample = [0x80u8; 64];
    let mut image = b"<< /Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 64 >>\nstream\n".to_vec();
    image.extend_from_slice(&sample);
    image.extend_from_slice(b"\nendstream");
    objs.push((4, image));
    for (i, p) in pages.iter().enumerate() {
        let mut content = String::new();
        if let Some((w, h)) = p.image {
            content.push_str(&format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q\n"));
        }
        if let Some(text) = &p.text {
            content.push_str(&format!("BT /F1 10 Tf 40 30 Td ({text}) Tj ET\n"));
        }
        let media = p
            .size
            .map(|(w, h)| format!("/MediaBox [0 0 {w} {h}] "))
            .unwrap_or_default();
        objs.push((
            10 + 2 * i as u32,
            format!(
                "<< /Type /Page /Parent 2 0 R {media}/Contents {} 0 R /Resources << /Font << /F1 3 0 R >> /XObject << /Im0 4 0 R >> >> >>",
                11 + 2 * i
            )
            .into_bytes(),
        ));
        objs.push((
            11 + 2 * i as u32,
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
            .into_bytes(),
        ));
    }
    objs.sort_by_key(|(n, _)| *n);
    let max = objs.last().unwrap().0;
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0usize; max as usize + 1];
    for (n, body) in &objs {
        offsets[*n as usize] = out.len();
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", max + 1).as_bytes());
    for offset in &offsets[1..] {
        if *offset == 0 {
            out.extend_from_slice(b"0000000000 65535 f \n");
        } else {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            max + 1
        )
        .as_bytes(),
    );
    out
}

pub const FULL_PAGE: Option<(u32, u32)> = Some((595, 842));

/// A page of real-looking running text, well above the sparse-text threshold on A4.
pub fn dense_text() -> String {
    "The quick brown fox jumps over the lazy dog while the Administrator considers the schedule. "
        .repeat(5)
}
