# Supply chain

## Dependencies

- Crates.io dependencies are locked in `Cargo.lock`. Build with `--locked`.
- Git dependencies are pinned to full revisions in `Cargo.toml` and allowed by `deny.toml`:
  `hashtrinity`, and the optional `framefold` and `transcode`.
- `calamine/` is a path sub-crate with its own `Cargo.toml` and `Cargo.lock`.

## Checks in CI

| Check | Workflow | What it does |
|---|---|---|
| RustSec audit | `supply-chain.yml` | `cargo audit --deny warnings` |
| Dependency policy | `supply-chain.yml` | `cargo deny check advisories bans licenses sources` |
| SBOM | `supply-chain.yml` | CycloneDX JSON, uploaded as an artifact |
| Dependency review | `dependency-review.yml` | fails pull requests that add high-severity advisories |
| CodeQL | `codeql.yml` | Rust analysis with `security-extended` |
| Scorecard | `scorecard.yml` | OpenSSF Scorecard; results uploaded to code scanning |

Dependency review, CodeQL and Scorecard publishing need GitHub Advanced Security or a public repository.

## Actions

Every `uses:` is pinned to a full commit SHA. Each SHA was checked against its upstream tag or branch with `git ls-remote` when the workflow was written. Renovate or Dependabot updates them weekly.

## Release artifacts

See [RELEASING.md](RELEASING.md). Verification commands for consumers are there too.
