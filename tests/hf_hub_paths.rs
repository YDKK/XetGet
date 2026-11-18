use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use hf_hub::{HFClient, HFError};

const COMMIT: &str = "1234567890123456789012345678901234567890";

// Exercise the patched crate directly; the CLI's own URL validation must not
// hide a regression in hf-hub. The entire Hub simulation stays on loopback.
struct Hub {
    endpoint: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Hub {
    fn new(listing: &str, xet: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let received = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = stop.clone();
        let listing = listing.to_string();
        let thread = thread::spawn(move || {
            while !server_stop.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
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
                if request.is_empty() {
                    continue;
                }
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                        break;
                    }
                }
                let path = request.split_whitespace().nth(1).unwrap();
                let head = request.starts_with("HEAD ");
                let body = if path.contains("/tree/") {
                    listing.clone()
                } else if path.contains("/api/") {
                    format!(r#"{{"sha":"{COMMIT}"}}"#)
                } else {
                    "ok".to_string()
                };
                received.lock().unwrap().push(request.clone());
                let extra = if xet && head {
                    format!("X-Xet-Hash: {}\r\n", "a".repeat(64))
                } else {
                    String::new()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Repo-Commit: {COMMIT}\r\nETag: \"abcd\"\r\n{extra}Connection: close\r\n\r\n{}",
                    body.len(),
                    if head { "" } else { &body }
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            endpoint,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn client(&self, cache: &Path) -> HFClient {
        HFClient::builder()
            .endpoint(&self.endpoint)
            .cache_dir(cache)
            .build()
            .unwrap()
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn unsafe_paths() -> Vec<&'static str> {
    vec![
        "",
        ".",
        "..",
        "../escaped",
        "sub/../../escaped",
        "/absolute",
        "a/../b",
        "a/./b",
        "a//b",
        "a/",
        "C:/escaped",
        "C:escaped",
        "\\\\server\\share\\file",
        "a\\..\\escaped",
        "file:stream",
        "file\0",
        "file\n",
    ]
}

#[tokio::test]
async fn single_file_rejects_traversal_before_network_in_local_and_cache_modes() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::new("[]", true);
    let client = hub.client(&dir.path().join("cache"));
    let repo = client.model("owner", "repo");
    for filename in unsafe_paths() {
        for local in [false, true] {
            let result = repo
                .download_file()
                .filename(filename)
                .maybe_local_dir(local.then(|| dir.path().join("local")))
                .send()
                .await;
            assert!(
                matches!(result, Err(HFError::InvalidParameter(_))),
                "accepted {filename:?}: {result:?}"
            );
        }
    }
    assert!(hub.requests.lock().unwrap().is_empty());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn snapshot_rejects_remote_traversal_before_head_or_cache_writes() {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "../escaped",
        "sub/../../escaped",
        "/absolute",
        "C:/escaped",
        "a\\..\\escaped",
    ] {
        let listing = format!(
            r#"[{{"type":"file","oid":"abcd","size":2,"path":"{}"}}]"#,
            path.replace('\\', "\\\\")
        );
        let hub = Hub::new(&listing, true);
        let client = hub.client(&dir.path().join("cache"));
        for local in [false, true] {
            let result = client
                .model("owner", "repo")
                .snapshot_download()
                .revision(COMMIT)
                .maybe_local_dir(local.then(|| dir.path().join("local")))
                .send()
                .await;
            assert!(
                matches!(result, Err(HFError::InvalidParameter(_))),
                "accepted {path:?}: {result:?}"
            );
        }
        let requests = hub.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|r| r.starts_with("GET /api/models/owner/repo/tree/")),
            "unexpected requests: {requests:?}"
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn safe_nested_files_work_for_single_and_snapshot_downloads() {
    let dir = tempfile::tempdir().unwrap();
    let listing = r#"[{"type":"file","oid":"abcd","size":2,"path":"nested/model.bin"}]"#;
    let hub = Hub::new(listing, false);
    let client = hub.client(&dir.path().join("cache"));
    let repo = client.model("owner", "repo");
    let path = repo
        .download_file()
        .filename("nested/model.bin")
        .local_dir(dir.path().join("single"))
        .send()
        .await
        .unwrap();
    assert_eq!(fs::read(path).unwrap(), b"ok");
    let path = repo
        .snapshot_download()
        .revision(COMMIT)
        .local_dir(dir.path().join("snapshot"))
        .send()
        .await
        .unwrap();
    assert_eq!(fs::read(path.join("nested/model.bin")).unwrap(), b"ok");
}

#[cfg(unix)]
#[tokio::test]
async fn refuses_existing_directory_file_and_incomplete_symlinks() {
    use std::os::unix::fs::{symlink, symlink as file_link};
    reject_links(
        |source, target| symlink(source, target),
        |source, target| file_link(source, target),
    )
    .await;
}

#[cfg(windows)]
#[tokio::test]
async fn refuses_existing_directory_file_and_incomplete_symlinks() {
    use std::os::windows::fs::{symlink_dir, symlink_file};
    reject_links(
        |source, target| symlink_dir(source, target),
        |source, target| symlink_file(source, target),
    )
    .await;
}

async fn reject_links(
    dir_link: impl Fn(&Path, &Path) -> std::io::Result<()>,
    file_link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    let local = dir.path().join("local");
    fs::create_dir(&outside).unwrap();
    fs::create_dir(&local).unwrap();
    let sentinel = outside.join("sentinel");
    fs::write(&sentinel, b"keep me").unwrap();
    let nested = local.join("nested");
    if let Err(error) = dir_link(&outside, &nested) {
        // Windows requires Developer Mode or the symlink privilege.
        if cfg!(windows) && error.kind() == std::io::ErrorKind::PermissionDenied {
            eprintln!("symlink test unavailable without Windows symlink privileges: {error}");
            return;
        }
        panic!("{error}");
    }
    file_link(&sentinel, &local.join("file.bin")).unwrap();
    file_link(&sentinel, &local.join("partial.bin.incomplete")).unwrap();
    let listing = r#"[{"type":"file","oid":"abcd","size":2,"path":"partial.bin"}]"#;
    let hub = Hub::new(listing, true);
    let client = hub.client(&dir.path().join("cache"));
    let repo = client.model("owner", "repo");
    for filename in ["nested/sentinel", "file.bin"] {
        let result = repo
            .download_file()
            .filename(filename)
            .local_dir(&local)
            .send()
            .await;
        assert!(matches!(result, Err(HFError::InvalidParameter(_))));
    }
    let result = repo
        .snapshot_download()
        .revision(COMMIT)
        .local_dir(&local)
        .send()
        .await;
    assert!(matches!(result, Err(HFError::InvalidParameter(_))));
    assert_eq!(hub.requests.lock().unwrap().len(), 1); // Only the snapshot listing.
    assert_eq!(fs::read(&sentinel).unwrap(), b"keep me");
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
}

#[cfg(windows)]
#[tokio::test]
async fn rejects_windows_devices_and_path_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let client = HFClient::builder()
        .endpoint("http://127.0.0.1:1")
        .build()
        .unwrap();
    for filename in [
        "NUL",
        "dir/CON.txt",
        "dir/CON .txt",
        "COM1",
        "LPT¹",
        "dir./file",
        "dir /file",
        "file.",
        "file?",
        "file*",
    ] {
        let result = client
            .model("o", "r")
            .download_file()
            .filename(filename)
            .local_dir(dir.path())
            .send()
            .await;
        assert!(
            matches!(result, Err(HFError::InvalidParameter(_))),
            "accepted {filename:?}: {result:?}"
        );
    }
}

#[cfg(windows)]
#[tokio::test]
async fn refuses_existing_directory_junction() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    let local = dir.path().join("local");
    fs::create_dir(&outside).unwrap();
    fs::create_dir(&local).unwrap();
    let sentinel = outside.join("sentinel");
    fs::write(&sentinel, b"keep me").unwrap();
    let result = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J"])
        .arg(local.join("junction"))
        .arg(&outside)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let client = HFClient::builder()
        .endpoint("http://127.0.0.1:1")
        .build()
        .unwrap();
    let result = client
        .model("o", "r")
        .download_file()
        .filename("junction/sentinel")
        .local_dir(&local)
        .send()
        .await;
    assert!(matches!(result, Err(HFError::InvalidParameter(_))));
    assert_eq!(fs::read(&sentinel).unwrap(), b"keep me");
}
