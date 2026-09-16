# What Fortax Mail can do

Everything below is in this repository and runs on the machine in front of you. Where a capability is
inherited from [Flectar Mail](https://github.com/flectar/mail) it is marked *(upstream)*; the hiSAI bridge is
this fork's own.

## Accounts and protocols *(upstream)*

| | |
|---|---|
| Mail | IMAP (IDLE), JMAP, SMTP submission |
| Calendar | CalDAV, with invitations (`REQUEST`/`REPLY`) and RSVP |
| Contacts | CardDAV |
| Files | JMAP and WebDAV storage, browsable and searchable |
| Sign-in | OAuth for Gmail, Outlook and Microsoft 365, or a password for any standards-based server |
| Several accounts | Unified inbox across them, or one at a time |

Gmail and Outlook OAuth need client IDs. Until Fortax's registrations are verified, paste your own under
**Sign-in settings** on the welcome screen, or connect over IMAP/JMAP.

## The mailbox itself *(upstream)*

- **Local first.** Mail, calendar, contacts and file metadata live in a local SQLite store; the app opens and
  searches without a network, and syncs when it has one.
- **Threads.** Replies grouped in order, with the open message expanded in the reading pane.
- **Search.** Full-text over messages, including attachment text, served from the local index.
- **Triage.** Folders, labels, categories, starring, archive, spam, snooze-style rules and routing.
- **Composer.** Rich text, attachments, named per-account signatures with separate new-message and reply
  defaults.
- **OpenPGP/MIME** on desktop, signing and encryption through an installed GnuPG 2.x with pinentry. Required
  encryption blocks sending when a recipient key is missing. S/MIME is not supported.
- **PDF, image and text preview**, and files kept offline on demand.
- **Two workspaces** (full three-pane or minimal), light and dark, built-in or custom colour palettes, and a
  compact layout for small screens.

## What it deliberately does not do *(upstream)*

- **No WebView.** Email HTML is rendered by [Blitz](https://github.com/DioxusLabs/blitz), a Rust HTML/CSS
  engine, so an email cannot reach a browser runtime. Email scripts do not execute.
- **No remote images** until you allow them, so a tracking pixel cannot report that you opened a message.
- **No cloud in the middle.** Nothing is relayed through Fortax. Passwords and OAuth tokens live in the
  operating system's keychain, not in the database.

Rendering arbitrary email HTML without a browser engine is young work; complicated messages can still render
imperfectly. That trade buys about 20 MB of memory instead of a few hundred.

## The hiSAI bridge *(this fork)*

[hiSAI](https://fortax.in) is Fortax's desktop agent for a CA firm — it works inside the firm's client folder
and on the government portals. The bridge is how it reads the firm's mail without the mail leaving the firm.

**Shape of it**

- Off until you switch it on: `fortax-mail --bridge on`, then restart the app (`--bridge status`, `--bridge off`).
- Listens on `127.0.0.1` only, on a port the OS assigns. A LAN or internet client cannot reach it.
- Every request carries a bearer token. The token is written to `bridge.json` in the application data
  directory with `0600` permissions, and hiSAI — running as the same user — is the only thing that can read
  it. A wrong or missing token gets `401`.
- **GET only, read only.** A `POST` gets `405`. There is no endpoint that deletes, moves, marks or sends
  anything.

**Endpoints** (`/v1`, JSON)

| Endpoint | Answers with |
|---|---|
| `/v1/ping` | that the bridge is alive |
| `/v1/accounts` | the connected accounts: address, provider, display name |
| `/v1/search?q=&limit=` | matching threads — subject, participants, date, snippet, counts |
| `/v1/thread?id=` | the messages in one thread, oldest first |
| `/v1/message?id=` | one message: headers, the plain-text body, and whether HTML and attachments exist |
| `/v1/contacts?q=&limit=` | contacts matching a name or address |
| `/v1/calendar?from=&to=` | events in a window (defaults to the next fortnight): title, time, location, organiser, join link |

**Sending is not one of its powers.** hiSAI's `mail_compose` opens a pre-filled draft in Fortax Mail; the
person reads it and presses Send. Nothing in the bridge can put a message on the wire.

**Where this leaves the data.** Message text travels from Fortax Mail to hiSAI over loopback, and from there
into whatever the agent is doing — including, if the CA asked for it, a model call. The bridge cannot know
what hiSAI does next; switch it off when you do not want that, and remember that turning it on is a decision
about the firm's mail.

## Platforms

macOS 14+, Windows 10+, and Linux (AppImage, Flatpak, `.deb`) — all *(upstream)*, all built from this tree.
Android is upstream's experimental build and untested in this fork; the hiSAI bridge is desktop-only, and
hiSAI itself runs on macOS and Windows.

## Build and package

```bash
cargo run --bin fortax-mail                 # run it
cargo test --workspace --locked             # the suite
bash scripts/package-macos.sh               # .app and .dmg
bash scripts/build-flatpak.sh               # Linux Flatpak
```

The hiSAI side of the bridge is tested from the hiSAI repository (`tests/mail/run.sh`), which drives a mock
bridge and then the real one.
