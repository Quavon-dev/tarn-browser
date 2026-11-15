# Security policy

## Reporting a vulnerability

Report vulnerabilities privately through GitHub: open the repository's **Security** tab and choose
**Report a vulnerability**. Please do not open a public issue for security problems.

Include the Tarn version or commit, the macOS version, steps to reproduce and the impact you expect.

## What to expect

- Acknowledgement within 3 working days.
- For a high-severity issue in Tarn's own code, the target is a fix release within 7 days of
  confirmation.
- Credit in the release notes, unless you ask not to be named.

## Scope

In scope: the Tarn shell (everything in this repository), its update and filter-list delivery, and
its handling of web content, extensions and AI providers.

Out of scope, report upstream instead:

- WebKit rendering, JavaScript, TLS and content-process sandbox bugs: Apple
  (<https://security.apple.com>). Tarn uses the system WebKit and cannot ship WebKit fixes.
- The Claude CLI, Ollama and browser extensions: their vendors.

The threat model is in [`docs/threat-model.md`](docs/threat-model.md).
