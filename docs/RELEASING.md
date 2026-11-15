# Releasing Ascolta

## One-time crates.io setup

Trusted Publishing is configured on crates.io, once per crate:

- `ascolta`
- `ascolta-domain`
- `ascolta-mpd`

For each crate open **Settings → Trusted Publishing** and use:

| Field | Value |
| --- | --- |
| Provider | GitHub Actions |
| Repository owner | `eval-exec` |
| Repository name | `ascolta` |
| Workflow filename | `publish.yml` |
| Environment | *(leave empty)* |

The first release of each crate had to be published manually with a token.
After that, trusted publishing issues a short-lived token for the
`.github/workflows/publish.yml` job; no long-lived crates.io secret is stored in
the repository.

## Release checklist

1. Bump the workspace version in `Cargo.toml`.
2. Update the version in `PKGBUILD`, `flake.nix`, and `docs/ascolta.1`.
3. Run `cargo check --workspace` so `Cargo.lock` is updated.
4. Run `just fmt`, `just clippy`, and `just test`.
5. Commit and push `main`.
6. Tag and push the release:

   ```sh
   git tag -a vX.Y.Z -m "Ascolta X.Y.Z"
   git push origin vX.Y.Z
   ```

7. The `Release` workflow builds Linux x86_64/aarch64 archives.
8. Publish the GitHub release with notes.
9. The `Publish crates` workflow runs on the published release and uploads
   `ascolta-domain`, then `ascolta-mpd`, then `ascolta` in order. It can also be
   started manually with `gh workflow run publish.yml`.
