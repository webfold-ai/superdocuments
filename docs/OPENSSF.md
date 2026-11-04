# OpenSSF status

This page records what the repository does for the OpenSSF Scorecard and Best Practices checks. It does not claim a score.

## Scorecard checks addressed by files in this repository

| Check | Where | Status |
|---|---|---|
| Security-Policy | `SECURITY.md` | present |
| License | `LICENSE` | present (FSL-1.1-ALv2; not OSI-approved) |
| Pinned-Dependencies | `.github/workflows/*.yml` | every `uses:` pinned to a full SHA |
| Token-Permissions | workflows | `contents: read` by default |
| SAST | `codeql.yml` | CodeQL, needs GHAS on a private repository |
| Dependency-Update-Tool | `.github/dependabot.yml` | cargo and github-actions |
| Fuzzing | not present | no fuzz targets yet |
| Signed-Releases | `release.yml` | cosign bundles and provenance; not yet exercised on a tag |
| Branch-Protection | GitHub settings | not configured by this repository |
| Code-Review | GitHub settings | depends on branch protection |
| Maintained, Contributors | — | depend on activity and contributor history |

## Open items

- **Repository visibility.** Scorecard publishing and most badges need a public repository.
- **Branch protection.** Not applied. Owners set in `.github/CODEOWNERS` are `@twn-lloyd @yokoszn`.
- **Best Practices badge.** Not registered. A person must complete the bestpractices.dev survey.
- **Fuzzing.** No fuzz target exists for the parsers.
- **GHAS.** Required for dependency review and CodeQL on a private repository.
