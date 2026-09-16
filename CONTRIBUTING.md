# Contributing to Fortax Mail

Thank you for helping improve Fortax Mail. Keep changes focused, reviewable, and
safe for a mail client that handles private user data.

Fortax Mail is a fork of [Flectar Mail](https://github.com/flectar/mail). A fix
that is not specific to this fork is usually worth offering upstream as well —
everyone downstream of Flectar benefits, and this fork keeps tracking upstream.
Anything about the hiSAI bridge, the Fortax branding or the CA-firm workflow
belongs here.

## Before starting

- Search existing issues and pull requests before opening a duplicate.
- Open an issue before substantial features, architecture changes, new
  dependencies, protocol changes, or user-visible licensing changes.
- Never submit real mailbox contents, credentials, OAuth tokens, signing
  material, or personal fixture data. Use reserved `.example` domains.

## Licensing of contributions

There is no contributor licence agreement. Inbound is outbound: what you send is
published as part of Fortax Mail under `AGPL-3.0-only`, unless a file is
explicitly identified as third-party material under another compatible licence.
You keep your copyright.

Sign off each commit with `git commit -s` to certify the
[Developer Certificate of Origin](https://developercertificate.org/) — that you
wrote the change, or have the right to submit it under this licence. If an
employer owns your work, make sure they allow you to contribute it.

## Pull requests

- Keep one logical change per pull request.
- Explain the problem, the chosen solution, tests, and user-visible effects.
- Add or update tests for behavior changes.
- Disclose material use of generative tools. You remain responsible for every
  submitted line and for confirming that generated material has valid
  provenance and compatible licensing.
- Preserve third-party copyright and license notices.
- Do not add dependencies or copied assets without documenting their source,
  version, license, and required notices.

## Local validation

Run the checks relevant to your change:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
slint-viewer --check ui/app.slint
```

Bridge changes must also pass the hiSAI-side tests in the hiSAI repository
(`tests/mail/run.sh`), which drive this bridge over loopback.

UI changes must be rendered and inspected in light and dark themes. Responsive
changes must also be checked at the documented phone and tablet preview sizes.
Use fictional data in screenshots.

Icon sources, generation and sizing are documented in
[Lucide icons](ui/icons/lucide/README.md).
