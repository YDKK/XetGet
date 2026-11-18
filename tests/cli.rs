use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

use sha2::{Digest, Sha256};

fn cli(directory: &Path, args: &[&str]) -> Output {
    command(directory).args(args).output().unwrap()
}

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xetget"));
    command
        .current_dir(directory)
        .env("HF_HOME", directory.join("hf-home"))
        .env("TMPDIR", directory)
        .env("TMP", directory)
        .env("TEMP", directory)
        .env("HF_XET_CACHE", directory.join("persistent-xet-cache"))
        .env("HF_XET_LOG_DEST", directory.join("persistent-xet.log"))
        .env_remove("HF_TOKEN")
        .env_remove("HF_TOKEN_PATH")
        .env_remove("HF_ENDPOINT");
    command
}

#[test]
fn preserves_xet_metadata_before_cdn_redirects() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    for xet in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server_endpoint = endpoint.clone();
        let done = Arc::new(AtomicBool::new(false));
        let server_done = done.clone();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(15);
            while !server_done.load(Ordering::Relaxed) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                // Windows may inherit the listener's nonblocking mode.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                        break;
                    }
                }
                let path = request.split_whitespace().nth(1).unwrap();
                let response = if path.contains("xet-read-token") {
                    // Reaching this authenticated API proves native Xet was selected.
                    "HTTP/1.1 403 Forbidden\r\nContent-Length: 6\r\nConnection: close\r\n\r\ndenied"
                        .to_string()
                } else if path == "/cdn" {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                        .to_string()
                } else {
                    let xet_header = if xet {
                        format!("X-Xet-Hash: {}\r\n", "a".repeat(64))
                    } else {
                        String::new()
                    };
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: {server_endpoint}/cdn\r\n{xet_header}X-Linked-Size: 2\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                };
                stream.write_all(response.as_bytes()).unwrap();
                requests.push(request);
            }
            requests
        });
        let output = command(dir.path())
            .env("HF_ENDPOINT", &endpoint)
            .arg("https://huggingface.co/owner/repo/resolve/main/model.bin")
            .output()
            .unwrap();
        done.store(true, Ordering::Relaxed);
        let requests = server.join().unwrap();
        if xet {
            assert!(!output.status.success());
            assert!(
                requests.iter().any(|r| r.contains("/xet-read-token/main")),
                "Xet was bypassed: {requests:?}"
            );
            assert!(
                !requests.iter().any(|r| r.contains(" /cdn ")),
                "unexpected HTTP fallback: {requests:?}"
            );
            assert!(!dir.path().join("model.bin").exists());
        } else {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(fs::read(dir.path().join("model.bin")).unwrap(), b"ok");
        }
    }
}

#[test]
fn help_and_version_work() {
    let dir = tempfile::tempdir().unwrap();
    let help = cli(dir.path(), &["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--output"));
    assert!(cli(dir.path(), &["--version"]).status.success());
    assert_eq!(cli(dir.path(), &[]).status.code(), Some(2));
}

#[test]
fn invalid_url_and_existing_output_fail_before_network() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        !cli(dir.path(), &["https://example.com/file"])
            .status
            .success()
    );
    fs::write(dir.path().join("file.bin"), b"keep me").unwrap();
    let output = cli(
        dir.path(),
        &["https://huggingface.co/does-not-exist/repo/blob/main/file.bin"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--force"));
    assert_eq!(fs::read(dir.path().join("file.bin")).unwrap(), b"keep me");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

/// Opt-in network smoke test. All downloaded data is small and lives in OS TEMP.
#[test]
#[ignore = "requires access to huggingface.co and the Xet CAS service"]
fn live_downloads_http_and_xet_without_persistent_cache() {
    let dir = tempfile::tempdir().unwrap();
    for (url, filename) in [
        (
            "https://huggingface.co/hf-internal-testing/tiny-random-bert/blob/f171d7baecaf37b5da5a3616d8833b9969753535/config.json",
            "config.json",
        ),
        (
            "https://huggingface.co/hf-internal-testing/tiny-random-bert/resolve/f171d7baecaf37b5da5a3616d8833b9969753535/onnx/model.onnx?download=true",
            "model.onnx",
        ),
    ] {
        let output = cli(dir.path(), &[url]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(dir.path().join(filename).is_file());
    }
    for (filename, hash) in [
        (
            "config.json",
            "54f9f0001ec62b6a53072a7c9d1b630e346ae51fca14a5799e54f0d918281872",
        ),
        (
            "model.onnx",
            "083459756ab4f6e37a2ebbd0c9ab4cebbda7638d2015958dd2a7ac9047588f93",
        ),
    ] {
        let digest = Sha256::digest(fs::read(dir.path().join(filename)).unwrap());
        assert_eq!(
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            hash,
            "incorrect contents: {filename}"
        );
    }
    assert_eq!(
        fs::metadata(dir.path().join("model.onnx")).unwrap().len(),
        449599
    );
    let failed = cli(
        dir.path(),
        &[
            "https://huggingface.co/hf-internal-testing/tiny-random-bert/resolve/f171d7baecaf37b5da5a3616d8833b9969753535/missing-file.bin",
            "--force",
            "-O",
            "model.onnx",
        ],
    );
    assert!(!failed.status.success());
    assert_eq!(
        fs::metadata(dir.path().join("model.onnx")).unwrap().len(),
        449599
    );
    let entries: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entries.len(), 2, "unexpected artifacts: {entries:?}");
}

#[cfg(unix)]
#[test]
fn interruption_cleans_temporary_files() {
    use std::{
        net::TcpListener,
        process::{Child, Stdio},
        thread,
        time::{Duration, Instant},
    };

    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let mut child = ChildGuard(command(dir.path())
        .arg("https://huggingface.co/hf-internal-testing/tiny-random-bert/resolve/f171d7baecaf37b5da5a3616d8833b9969753535/onnx/model.onnx")
        .env("HF_ENDPOINT", endpoint)
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
    // Hold the local HEAD response until SIGINT, so completion cannot race the
    // signal and no file is downloaded from an external repository.
    let deadline = Instant::now() + Duration::from_secs(15);
    let (_connection, _) = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    child.0.try_wait().unwrap().is_none(),
                    "download exited before making a request"
                );
                assert!(
                    Instant::now() < deadline,
                    "download did not contact the mock Hub"
                );
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("{error}"),
        }
    };
    assert!(
        Command::new("kill")
            .args(["-INT", &child.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            panic!("interrupted download did not exit");
        }
        thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(130));
    let entries: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(entries.is_empty(), "leftover files: {entries:?}");
}
