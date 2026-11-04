# Contributing to superdocuments

Thank you for your interest. Please read the licence in `LICENSE` first. Contributions are accepted under its terms.

## Before you start

- Open an issue for anything beyond a small fix, so the approach can be agreed first.
- Report security problems privately, as described in [SECURITY.md](SECURITY.md). Do not open a public issue for them.

## Development

```sh
cargo fmt --all
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

You need access to the git dependencies listed in the README.

## Pull requests

- Keep each pull request to one change, with tests where behaviour changes.
- Use a commit message that says what changed and why.
- CI must pass. The required check is `Rust checks`.
- Code owners review changes (see `.github/CODEOWNERS`).

## Conduct

Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
