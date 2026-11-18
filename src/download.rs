use std::{fs, path::Path, time::Duration};

use anyhow::{Context, Result, ensure};
use hf_hub::{
    HFClient,
    progress::{DownloadEvent, ProgressEvent, ProgressHandler},
};
use indicatif::{ProgressBar, ProgressStyle};
use tempfile::{NamedTempFile, TempPath};

use crate::{
    Args,
    source::{RepoKind, Source},
};

#[derive(Debug)]
pub struct Interrupted;

impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("download interrupted")
    }
}

impl std::error::Error for Interrupted {}

pub fn run(args: Args) -> Result<()> {
    let source = Source::parse(&args.url)?;
    let output = args
        .output
        .unwrap_or_else(|| source.basename.clone().into());
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure!(output.file_name().is_some(), "output must be a file path");
    check_output(&output, args.force)?;
    let staging = tempfile::Builder::new()
        .prefix(".xetget-")
        .tempdir_in(parent)
        .with_context(|| format!("cannot create temporary directory in {}", parent.display()))?;
    let xet_work = tempfile::Builder::new().prefix("xetget-xet-").tempdir()?;
    configure_xet(xet_work.path());
    let progress = TransferProgress::new(args.quiet);
    if !args.quiet {
        eprintln!(
            "Downloading {}/{} @ {}: {}",
            source.owner, source.repo, source.revision, source.filename
        );
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        let client = HFClient::builder().cache_enabled(false).build()?;
        tokio::select! {
            biased;
            signal = tokio::signal::ctrl_c() => {
                signal.context("cannot listen for Ctrl-C")?;
                Err(anyhow::Error::new(Interrupted))
            }
            result = fetch(&client, &source, staging.path(), progress.clone()) => result,
        }
    });
    // Drop the runtime before cleaning up files that download tasks may have opened.
    drop(runtime);
    progress.0.finish_and_clear();
    let result = result
        .context("download failed")
        .and_then(|downloaded| publish(&downloaded, &output, args.force));
    let cleanup = staging
        .close()
        .context("cannot remove temporary download directory");
    let xet_cleanup = xet_work
        .close()
        .context("cannot remove temporary Xet working directory");
    // Preserve the original error while reporting any failure to remove temporary data.
    let cleanup = cleanup.and(xet_cleanup);
    match (result, cleanup) {
        (Err(error), Err(cleanup)) => {
            return Err(error.context(format!("temporary-file cleanup also failed: {cleanup:#}")));
        }
        (Err(error), _) | (_, Err(error)) => return Err(error),
        _ => {}
    }
    if !args.quiet {
        eprintln!(
            "Saved {} ({} bytes)",
            output.display(),
            fs::metadata(&output)?.len()
        );
    }
    Ok(())
}

fn configure_xet(directory: &Path) {
    // SAFETY: run() is called once from synchronous main, before starting any threads
    // (including the progress renderer and Tokio). No other thread can read the environment.
    unsafe {
        std::env::set_var("HF_XET_CACHE", directory);
        std::env::set_var("HF_XET_CHUNK_CACHE_SIZE_BYTES", "0");
        std::env::set_var("HF_XET_LOG_DEST", directory.join("xet.log"));
        std::env::set_var("HF_XET_SHARD_CACHE_SUBDIR", "shard-cache");
        std::env::set_var("HF_XET_DATA_STAGING_SUBDIR", "staging");
        std::env::set_var("HF_XET_SESSION_DIR_NAME", "session");
    }
}

fn check_output(output: &Path, force: bool) -> Result<()> {
    match fs::symlink_metadata(output) {
        Ok(metadata) => {
            ensure!(
                force,
                "{} already exists; use --force to replace it",
                output.display()
            );
            ensure!(
                metadata.is_file(),
                "{} is not a regular file",
                output.display()
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("cannot inspect {}", output.display()));
        }
    }
    Ok(())
}

fn publish(downloaded: &Path, output: &Path, force: bool) -> Result<()> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(downloaded)?;
    file.sync_all().context("cannot flush downloaded file")?;
    let temp = NamedTempFile::from_parts(file, TempPath::try_from_path(downloaded)?);
    if force {
        check_output(output, true)?;
        temp.persist(output)
    } else {
        temp.persist_noclobber(output)
    }
    .with_context(|| format!("cannot save {}", output.display()))?;
    Ok(())
}

async fn fetch(
    client: &HFClient,
    source: &Source,
    directory: &Path,
    progress: TransferProgress,
) -> Result<std::path::PathBuf> {
    // Repository kinds have distinct type parameters; finish each builder in its branch.
    let result = match source.kind {
        RepoKind::Model => {
            client
                .model(&source.owner, &source.repo)
                .download_file()
                .filename(&source.filename)
                .revision(&source.revision)
                .local_dir(directory)
                .progress(progress)
                .send()
                .await
        }
        RepoKind::Dataset => {
            client
                .dataset(&source.owner, &source.repo)
                .download_file()
                .filename(&source.filename)
                .revision(&source.revision)
                .local_dir(directory)
                .progress(progress)
                .send()
                .await
        }
        RepoKind::Space => {
            client
                .space(&source.owner, &source.repo)
                .download_file()
                .filename(&source.filename)
                .revision(&source.revision)
                .local_dir(directory)
                .progress(progress)
                .send()
                .await
        }
    };
    result.context("Hugging Face request failed (private/gated repositories may require HF_TOKEN and access approval)")
}

#[derive(Clone)]
struct TransferProgress(ProgressBar);

impl TransferProgress {
    fn new(quiet: bool) -> Self {
        let bar = if quiet {
            ProgressBar::hidden()
        } else {
            ProgressBar::new_spinner()
        };
        bar.set_style(ProgressStyle::with_template("{spinner:.cyan} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec} ETA {eta}").expect("valid progress template"));
        bar.enable_steady_tick(Duration::from_millis(100));
        Self(bar)
    }

    fn update(&self, completed: u64, total: u64) {
        if total > 0 {
            self.0.set_length(total);
        }
        self.0.set_position(completed);
    }
}

impl ProgressHandler for TransferProgress {
    fn on_progress(&self, event: &ProgressEvent) {
        match event {
            ProgressEvent::Download(DownloadEvent::Start { total_bytes, .. }) => {
                self.update(0, *total_bytes)
            }
            ProgressEvent::Download(DownloadEvent::AggregateProgress {
                bytes_completed,
                total_bytes,
                ..
            }) => self.update(*bytes_completed, *total_bytes),
            ProgressEvent::Download(DownloadEvent::Progress { files }) => {
                if let Some(file) = files.last() {
                    self.update(file.bytes_completed, file.total_bytes);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_without_clobbering_and_supports_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("file.bin");
        let staging = dir.path().join("partial");
        fs::write(&output, b"existing").unwrap();
        fs::write(&staging, b"downloaded").unwrap();
        assert!(publish(&staging, &output, false).is_err());
        assert_eq!(fs::read(&output).unwrap(), b"existing");
        fs::write(&staging, b"downloaded").unwrap();
        publish(&staging, &output, true).unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"downloaded");
        assert!(!staging.exists());
    }

    #[test]
    fn rejects_existing_directories_even_with_force() {
        let dir = tempfile::tempdir().unwrap();
        assert!(check_output(dir.path(), true).is_err());
    }
}
