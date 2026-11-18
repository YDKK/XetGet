//! Checks for repository-controlled filenames before using them as local paths.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use crate::error::{HFError, HFResult};

pub(super) fn validate_filename(filename: &str) -> HFResult<()> {
    // Check both platforms' syntax, even when the client is running on Unix.
    // Splitting also detects dot/empty segments that Path::components normalizes.
    if filename.is_empty()
        || filename.chars().any(|c| c.is_control() || matches!(c, '\\' | ':'))
        || filename
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || Path::new(filename)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(HFError::InvalidParameter(format!(
            "unsafe download filename: {filename:?}"
        )));
    }
    #[cfg(windows)]
    for part in filename.split('/') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches(' ')
            .to_ascii_uppercase();
        if part.ends_with(['.', ' '])
            || part.contains(['<', '>', '"', '|', '?', '*'])
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.chars().count() == 4
                && matches!(stem.chars().last(), Some('1'..='9' | '¹' | '²' | '³')))
        {
            return Err(HFError::InvalidParameter(format!(
                "unsafe Windows download filename: {filename:?}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn local_download_path(local_dir: &Path, filename: &str) -> HFResult<PathBuf> {
    validate_filename(filename)?;
    // The caller chooses local_dir; links below that root are not repository paths.
    // This rejects existing links, not concurrent changes by another local process.
    let mut path = local_dir.to_path_buf();
    for part in filename.split('/') {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if is_link(&metadata) => {
                return Err(HFError::InvalidParameter(format!(
                    "download path contains a link: {}",
                    path.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(path)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes junctions and other reparse points, not just symbolic links.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
