use std::{collections::BTreeMap, fs, path::Path, process::Command};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};

type Sources = BTreeMap<String, Vec<u8>>;

#[derive(Deserialize)]
struct Manifest {
    version: String,
    source_sha256: String,
    patches: Vec<String>,
}

#[test]
fn hf_hub_patches_match_upstream() -> Result<()> {
    check_patches(Path::new(env!("CARGO_MANIFEST_DIR")))
}

fn check_patches(root: &Path) -> Result<()> {
    let patches = root.join("patches/hf-hub");
    let manifest: Manifest = serde_json::from_slice(&fs::read(patches.join("upstream.json"))?)?;
    let original = source_files(&root.join("vendor/hf-hub"))?;
    let temp_root = std::env::temp_dir().canonicalize()?;
    let work = tempfile::Builder::new()
        .prefix("xetget-vendor-check-")
        .tempdir_in(&temp_root)?;
    let result = verify_sources(work.path(), &patches, &manifest, &original);
    // Attempt cleanup on failure too, and report a failure to remove temporary files.
    work.close()
        .context("cannot remove temporary patch-check directory")?;
    result?;
    println!(
        "Verified hf-hub {}: upstream source and patch round-trip match",
        manifest.version
    );
    Ok(())
}

fn verify_sources(
    work: &Path,
    patches: &Path,
    manifest: &Manifest,
    original: &Sources,
) -> Result<()> {
    for (name, data) in original {
        let target = work.join(name);
        fs::create_dir_all(target.parent().context("source file has no parent")?)?;
        fs::write(target, data)?;
    }
    run_git(Command::new("git").args(["init", "--quiet"]).arg(work))?;
    for patch in manifest.patches.iter().rev() {
        apply_patch(work, &patches.join(patch), true)?;
    }
    ensure!(
        source_hash(&source_files(work)?) == manifest.source_sha256,
        "vendored sources differ from the recorded upstream baseline"
    );
    for patch in &manifest.patches {
        apply_patch(work, &patches.join(patch), false)?;
    }
    ensure!(
        source_files(work)? == *original,
        "applying the patches did not reproduce the vendored sources"
    );
    Ok(())
}

fn source_files(root: &Path) -> Result<Sources> {
    fn visit(directory: &Path, prefix: &str, files: &mut Sources) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().context("source filename is not UTF-8")?;
            let relative = format!("{prefix}/{name}");
            let kind = entry.file_type()?;
            if kind.is_dir() {
                visit(&entry.path(), &relative, files)?;
            } else {
                ensure!(kind.is_file(), "source is not a regular file: {relative}");
                let data = fs::read(entry.path())?;
                // Match the recorded hash on both LF and CRLF checkouts.
                let normalized = data
                    .iter()
                    .enumerate()
                    .filter_map(|(index, &byte)| {
                        (byte != b'\r' || data.get(index + 1) != Some(&b'\n')).then_some(byte)
                    })
                    .collect();
                files.insert(relative, normalized);
            }
        }
        Ok(())
    }

    let mut files = BTreeMap::new();
    visit(&root.join("src"), "src", &mut files)?;
    Ok(files)
}

fn source_hash(files: &Sources) -> String {
    let mut digest = Sha256::new();
    for (name, data) in files {
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update((data.len() as u64).to_be_bytes());
        digest.update(data);
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn apply_patch(work: &Path, patch: &Path, reverse: bool) -> Result<()> {
    let mut command = Command::new("git");
    command
        .current_dir(work)
        .args(["-c", "core.autocrlf=false", "apply"]);
    if reverse {
        command.arg("--reverse");
    }
    run_git(command.arg(patch))
        .with_context(|| format!("cannot apply patch {} (reverse={reverse})", patch.display()))
}

fn run_git(command: &mut Command) -> Result<()> {
    let output = command.output().context("cannot start git")?;
    ensure!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn fixture() -> Result<tempfile::TempDir> {
    let root = tempfile::Builder::new()
        .prefix("xetget-patch-fixture-")
        .tempdir()?;
    let vendor = root.path().join("vendor/hf-hub/src");
    let patches = root.path().join("patches/hf-hub");
    fs::create_dir_all(&vendor)?;
    fs::create_dir_all(&patches)?;
    fs::write(vendor.join("lib.rs"), b"patched\r\n")?;
    fs::write(
        patches.join("test.patch"),
        "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-upstream\n+patched\n",
    )?;
    let original = BTreeMap::from([("src/lib.rs".to_string(), b"upstream\n".to_vec())]);
    let manifest = serde_json::json!({
        "version": "fixture",
        "source_sha256": source_hash(&original),
        "patches": ["test.patch"],
    });
    fs::write(
        patches.join("upstream.json"),
        serde_json::to_vec(&manifest)?,
    )?;
    Ok(root)
}

#[test]
fn rejects_unrecorded_source_changes() -> Result<()> {
    let root = fixture()?;
    // A valid CRLF checkout must pass before we introduce an unrecorded file.
    check_patches(root.path())?;
    fs::write(
        root.path().join("vendor/hf-hub/src/unrecorded.rs"),
        b"extra\n",
    )?;
    let error = check_patches(root.path()).unwrap_err();
    ensure!(
        error.to_string().contains("recorded upstream baseline"),
        "{error:#}"
    );
    Ok(())
}

#[test]
fn rejects_patch_that_does_not_apply() -> Result<()> {
    let root = fixture()?;
    fs::write(root.path().join("vendor/hf-hub/src/lib.rs"), b"different\n")?;
    let error = check_patches(root.path()).unwrap_err();
    ensure!(
        error.to_string().contains("cannot apply patch"),
        "{error:#}"
    );
    Ok(())
}
