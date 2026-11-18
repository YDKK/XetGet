use anyhow::{Context, Result, bail, ensure};
use percent_encoding::percent_decode_str;
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoKind {
    Model,
    Dataset,
    Space,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Source {
    pub kind: RepoKind,
    pub owner: String,
    pub repo: String,
    pub revision: String,
    pub filename: String,
    pub basename: String,
}

impl Source {
    pub fn parse(input: &str) -> Result<Self> {
        let url = Url::parse(input).context("invalid URL")?;
        ensure!(
            url.scheme() == "https" && url.host_str() == Some("huggingface.co"),
            "expected an https://huggingface.co/ URL"
        );
        ensure!(
            url.username().is_empty() && url.password().is_none() && url.port().is_none(),
            "URL credentials and custom ports are not supported; use HF_TOKEN for authentication"
        );

        // Inspect the original path: Url normalizes dot segments before exposing path().
        let path = input
            .split_once("://")
            .and_then(|(_, rest)| rest.split_once('/'))
            .map(|(_, path)| path.split(['?', '#']).next().unwrap_or_default())
            .context("URL has no file path")?;
        ensure!(
            !path.contains('\\'),
            "backslashes are not allowed in URL paths"
        );
        let mut parts: Vec<String> = path.split('/').map(decode).collect::<Result<_>>()?;
        let kind = match parts.first().map(String::as_str) {
            Some("datasets") => {
                parts.remove(0);
                RepoKind::Dataset
            }
            Some("spaces") => {
                parts.remove(0);
                RepoKind::Space
            }
            _ => RepoKind::Model,
        };
        ensure!(
            parts.len() >= 5 && matches!(parts[2].as_str(), "blob" | "resolve"),
            "expected /OWNER/REPO/blob/REVISION/FILE or /OWNER/REPO/resolve/REVISION/FILE"
        );
        for component in [&parts[0], &parts[1]] {
            validate_component(component)?;
        }
        let revision = parts[3].clone();
        ensure!(
            !revision.is_empty() && !revision.chars().any(|c| c.is_control() || c == '\\'),
            "invalid revision (encode slashes as %2F)"
        );
        for component in &parts[4..] {
            validate_component(component)?;
        }
        Ok(Self {
            kind,
            owner: parts[0].clone(),
            repo: parts[1].clone(),
            revision,
            filename: parts[4..].join("/"),
            basename: parts.last().unwrap().clone(),
        })
    }
}

fn decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && (i + 2 >= bytes.len() || !bytes[i + 1..=i + 2].iter().all(u8::is_ascii_hexdigit))
        {
            bail!("invalid percent encoding in URL path");
        }
    }
    Ok(percent_decode_str(value)
        .decode_utf8()
        .context("URL path is not valid UTF-8")?
        .into_owned())
}

fn validate_component(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value != "."
            && value != ".."
            && !value
                .chars()
                .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':')),
        "unsafe or empty path component in URL"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blob_and_resolve_urls() {
        let prefix = "https://huggingface.co/hf-internal-testing/tiny-random-bert";
        let file = "onnx/model.onnx";
        let blob = Source::parse(&format!("{prefix}/blob/main/{file}")).unwrap();
        let resolve =
            Source::parse(&format!("{prefix}/resolve/main/{file}?download=true")).unwrap();
        assert_eq!(blob, resolve);
        assert_eq!(blob.filename, file);
        assert_eq!(blob.basename, "model.onnx");
    }

    #[test]
    fn handles_repo_types_and_encoded_paths() {
        for (prefix, kind) in [
            ("", RepoKind::Model),
            ("datasets/", RepoKind::Dataset),
            ("spaces/", RepoKind::Space),
        ] {
            let source = Source::parse(&format!("https://huggingface.co/{prefix}owner/repo/resolve/refs%2Fpr%2F1/sub/%E6%97%A5%E6%9C%AC%E8%AA%9E%20%23.bin?download=true#ignored")).unwrap();
            assert_eq!(source.kind, kind);
            assert_eq!(source.revision, "refs/pr/1");
            assert_eq!(source.filename, "sub/日本語 #.bin");
        }
    }

    #[test]
    fn rejects_invalid_or_unsafe_urls() {
        for input in [
            "http://huggingface.co/a/b/blob/main/file",
            "https://huggingface.co.evil.example/a/b/blob/main/file",
            "https://user:secret@huggingface.co/a/b/blob/main/file",
            "https://huggingface.co:8443/a/b/blob/main/file",
            "https://huggingface.co/a/b/tree/main/file",
            "https://huggingface.co/a/b/blob/main/",
            "https://huggingface.co/a/b/blob/main/../file",
            "https://huggingface.co/a/b/blob/main/%2e%2e/file",
            "https://huggingface.co/a/b/blob/main/%2Ftmp%2Fevil",
            "https://huggingface.co/a/b/blob/main/dir%5Cevil",
            "https://huggingface.co/a/b/blob/main/file%3Astream",
            "https://huggingface.co/a/b/blob/main/file%00",
            "https://huggingface.co/a/b/blob/main/%FF",
            "https://huggingface.co/a/b/blob/main/%ZZ",
        ] {
            assert!(Source::parse(input).is_err(), "accepted {input}");
        }
    }
}
