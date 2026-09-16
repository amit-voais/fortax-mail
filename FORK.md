# What Fortax Mail changes, and what it keeps

Fortax Mail is [Flectar Mail](https://github.com/flectar/mail) with a different name, a different icon, its
own application identity, and one new capability: a local bridge that lets hiSAI work with the mailbox.

Flectar's [trademark policy](https://github.com/flectar/mail/blob/main/TRADEMARKS.md) asks a modified build to
rename itself, replace the brand assets and identifiers, say where it came from, and keep clear of Flectar's
own channels. That is what the list below does.

## Changed

| | Upstream | Here |
|---|---|---|
| Name | Flectar Mail | Fortax Mail |
| Binary / crates | `flectar-mail`, `flectar-mail-core` | `fortax-mail`, `fortax-mail-core` |
| Application id | `com.flectar.mail` | `in.fortax.mail` |
| Icon and brand art | Flectar's | Drawn for Fortax (`tools/make-icons.py`) |
| Home and support | flectar.com, Flectar's issue tracker | fortax.in, this repository's issues |
| Update channel and signing identity | Flectar's | Fortax's |

## Added

- **The hiSAI bridge** — a local, token-authenticated interface on the loopback address that lets the hiSAI
  desktop agent search the mailbox, read a message, save an attachment into the client folder and put a draft
  in the account's Drafts. It is off until you switch it on, it never leaves the machine, and sending is not
  one of its powers: a draft still needs a person to press Send.

## Kept

Everything else: the Rust core, the Slint interface, the Blitz renderer, IMAP/JMAP/SMTP, CalDAV and CardDAV,
OpenPGP through GnuPG, the local-first store, and the AGPL-3.0-only licence with the third-party notices.

## Keeping up with upstream

The upstream project is a git remote here called `upstream`. To take its newer work:

```bash
git fetch upstream
git merge upstream/main       # the rename touches many files; expect to resolve some of it by hand
python3 tools/make-icons.py   # if the icon paths moved
```
