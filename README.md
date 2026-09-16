<div align="center">
  <img src="resources/app-icon/fortax-mail-masked-512.png" width="112" alt="Fortax Mail">
  <h1 align="center">Fortax Mail</h1>
  <div align="center">
    <h3>The CA firm's mailbox, on the firm's own machine</h3>
    <p>A fast, native, offline-first mail, calendar and contacts client — and the mailbox hiSAI works from,
       so a client mail is read, drafted and sent from your own address, not from a server in the middle.</p>
  </div>
  <p>
    <a href="https://fortax.in">fortax.in</a> ·
    <a href="https://github.com/amit-voais/fortax-mail/issues">Report an issue</a> ·
    <a href="FORK.md">What we changed</a>
  </p>
</div>

> **Derived from Flectar Mail.** Fortax Mail is a modified build of
> [Flectar Mail](https://github.com/flectar/mail) by Flectar. It is **not produced, sponsored, endorsed or
> supported by Flectar** — please do not take Flectar's issue tracker, security contacts or support channels
> to problems with this build; bring them [here](https://github.com/amit-voais/fortax-mail/issues) instead.
> Like the original, Fortax Mail is licensed under the **GNU Affero General Public License v3 only**
> ([LICENSE](LICENSE)). See [FORK.md](FORK.md) for exactly what differs.

## Why Fortax Mail exists

A Chartered Accountant's mail *is* the practice: the client asking for a figure, the notice with a deadline,
the acknowledgement that has to be filed away. hiSAI already works inside the firm's client folder and on the
government portals. The one thing it could not touch was the mailbox — so mail went out through a server, from
an address that was not the firm's.

Fortax Mail closes that gap:

- **Your own account, connected once.** Gmail, Outlook and Microsoft 365, or plain IMAP/SMTP, JMAP, CalDAV and
  CardDAV — the same standards the upstream client supports.
- **It stays on the machine.** Mail, calendar and contacts are stored locally and work offline. Nothing is
  copied to Fortax's servers, and passwords and tokens live in the operating system's own keychain.
- **hiSAI can read it, with your permission.** A local bridge — off until you turn it on, loopback only,
  token-authenticated and read-only — lets hiSAI search the mailbox, read a thread, look up a contact and see
  the calendar. Writing is not one of its powers: a reply arrives as a draft for you to send.
- **Native and light.** Rust, Slint and Blitz — no browser engine, no WebView for email HTML, remote images
  blocked until you allow them.

## What it can do

The short version: a full native mail, calendar, contacts and files client inherited from upstream, plus a
local read-only bridge for hiSAI. **[docs/CAPABILITIES.md](docs/CAPABILITIES.md)** has the long version —
protocols, what the client deliberately refuses to do, every bridge endpoint, and where the data goes.

## Status

Early. Upstream is itself pre-stable, and this fork adds the hiSAI bridge on top. Treat it as a preview:
use it beside your normal mail client, not instead of it, until we say otherwise.

Gmail and Outlook sign-in needs OAuth client IDs. Until Fortax's own registrations are approved by Google and
Microsoft, use **Sign-in settings** on the welcome screen to paste your own, or connect the account over
IMAP/JMAP.

## Build it

```bash
# macOS 14+, Windows 10+, or Linux. Needs the Rust toolchain (https://rustup.rs), 1.92 or newer.
cargo run --bin fortax-mail          # run it
bash scripts/package-macos.sh        # a .app and a .dmg
fortax-mail --bridge on              # let hiSAI read this mailbox (restart the app afterwards)
```

## Licence

AGPL-3.0-only, the same as upstream. That is deliberate: the client that reads your mail should be one you
can read too. The complete corresponding source of any build we ship is this repository at the tag that
built it. Third-party components keep their own licences — see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [LICENSES/](LICENSES).

## Acknowledgements

- [Flectar Mail](https://github.com/flectar/mail) — the client this one is built from.
- [Slint](https://slint.dev/), the native UI toolkit.
- [Blitz](https://github.com/DioxusLabs/blitz), the Rust HTML/CSS renderer from the Dioxus team.
