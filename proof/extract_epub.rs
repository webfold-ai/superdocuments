//! EPUB package, manifest, spine, container-path and bounds kernels for the Charon/Aeneas
//! extraction (G12).
//!
//! Each function is loop-free and returns its decision as a `bool`, `u8`, `u32` or `u64`
//! value (or a `(bool, value)` pair), so the extraction is total. `proof/regen.sh` extracts
//! exactly these nine names into `EpubMath.lean`; `EpubProofs.lean` proves each equals its
//! field function in `EpubSpec.lean`. Production (`src/epub.rs`) includes this file with
//! `#[path]` and calls the same functions on its decision path. The loops that walk the
//! package, the spine, the path segments and the percent-encoding stay in Rust; they are the
//! recursions named in `EpubSpec.lean`. Changing a signature or a contract is a specification
//! change.

/// `opf_child_role`: the role of an element named `name` (`1` package, `2` manifest, `3`
/// spine, `4` item, `5` itemref, `0` any other local name) under a parent of role `parent`
/// (`0` document, `1` package, `2` manifest, `3` spine, `4`/`5`/`6` leaf or ignored).
/// Roles: `1` package, `2` manifest, `3` spine, `4` manifest item, `5` spine itemref,
/// `6` ignored. An element outside the OPF namespace is ignored.
pub fn opf_child_role(parent: u8, name: u8, opf_ns: bool) -> u8 {
    if !opf_ns {
        6
    } else if parent == 0 {
        if name == 1 {
            1
        } else {
            6
        }
    } else if parent == 1 {
        if name == 2 {
            2
        } else if name == 3 {
            3
        } else {
            6
        }
    } else if parent == 2 {
        if name == 4 {
            4
        } else {
            6
        }
    } else if parent == 3 {
        if name == 5 {
            5
        } else {
            6
        }
    } else {
        6
    }
}

/// `manifest_item_accept`: an `item` enters the manifest only with its required `id` and
/// `href` attributes and a non-empty id.
pub fn manifest_item_accept(has_id: bool, has_href: bool, id_len: u64) -> bool {
    has_id && has_href && id_len > 0
}

/// `manifest_index_step`: the `(first index, duplicated)` pair written for manifest entry `k`,
/// given whether the table already holds its id (`present`) and the stored first index.
pub fn manifest_index_step(present: bool, prev_index: u32, k: u32) -> (u32, bool) {
    if present {
        (prev_index, true)
    } else {
        (k, false)
    }
}

/// `spine_step`: the code of one `itemref`. Codes: `0` read, `1` missing, `2` duplicate,
/// `3` repeated, `4` foreign, `5` corrupt (the table answered out of range or with an entry
/// that has another id). The kernel reads only a found, unduplicated, in-range entry whose
/// stored id equals the idref, that is a content document and was not read before.
pub fn spine_step(
    found: bool,
    dup: bool,
    index: u32,
    count: u32,
    id_eq: bool,
    content_doc: bool,
    seen: bool,
) -> u8 {
    if !found {
        1
    } else if dup {
        2
    } else if !(index < count && id_eq) {
        5
    } else if !content_doc {
        4
    } else if seen {
        3
    } else {
        0
    }
}

/// `href_byte_class`: `1` the separator `/`, `2` `%`, `3` `#` or `?` (the path ends), `0`
/// any other byte.
pub fn href_byte_class(b: u8) -> u8 {
    if b == 47 {
        1
    } else if b == 37 {
        2
    } else if b == 35 || b == 63 {
        3
    } else {
        0
    }
}

/// `pct_hex`: whether `b` is an ASCII hex digit, and its value (`0` when it is not).
pub fn pct_hex(b: u8) -> (bool, u8) {
    if 48 <= b && b <= 57 {
        (true, b - 48)
    } else if 65 <= b && b <= 70 {
        (true, b - 55)
    } else if 97 <= b && b <= 102 {
        (true, b - 87)
    } else {
        (false, 0)
    }
}

/// `path_segment_kind`: `1` the segment `.`, `2` the segment `..`, `0` any other segment.
/// `len` is the segment length, `b0` and `b1` its first two bytes (`0` when absent).
pub fn path_segment_kind(len: u64, b0: u8, b1: u8) -> u8 {
    if len == 1 && b0 == 46 {
        1
    } else if len == 2 && b0 == 46 && b1 == 46 {
        2
    } else {
        0
    }
}

/// `epub_text_budget`: the joined text grows by the separator and the chapter only when the
/// result stays within `max`. Refusal keeps `out_len`.
pub fn epub_text_budget(out_len: u64, sep_len: u64, md_len: u64, max: u64) -> (bool, u64) {
    if out_len <= max {
        let room = max - out_len;
        if sep_len <= room {
            let room2 = room - sep_len;
            if md_len <= room2 {
                (true, out_len + sep_len + md_len)
            } else {
                (false, out_len)
            }
        } else {
            (false, out_len)
        }
    } else {
        (false, out_len)
    }
}

/// `zip_entry_bound`: a ZIP entry is accepted only when both its declared size and the number
/// of bytes actually read are within `max`.
pub fn zip_entry_bound(declared: u64, read: u64, max: u64) -> bool {
    declared <= max && read <= max
}
