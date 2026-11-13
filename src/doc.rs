//! Legacy Word binary (`.doc`, Word 97 to 2003 and the older Word 6/95 layout): the text of the
//! main document, read straight from the OLE compound file.
//!
//! What is read: the `WordDocument` stream's FIB, the piece table (`Clx`) in the `0Table` or
//! `1Table` stream and the main-document characters it addresses (8-bit or UTF-16 pieces). Field
//! instructions are dropped and field results kept; table cells are rebuilt from the cell marks
//! and rendered as Markdown tables. Not read: styles (so no headings or lists), character
//! formatting, headers, footers, footnotes, text boxes, embedded objects, pictures. An encrypted
//! file is refused by name.
//!
//! Everything is bounded: the container is read from memory, sector chains are followed at most
//! once per sector, and a stream is never larger than the file.

use std::collections::HashSet;

const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const FREE: u32 = 0xFFFF_FFFF;
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
const MAX_SECTOR: u32 = 0xFFFF_FFFA;
/// Largest main-document text accepted, in characters.
const MAX_CHARS: usize = 64 * 1024 * 1024;

/// Convert a `.doc` file's bytes to Markdown. The content is sniffed, not the extension trusted:
/// a `.doc` that is really RTF or OOXML is handed to those readers.
pub fn convert(data: &[u8]) -> Result<String, String> {
    if data.starts_with(b"PK\x03\x04") {
        return crate::ooxml::docx::convert(data);
    }
    if data.starts_with(b"{\\rtf") {
        return crate::rtf::convert(data);
    }
    if !data.starts_with(&OLE_MAGIC) {
        return Err(
            "not a legacy Word (.doc) file: the content is neither an OLE compound file, RTF \
             nor OOXML"
                .into(),
        );
    }
    let text = word_text(data)?;
    Ok(render(&text))
}

