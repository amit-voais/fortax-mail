# Fortax Mail licensing

Fortax Mail is one open-source application, derived from
[Flectar Mail](https://github.com/flectar/mail). There is no separate "community
edition", "commercial edition", or proprietary Fortax Mail client, and there is
no build of this client that Fortax ships without publishing its source.

## The licence

Unless a file or directory says otherwise, the source in this repository is
licensed under the GNU Affero General Public License, version 3 only
(`AGPL-3.0-only`) — the licence it carries upstream. The complete terms are in
[`LICENSE`](LICENSE).

The AGPL permits use, study, modification, redistribution, and commercial use.
When its conditions apply, distributors must provide corresponding source under
the same licence. Modified versions offered for remote interaction must also
offer their corresponding source to those users. Publishing a version under the
AGPL is permanent: later business decisions cannot revoke rights already granted
for that version.

The complete corresponding source of any Fortax Mail build we ship is this
repository at the tag that built it.

## The hiSAI bridge

The bridge that lets [hiSAI](https://fortax.in) read the mailbox is part of this
repository and carries the same licence. It listens on loopback only, is off
until the user turns it on, and is read-only. hiSAI itself is a separate,
proprietary Fortax product: it talks to this bridge over a documented local HTTP
interface and is not derived from it. Speaking to an independent program over a
network interface does not place that program under this licence — see section
13 of the AGPL for what does.

Fortax's paid services (hiSAI seats, the Fortax platform) are priced for that
service. They do not buy a differently licensed mail client, and nothing in them
removes your rights to this code.

## Contributions

Contributions are accepted without a contributor licence agreement, under the
Developer Certificate of Origin. Contributors keep their copyright, and accepted
contributions are published under `AGPL-3.0-only`. See
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## Trademarks

The licence covers the code, not the names. "Fortax", "hiSAI" and the Fortax
Mail artwork are Fortax marks; "Flectar" and "Flectar Mail" are Flectar's. A
modified version you distribute must be renamed and re-badged — see
[`TRADEMARKS.md`](TRADEMARKS.md), and upstream's policy at
<https://github.com/flectar/mail/blob/main/TRADEMARKS.md>.

## Third-party material

Dependencies, vendored components, fonts, and example brand marks retain their
own licences and notices. They are documented in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) and the [`LICENSES/`](LICENSES)
directory. Fortax cannot offer rights to material it does not own.

## Practical summary

| Use | Fortax Mail |
|---|---|
| Use privately or commercially | Yes |
| Modify or fork | Yes |
| Sell copies or services | Yes |
| Distribute a closed-source modified build | No |
| Use Fortax or Flectar branding for a modified distribution | No |
| Use hiSAI with it | Yes, with a hiSAI seat; the bridge itself is free software |

This document explains the model; the licence texts control if there is any
conflict.
