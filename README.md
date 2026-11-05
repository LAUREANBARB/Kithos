# Kithos

Private voice, video and text for a small friend group.

Kithos is a self-hosted alternative to Discord, built for roughly 5-15 people.
It targets the four things the existing options each get partly wrong: voice
that sounds fine, screen sharing that holds 1080p60, chat with durable
searchable history, and a client whose theme, fonts, avatars and banners you
actually control.

**Status: pre-alpha. There is no code yet.** The design document is kept
privately and is not part of this repository.

## Why

Discord is a 400-800 MB Electron client that collects far more data than a
group chat needs, and it has leaked that data more than once. The open
alternatives solve chat well and media badly, and the lightweight ones give up
theming entirely. Kithos is deliberately narrow: one small server, one native
client, no telemetry, no third party holding the messages.

## Shape

| Part | Choice | Why |
| --- | --- | --- |
| Client shell | Tauri v2, webview UI | CSS gives themes, fonts and animated avatars for free |
| Media core | LiveKit Rust SDK + native capture | Webview `getDisplayMedia` is unreliable on Linux |
| SFU | Self-hosted LiveKit | Forwards packets, never transcodes, so 15 users fit on 2 vCPU |
| Backend | Rust, axum, SQLite | One file of state, one process, no cluster |
| Server cost | One VPS, roughly 4 EUR/month | Traffic is not a constraint at this scale |

Media never passes through the application server. The server issues a
short-lived, room-scoped LiveKit token and gets out of the way.

## Repository layout

Planned; the directories do not exist yet.

```
crates/proto         wire types shared by server and client
crates/server        auth, chat, presence, uploads, token signing
crates/client-core   media pipeline and chat transport, no UI
apps/desktop         Tauri shell and webview UI
```

`client-core` stays UI-agnostic so the shell could be replaced without touching
the media work.

## Roadmap

- **P0** Media spike: native screen capture, 1080p60 published and rendered on
  a second machine. This is the go/no-go for the whole design.
- **P1** Server: schema, auth, channels, message history, uploads, WebSocket
  fan-out.
- **P2** Client UI: chat, theming, profiles, avatars and banners.
- **P3** Media integration: voice channels, screen share toggle, device
  selection.
- **P4** Deploy: TLS, systemd units, backups, one rehearsed restore.

## Development

You need a Rust toolchain, Node 20 or newer, and the Tauri prerequisites for
your platform. On Linux that means WebKitGTK 4.1, GTK3, and the PipeWire and
`xdg-desktop-portal` packages for screen capture.

Build instructions will land with P0. Until then there is nothing to build.

Codespaces and other headless containers cannot develop this project: the
client needs a display, an audio device, a GPU and a real screen to capture.

## License

Source-available under the **Kithos Personal Use License (KPUL) v1.0**. See
[`LICENSE`](LICENSE).

This is **not** an open-source license and is not approved by the Open Source
Initiative. You may read, modify and self-host Kithos for yourself or your own
private group. You may not redistribute the source or commercially exploit it.
Third-party components keep their own licenses; see `THIRD_PARTY_NOTICES` once
it exists.

Copyright (c) 2026 DVRKXN. All rights reserved.
