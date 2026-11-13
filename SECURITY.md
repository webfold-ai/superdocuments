# Security policy

## Supported versions

Only the latest release line receives security fixes. At the time of writing that is 0.8.x.

## Reporting a vulnerability

Please report security problems privately. Do **not** open a public issue or pull request.

- Use GitHub's private vulnerability reporting: <https://github.com/webfold-ai/superdocuments/security/advisories/new>, or
- email the security team at **security@webfold.ai**.

Include:

- the affected version or commit,
- steps to reproduce, with a minimal input file if possible,
- the impact you expect.

**Response target:** to be confirmed by the maintainers. We will acknowledge the report, confirm whether it is a vulnerability, and coordinate a fix and disclosure timeline with you.

## Scope

Parsers handle untrusted documents. Memory-safety issues, panics on crafted input, unbounded resource use, and sandbox or path escapes in the conversion paths are in scope.
