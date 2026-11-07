//! Lists the embedded images of a PDF with `superdocuments::pdf::extract_images`, and writes
//! each one to a directory: JPEG as `.jpg`, PNG as `.png`, other data as `.bin`.
//!
//! Usage: `cargo run --example pdf_image_list -- <file.pdf> [out-dir]`

use std::path::PathBuf;

use superdocuments::pdf::{extract_images, ImageEncoding};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: pdf_image_list <file.pdf> [out-dir]");
        std::process::exit(2);
    };
    let out_dir = args.next().map(PathBuf::from);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let images = match extract_images(&bytes) {
        Ok(images) => images,
        Err(e) => {
            eprintln!("extraction failed: {e}");
            std::process::exit(1);
        }
    };
    for (i, img) in images.iter().enumerate() {
        let encoding = match &img.encoding {
            ImageEncoding::Jpeg => "jpeg".to_string(),
            ImageEncoding::Png => "png".to_string(),
            ImageEncoding::RawSamples => "raw".to_string(),
            ImageEncoding::Jpx => "jpx".to_string(),
            ImageEncoding::Unsupported(f) => format!("unsupported({f})"),
        };
        println!(
            "{} {} {} {} {} {} {} {}",
            img.page + 1,
            i,
            img.object,
            img.width,
            img.height,
            img.color_space,
            img.bits_per_component,
            encoding
        );
        if let Some(dir) = &out_dir {
            let ext = match img.encoding {
                ImageEncoding::Jpeg => "jpg",
                ImageEncoding::Png => "png",
                _ => "bin",
            };
            let name = format!("p{}-{}-o{}.{ext}", img.page + 1, i, img.object);
            if let Err(e) = std::fs::write(dir.join(name), &img.data) {
                eprintln!("cannot write image {i}: {e}");
                std::process::exit(1);
            }
        }
    }
}
