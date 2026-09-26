//! PDF-to-Markdown process adapter. The PDF parser remains an external `pdffold`
//! executable until its implementation is available as a library dependency.
use std::io;
use std::path::Path;
use std::process::Command;

/// Convert a PDF path using `PDFFOLD_BIN`, `PDFFOLD_MANIFEST`, or `pdffold` on `PATH`.
pub fn to_markdown(path: impl AsRef<Path>) -> io::Result<String> {
    to_markdown_with_manifest(path, None)
}

/// Convert a PDF path while retaining an optional legacy Cargo-manifest fallback.
/// Explicit `PDFFOLD_BIN` takes priority, then `manifest`, then `PDFFOLD_MANIFEST`,
/// and finally an installed `pdffold` executable.
pub fn to_markdown_with_manifest(
    path: impl AsRef<Path>,
    manifest: Option<&Path>,
) -> io::Result<String> {
    let path = path.as_ref();
    if let Ok(bin) = std::env::var("PDFFOLD_BIN") {
        return run(Command::new(bin).arg(path));
    }
    let manifest = manifest
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("PDFFOLD_MANIFEST").map(Into::into));
    if let Some(manifest) = manifest {
        let mut command = Command::new("cargo");
        command
            .args(["run", "--quiet", "--example", "md", "--manifest-path"])
            .arg(manifest)
            .arg("--")
            .arg(path);
        return run(&mut command);
    }
    run(Command::new("pdffold").arg(path))
}

fn run(command: &mut Command) -> io::Result<String> {
    let output = command.output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        Err(io::Error::other(error.trim().to_string()))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn process_adapter_returns_stdout_and_surfaces_failures() {
        let mut success = Command::new("sh");
        success.args(["-c", "printf converted"]);
        assert_eq!(run(&mut success).unwrap(), "converted");

        let mut failure = Command::new("sh");
        failure.args(["-c", "printf failed >&2; exit 7"]);
        assert_eq!(run(&mut failure).unwrap_err().to_string(), "failed");
    }
}
