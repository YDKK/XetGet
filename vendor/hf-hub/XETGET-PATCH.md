# Local hf-hub patch

Source: `hf-hub` 1.0.0 from crates.io; upstream revision
`dcf865adacf7cbb750038d6521deb16c670ffd42` (`hf-hub/` subdirectory).
Upstream: https://github.com/huggingface/hf-hub
License: Apache-2.0, included in LICENSE.

Patch 1 changes `src/repository/download.rs`, `download_file_to_local_dir`:

1. Use `head_with_relative_redirects` to retain the original Hub response's
   `X-Xet-Hash` / `X-Linked-Size` headers instead of following the CDN redirect.
2. Accept that terminal redirect response as metadata. Continue to validate
   non-redirect responses with the existing error handler.

The unmodified 1.0.0 implementation follows the HEAD redirect and loses the
Xet hash, causing HTTP downloads even for Xet-managed files. Simply disabling
redirects on a custom client fails because its 302 is rejected before the Xet
branch. The fix uses the same metadata strategy as the upstream cache path.
Non-Xet GETs retain the standard redirect-following client.

Patch 2 addresses [issue #241](https://github.com/huggingface/hf-hub/issues/241).
It adds `repository/download_path.rs` and calls its checks from `download.rs`
and `xet.rs`, with the module declared in `repository/mod.rs`:

- Reject unsafe repository filenames in single-file local/cache downloads and
  snapshot listings before HEAD requests, cached-path lookups or negative-cache
  writes. HTTP and Xet paths share the validation.
- Reject existing symlinks/reparse points below `local_dir`, including final
  files and Xet batch `.incomplete` files. The caller-selected root itself may
  be a link; concurrent replacement by another local process is not covered.

`tests/cli.rs` covers Xet selection with a local Hub/CDN simulation and opt-in
live downloads. `tests/hf_hub_paths.rs` exercises the crate API directly for
single-file and snapshot traversal, ordinary nested files, Unix symlinks and
Windows links/junctions/device names.

The standalone patches and an offline source-parity check are in
[`patches/hf-hub`](../../patches/hf-hub/README.md). Remove each patch after a
released upstream version provides its fix and the relevant regression tests
pass against that version.

Packaging: `src/hub.rs` in the XetGet root compiles these modules as an internal
library target of the `xetget` package. `Cargo.toml.upstream` records the original
dependency configuration. The root manifest includes the source and this license
in its crates.io archive; it does not use a local dependency override.
