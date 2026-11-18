//! End-to-end installation of an unpublished release archive via loopback HTTP.
//! Run explicitly after scripts/package-release.ps1 with cargo-binstall installed.
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, ensure};

struct ArchiveServer {
    endpoint: String,
    downloads: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ArchiveServer {
    fn new(path: String, data: Vec<u8>) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let stop = Arc::new(AtomicBool::new(false));
        let downloads = Arc::new(AtomicUsize::new(0));
        let stopped = stop.clone();
        let received = downloads.clone();
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("archive server: {error}"),
                };
                // Windows may inherit the listener's nonblocking mode.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line == "\r\n" => break,
                        _ => {}
                    }
                }
                let parts: Vec<_> = request.split_whitespace().collect();
                let valid =
                    parts.len() >= 2 && parts[1] == path && matches!(parts[0], "GET" | "HEAD");
                let status = if valid { "200 OK" } else { "404 Not Found" };
                let length = if valid { data.len() } else { 0 };
                if write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
                )
                .is_err()
                {
                    continue;
                }
                if valid && parts[0] == "GET" && stream.write_all(&data).is_ok() {
                    received.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        Ok(Self {
            endpoint,
            downloads,
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for ArchiveServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("archive server panicked");
        }
    }
}

#[test]
#[ignore = "requires cargo-binstall, XETGET_TEST_ARCHIVE and XETGET_TEST_TARGET"]
fn binstall_installs_release_archive() -> Result<()> {
    let archive = PathBuf::from(std::env::var("XETGET_TEST_ARCHIVE")?);
    let target = std::env::var("XETGET_TEST_TARGET")?;
    let metadata = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .output()?;
    ensure!(metadata.status.success(), "cargo metadata failed");
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout)?;
    let package = metadata["packages"]
        .as_array()
        .context("missing packages")?
        .iter()
        .find(|p| p["name"] == "xetget")
        .context("missing xetget")?;
    let config = &package["metadata"]["binstall"];
    let format = config["overrides"][&target]["pkg-fmt"]
        .as_str()
        .or_else(|| config["pkg-fmt"].as_str())
        .context("missing archive format")?;
    let version = package["version"].as_str().context("missing version")?;
    let suffix = format!(".{format}");
    let template = config["pkg-url"]
        .as_str()
        .context("missing URL template")?
        .replace("{ name }", "xetget")
        .replace("{ version }", version)
        .replace("{ target }", &target)
        .replace("{ archive-suffix }", &suffix);
    let path = template
        .strip_prefix("{ repo }")
        .context("expected repository URL prefix")?;
    ensure!(
        archive.file_name().and_then(|n| n.to_str()) == path.rsplit('/').next(),
        "archive filename does not match binstall metadata: {template}"
    );
    let server = ArchiveServer::new(path.to_string(), fs::read(&archive)?)?;
    let install = tempfile::Builder::new()
        .prefix("xetget-binstall-")
        .tempdir()?;
    let manifest = PathBuf::from(
        package["manifest_path"]
            .as_str()
            .context("missing manifest path")?,
    );
    let output = Command::new("cargo")
        .args([
            "binstall",
            "--no-confirm",
            "--no-symlinks",
            "--strategies",
            "crate-meta-data",
            "--allow-insecure-http",
            "--manifest-path",
        ])
        .arg(manifest)
        .arg("--root")
        .arg(install.path())
        .arg("--pkg-url")
        .arg(template.replace("{ repo }", &server.endpoint))
        .arg("--target")
        .arg(&target)
        .arg("--version")
        .arg(version)
        .arg("xetget")
        .env("BINSTALL_DISABLE_TELEMETRY", "true")
        .output()
        .context("cannot run cargo-binstall")?;
    ensure!(
        output.status.success(),
        "cargo-binstall failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        server.downloads.load(Ordering::Relaxed) > 0,
        "archive was not downloaded"
    );
    let binary = install
        .path()
        .join("bin")
        .join(format!("xetget{}", std::env::consts::EXE_SUFFIX));
    let output = Command::new(&binary).arg("--version").output()?;
    ensure!(output.status.success(), "installed binary failed");
    ensure!(
        String::from_utf8_lossy(&output.stdout).trim() == format!("xetget {version}"),
        "installed version differs from manifest"
    );
    ensure!(
        Command::new(binary).arg("--help").status()?.success(),
        "installed help failed"
    );
    install
        .close()
        .context("cannot remove temporary binstall installation")?;
    Ok(())
}
