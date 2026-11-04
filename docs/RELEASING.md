# Releasing

Releases are tag-gated. Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`.

## Steps

1. Update `version` in `Cargo.toml` and `CHANGELOG.md`.
2. Merge to `main` through a reviewed pull request.
3. Tag the merged commit as `vX.Y.Z` and push the tag.
4. The workflow checks that the tag matches `Cargo.toml`, runs `cargo test --locked`, and packages a source archive.
5. It signs the archive and `SHA256SUMS` with cosign keyless bundles (`*.sigstore.json`), attests build provenance, and copies the bundle to `superdocuments-vX.Y.Z.tar.gz.intoto.jsonl`.
6. It creates a **draft** GitHub release. A maintainer reviews it and publishes it by hand.

## Verifying a release

```sh
sha256sum -c SHA256SUMS
cosign verify-blob \
  --bundle superdocuments-vX.Y.Z.tar.gz.sigstore.json \
  --certificate-identity 'https://github.com/webfold-ai/superdocuments/.github/workflows/release.yml@refs/tags/vX.Y.Z' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  superdocuments-vX.Y.Z.tar.gz
gh attestation verify superdocuments-vX.Y.Z.tar.gz --repo webfold-ai/superdocuments
```

These commands have not been run against a real release.