/// The main-document characters of a Word binary file, control characters intact.
pub fn word_text(data: &[u8]) -> Result<String, String> {
    let cfb = Cfb::parse(data).map_err(|e| format!("legacy Word (.doc) file unreadable: {e}"))?;
    let word = cfb.stream("WordDocument").ok_or(
        "OLE compound file without a WordDocument stream: not a Word binary document \
         (a legacy Excel or PowerPoint file saved as .doc?)",
    )?;
    let u16_at = |b: &[u8], o: usize| b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
    let u32_at = |b: &[u8], o: usize| {
        b.get(o..o + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    if u16_at(&word, 0) != Some(0xA5EC) {
        return Err("legacy Word (.doc) file unreadable: the FIB signature is not 0xA5EC".into());
    }
    let n_fib = u16_at(&word, 2).ok_or("truncated Word header")?;
    let flags = u16_at(&word, 0x0A).ok_or("truncated Word header")?;
    if flags & 0x0100 != 0 {
        return Err("encrypted (password-protected) legacy Word (.doc) file: not readable".into());
    }
    if n_fib < 0x00C1 {
        // Word 6 / Word 95: the text is one run of 8-bit characters between fcMin and fcMac.
        let (min, mac) = (
            u32_at(&word, 0x18).ok_or("truncated Word 6/95 header")? as usize,
            u32_at(&word, 0x1C).ok_or("truncated Word 6/95 header")? as usize,
        );
        let bytes = word
            .get(min..mac.min(word.len()))
            .ok_or("Word 6/95 text range outside the file")?;
        return Ok(decode_8bit(bytes));
    }
    // Word 97 and later: FibBase (32 bytes), csw + FibRgW, cslw + FibRgLw, cbRgFcLcb + pairs.
    let csw = u16_at(&word, 32).ok_or("truncated FIB")? as usize;
    let lw = 34 + csw * 2;
    let cslw = u16_at(&word, lw).ok_or("truncated FIB")? as usize;
    let rglw = lw + 2;
    let ccp_text = u32_at(&word, rglw + 3 * 4).ok_or("truncated FIB")? as usize;
    let fclcb = rglw + cslw * 4 + 2;
    let pairs = u16_at(&word, fclcb - 2).ok_or("truncated FIB")? as usize;
    const CLX_PAIR: usize = 33;
    if pairs <= CLX_PAIR {
        return Err("Word FIB has no piece table (fcClx)".into());
    }
    let fc_clx = u32_at(&word, fclcb + CLX_PAIR * 8).ok_or("truncated FIB")? as usize;
    let lcb_clx = u32_at(&word, fclcb + CLX_PAIR * 8 + 4).ok_or("truncated FIB")? as usize;
    let table_name = if flags & 0x0200 != 0 {
        "1Table"
    } else {
        "0Table"
    };
    let table = cfb
        .stream(table_name)
        .ok_or_else(|| format!("Word file without its {table_name} stream"))?;
    let clx = table
        .get(fc_clx..fc_clx.saturating_add(lcb_clx).min(table.len()))
        .ok_or("piece table outside the table stream")?;
    let pieces = piece_table(clx)?;
    let mut out = String::new();
    let mut taken = 0usize;
    let limit = ccp_text.min(MAX_CHARS);
    for p in &pieces {
        if taken >= limit {
            break;
        }
        let n = (p.cp_end.saturating_sub(p.cp_start)).min(limit - taken);
        if p.compressed {
            let start = p.fc / 2;
            let bytes = word
                .get(start..(start + n).min(word.len()))
                .ok_or("text piece outside the WordDocument stream")?;
            out.push_str(&decode_8bit(bytes));
        } else {
            let bytes = word
                .get(p.fc..(p.fc + n * 2).min(word.len()))
                .ok_or("text piece outside the WordDocument stream")?;
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            out.push_str(&String::from_utf16_lossy(&units));
        }
        taken += n;
    }
    Ok(out)
}

struct Piece {
    cp_start: usize,
    cp_end: usize,
    /// Byte offset in the WordDocument stream (already halved for 8-bit text is NOT applied here).
    fc: usize,
    compressed: bool,
}

fn piece_table(clx: &[u8]) -> Result<Vec<Piece>, String> {
    let mut i = 0usize;
    // Prc entries (clxt 1) carry property modifiers; skip them.
    while clx.get(i) == Some(&0x01) {
        let cb = clx
            .get(i + 1..i + 3)
            .map(|s| u16::from_le_bytes([s[0], s[1]]) as usize)
            .ok_or("truncated Clx")?;
        i += 3 + cb;
    }
    if clx.get(i) != Some(&0x02) {
        return Err("Word piece table (Pcdt) not found".into());
    }
    let lcb = clx
        .get(i + 1..i + 5)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
        .ok_or("truncated Clx")?;
    let plc = clx
        .get(i + 5..(i + 5).saturating_add(lcb).min(clx.len()))
        .ok_or("truncated piece table")?;
    // PlcPcd: n+1 character positions then n 8-byte Pcd: lcb = 4(n+1) + 8n.
    if plc.len() < 4 {
        return Err("empty piece table".into());
    }
    let n = (plc.len() - 4) / 12;
    let cp = |k: usize| u32::from_le_bytes([plc[4 * k], plc[4 * k + 1], plc[4 * k + 2], plc[4 * k + 3]]);
    let mut pieces = Vec::with_capacity(n);
    for k in 0..n {
        let pcd = 4 * (n + 1) + 8 * k;
        let fc = u32::from_le_bytes([plc[pcd + 2], plc[pcd + 3], plc[pcd + 4], plc[pcd + 5]]);
        let compressed = fc & 0x4000_0000 != 0;
        pieces.push(Piece {
            cp_start: cp(k) as usize,
            cp_end: cp(k + 1) as usize,
            fc: (fc & 0x3FFF_FFFF) as usize,
            compressed,
        });
    }
    Ok(pieces)
}

fn decode_8bit(bytes: &[u8]) -> String {
    encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned()
}

// ── Markdown rendering of the raw character stream ──────────────────────────────────────────

/// Turn the raw main-document characters into Markdown paragraphs and tables.
pub fn render(raw: &str) -> String {
    // Fields: keep the result between 0x14 and 0x15, drop the instruction before 0x14.
    let mut text = String::with_capacity(raw.len());
    let mut depth_stack: Vec<bool> = Vec::new(); // true = inside the instruction part
    for c in raw.chars() {
        match c {
            '\u{13}' => depth_stack.push(true),
            '\u{14}' => {
                if let Some(top) = depth_stack.last_mut() {
                    *top = false;
                }
            }
            '\u{15}' => {
                depth_stack.pop();
            }
            _ if depth_stack.iter().any(|in_instruction| *in_instruction) => {}
            '\u{0B}' => text.push('\n'),
            '\u{0C}' => text.push('\n'),
            '\u{1E}' => text.push('-'),
            '\u{01}' | '\u{02}' | '\u{05}' | '\u{08}' | '\u{1F}' => {}
            c if c.is_control() && !matches!(c, '\r' | '\u{07}' | '\n' | '\t') => {}
            c => text.push(c),
        }
    }
    // Blocks. Every cell and end-of-row mark is a 0x07: split on it. A segment that ends in a mark is
    // a cell; an empty cell right after a cell is the row end; the last segment is plain text.
    // Paragraph breaks (\r) inside a cell become spaces, except where text before the cell's own
    // paragraph is outside any table row, which is emitted as paragraphs before the table.
    let mut blocks: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    fn paragraphs(text: &str, blocks: &mut Vec<String>) {
        for p in text.split('\r') {
            let t = p.trim();
            if !t.is_empty() {
                blocks.push(t.to_string());
            }
        }
    }
    fn close(rows: &mut Vec<Vec<String>>, row: &mut Vec<String>, blocks: &mut Vec<String>) {
        if !row.is_empty() {
            rows.push(std::mem::take(row));
        }
        if !rows.is_empty() {
            blocks.push(table(std::mem::take(rows)));
        }
    }
    let mut segments: Vec<&str> = text.split('\u{07}').collect();
    let tail = segments.pop().unwrap_or("");
    for seg in segments {
        let at_row_start = row.is_empty();
        if seg.is_empty() && !at_row_start {
            rows.push(std::mem::take(&mut row));
            continue;
        }
        let cell = if at_row_start {
            // Text before the cell's own paragraph is not part of any table.
            match seg.rfind('\r') {
                Some(i) => {
                    close(&mut rows, &mut row, &mut blocks);
                    paragraphs(&seg[..i], &mut blocks);
                    &seg[i + 1..]
                }
                None => seg,
            }
        } else {
            seg
        };
        row.push(cell.replace('\r', " ").replace('\n', " ").trim().to_string());
    }
    close(&mut rows, &mut row, &mut blocks);
    paragraphs(tail, &mut blocks);
    blocks.join("\n\n")
}

fn table(rows: Vec<Vec<String>>) -> String {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let mut out = String::new();
    for (i, row) in rows.iter().enumerate() {
        out.push('|');
        for c in 0..width {
            out.push(' ');
            out.push_str(&row.get(c).map_or(String::new(), |s| s.replace('|', "\\|")));
            out.push_str(" |");
        }
        out.push('\n');
        if i == 0 {
            out.push('|');
            for _ in 0..width {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

// ── the OLE compound file ───────────────────────────────────────────────────────────────────

struct Entry {
    name: String,
    kind: u8,
    start: u32,
    size: u64,
}

struct Cfb<'a> {
    data: &'a [u8],
    sector: usize,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    mini_stream: Vec<u8>,
    mini_cutoff: u64,
    entries: Vec<Entry>,
}

impl<'a> Cfb<'a> {
    fn parse(data: &'a [u8]) -> Result<Self, String> {
        let u16_at = |o: usize| data.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]));
        let u32_at = |o: usize| {
            data.get(o..o + 4)
                .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        };
        let short = || "truncated OLE header".to_string();
        let shift = u16_at(0x1E).ok_or_else(short)?;
        if !(7..=16).contains(&shift) {
            return Err(format!("unsupported OLE sector size 2^{shift}"));
        }
        let sector = 1usize << shift;
        let mini_cutoff = u32_at(0x38).ok_or_else(short)? as u64;
        let dir_start = u32_at(0x30).ok_or_else(short)?;
        let mini_fat_start = u32_at(0x3C).ok_or_else(short)?;
        let mut difat_sector = u32_at(0x44).ok_or_else(short)?;
        let max_sectors = data.len() / sector + 1;
        let sector_bytes = |id: u32| -> Option<&'a [u8]> {
            let start = (id as usize).checked_add(1)?.checked_mul(sector)?;
            data.get(start..start.checked_add(sector)?)
        };
        // DIFAT: 109 entries in the header, then a chain of DIFAT sectors.
        let mut fat_sectors: Vec<u32> = (0..109)
            .filter_map(|k| u32_at(0x4C + 4 * k))
            .filter(|id| *id < MAX_SECTOR)
            .collect();
        let mut seen = HashSet::new();
        while difat_sector < MAX_SECTOR && seen.insert(difat_sector) && seen.len() <= max_sectors {
            let s = sector_bytes(difat_sector).ok_or("DIFAT sector outside the file")?;
            let ids: Vec<u32> = s
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            fat_sectors.extend(ids[..ids.len() - 1].iter().copied().filter(|id| *id < MAX_SECTOR));
            difat_sector = *ids.last().unwrap_or(&END_OF_CHAIN);
        }
        let mut fat: Vec<u32> = Vec::new();
        for id in &fat_sectors {
            let s = sector_bytes(*id).ok_or("FAT sector outside the file")?;
            fat.extend(s.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])));
        }
        let chain = |start: u32, fat: &[u32]| -> Result<Vec<u8>, String> {
            let mut out = Vec::new();
            let (mut id, mut seen) = (start, 0usize);
            while id < MAX_SECTOR {
                seen += 1;
                if seen > max_sectors {
                    return Err("cyclic sector chain".into());
                }
                out.extend_from_slice(sector_bytes(id).ok_or("sector outside the file")?);
                id = *fat.get(id as usize).unwrap_or(&END_OF_CHAIN);
            }
            Ok(out)
        };
        let dir_bytes = chain(dir_start, &fat)?;
        let entries: Vec<Entry> = dir_bytes
            .chunks_exact(128)
            .map(|e| {
                let name_len = (u16::from_le_bytes([e[0x40], e[0x41]]) as usize).min(64);
                let units: Vec<u16> = e[..name_len.saturating_sub(2)]
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                Entry {
                    name: String::from_utf16_lossy(&units),
                    kind: e[0x42],
                    start: u32::from_le_bytes([e[0x74], e[0x75], e[0x76], e[0x77]]),
                    size: u64::from_le_bytes([
                        e[0x78], e[0x79], e[0x7A], e[0x7B], e[0x7C], e[0x7D], e[0x7E], e[0x7F],
                    ]),
                }
            })
            .collect();
        let root = entries.first().ok_or("empty OLE directory")?;
        let (mini_fat, mini_stream) = if mini_fat_start < MAX_SECTOR {
            let mf = chain(mini_fat_start, &fat)?;
            let mut ms = chain(root.start, &fat)?;
            ms.truncate(root.size as usize);
            (
                mf.chunks_exact(4)
                    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect(),
                ms,
            )
        } else {
            (Vec::new(), Vec::new())
        };
        Ok(Cfb {
            data,
            sector,
            fat,
            mini_fat,
            mini_stream,
            mini_cutoff,
            entries,
        })
    }

    fn stream(&self, name: &str) -> Option<Vec<u8>> {
        let e = self.entries.iter().find(|e| e.kind == 2 && e.name == name)?;
        let size = usize::try_from(e.size).ok()?.min(self.data.len());
        let mut out = Vec::with_capacity(size);
        let (mut id, mut hops) = (e.start, 0usize);
        if e.size < self.mini_cutoff {
            while id < MAX_SECTOR && out.len() < size {
                hops += 1;
                if hops > self.mini_stream.len() / 64 + 1 {
                    return None;
                }
                let start = id as usize * 64;
                out.extend_from_slice(self.mini_stream.get(start..start + 64)?);
                id = *self.mini_fat.get(id as usize).unwrap_or(&END_OF_CHAIN);
            }
        } else {
            while id < MAX_SECTOR && out.len() < size {
                hops += 1;
                if hops > self.data.len() / self.sector + 1 {
                    return None;
                }
                let start = (id as usize + 1) * self.sector;
                out.extend_from_slice(self.data.get(start..start + self.sector)?);
                id = *self.fat.get(id as usize).unwrap_or(&END_OF_CHAIN);
            }
        }
        out.truncate(size);
        let _ = FREE;
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_splits_paragraphs_and_keeps_field_results() {
        let raw = "Title\rSee \u{13} HYPERLINK \"x\" \u{14}the page\u{15} now\rlast";
        assert_eq!(render(raw), "Title\n\nSee the page now\n\nlast");
    }

    #[test]
    fn render_rebuilds_a_table_from_cell_marks() {
        let raw = "before\ra\u{07}b\u{07}\u{07}c\u{07}d\u{07}\u{07}after";
        assert_eq!(
            render(raw),
            "before\n\n| a | b |\n| --- | --- |\n| c | d |\n\nafter"
        );
    }

    #[test]
    fn convert_names_the_format_for_foreign_content() {
        let err = convert(b"plain text, not a document").unwrap_err();
        assert!(err.contains("legacy Word (.doc)"), "{err}");
    }
}
