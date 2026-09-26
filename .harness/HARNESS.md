# Harness — Add a superdocuments::markdown module: hand-rolled docs-shaped markdown parser (headings, paragraphs, code fences, nested lists, links, images, GFM tables, blockquotes, inline emphasis/strong/code, YAML frontmatter) with parse() -> MarkdownDocument and to_html() -> String, informed by pulldown-cmark/comrak architecture but zero third-party markdown deps

repo: /home/ctown/projects/superdocuments · phase: Planning

status: **FRESH**

## Constraints (numbered, prioritized — rank 0 is a refusal)

- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.env`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.pem`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.key`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.p12`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.pfx`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `credentials`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `secrets/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.ssh/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.aws/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.gnupg/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.git/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.claude/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.agents/`
- **FLOOR** (rank 0) [machine floor] Never edit/write anything whose path contains `.config/opencode`
- **C3** (rank 0) [/home/ctown/.config/opencode/AGENTS.md] A class of solution that failed once is never re-proposed; three failed
- **C4** (rank 0) [/home/ctown/.config/opencode/AGENTS.md] Never run the heavy path when the cheap path answers the question.
- **C5** (rank 0) [/home/ctown/.config/opencode/AGENTS.md] 5. Secrets are never read, written, or committed
- **C6** (rank 0) [/home/ctown/.claude/CLAUDE.md] step that is red means **REPLAN** — never retry blind, never continue
- **C7** (rank 0) [/home/ctown/.claude/CLAUDE.md] Secrets are never read, written, or committed (`.env*`, keys, credentials,
- **C8** (rank 0) [/home/ctown/.claude/CLAUDE.md] Destructive commands are never run (sudo, shutdown, mkfs, dd, force-push,
- **C9** (rank 0) [/home/ctown/.claude/CLAUDE.md] The anti-loop rule: a class of solution that failed once is never
- **C1** (rank 2) [/home/ctown/.config/opencode/AGENTS.md] If a guard ever passes something it should refuse, fix the guard — do not
- **C2** (rank 2) [/home/ctown/.config/opencode/AGENTS.md] the work. Do not substitute a generic plan, a different checkout, or a

## Verification steps

- **V1** [advisory] rustfmt clean — `cargo fmt --all -- --check`
- **V2** [advisory] clippy clean — `cargo clippy --all-targets -- -D warnings`
- **V3** [blocking] tests pass — `cargo test --quiet`
