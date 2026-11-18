# hf-hub patches

Base: crates.io `hf-hub` 1.0.0, upstream commit
`dcf865adacf7cbb750038d6521deb16c670ffd42`, `hf-hub/` subdirectory.
The crate's source is Apache-2.0 licensed.

Apply in this order:

1. `0001-preserve-xet-metadata.patch`: preserve Hub metadata across CDN redirects.
2. `0002-confine-download-paths.patch`: reject unsafe filenames in single-file and
   snapshot downloads, before network/cache side effects; check existing links
   below `local_dir`, including Xet batch `.incomplete` paths.

The second patch addresses https://github.com/huggingface/hf-hub/issues/241.
It rejects traversal, absolute paths, backslashes and drive/ADS syntax on all
platforms, plus Windows device names and path aliases on Windows. Existing
symlinks and Windows reparse points below the chosen output root are rejected.
It does not protect against another local process replacing paths concurrently.

The official upstream repository cannot contain our unpublished patched commits.
An unmodified official submodule plus these patches would require a manual patch
step before Cargo compiles dependencies. Keep the patched source under
`vendor/hf-hub`. The internal library target in `src/hub.rs` compiles this source
inside the `xetget` package, so crates.io and Git installs use the same fixes.
The upstream manifest is retained as `Cargo.toml.upstream` for dependency
reference; do not restore its original filename, because Cargo excludes nested
packages from the parent package archive. The root manifest carries the required
dependencies and features. No separately published fork or Cargo patch is needed.

To apply these patches in a checkout of the official monorepo at the base commit:

```sh
git -C /path/to/hf-hub apply --directory=hf-hub /absolute/path/to/0001-preserve-xet-metadata.patch
git -C /path/to/hf-hub apply --directory=hf-hub /absolute/path/to/0002-confine-download-paths.patch
```

To check the checked-in vendor and patch files (Rust and Git):

```console
cargo test --locked --test hf_hub_patches -- --nocapture
```

The checker reverses the patches in OS TEMP, verifies a hash of the original
upstream `src` tree recorded in `upstream.json`, then reapplies both patches and
checks that the result matches the vendored source. It needs no network access
and removes its temporary files. It is also included in `cargo test`, so CI runs
the check on Windows and Linux along with the other tests.
