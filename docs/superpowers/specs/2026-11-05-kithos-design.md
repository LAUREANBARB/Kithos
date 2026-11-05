# Kithos: private voice, video and text for a small friend group

Status: draft for review, 2026-11-05
Project name: `kithos`. License: KPUL v1.0 (see `LICENSE`).

## 1. Problem

Discord is a data-harvesting, breach-prone, Electron-heavy client with poor
platform decisions. Existing alternatives each miss one of four things this
group needs together:

1. Acceptable voice quality.
2. Screen sharing that holds 1080p60.
3. Text chat with durable history.
4. Real personalization: theme, fonts, font size and color, avatars, animated
   banners, bios.

The missing piece in every alternative is the combination of lightweight client
plus competent screen sharing. Chat is solved everywhere; media is not.

## 2. Goals

- 5-15 users, all in the EU, no cross-region concerns.
- Self-hosted on one small VPS, no vendor with access to plaintext messages.
- Client RAM well under Discord's 400-800 MB, with no Electron.
- Source-available under KPUL v1.0, private use allowed.
- Runs on Windows and Linux desktops.

## 3. Non-goals (v1)

- Mobile clients.
- Roles and permissions beyond `admin | member`.
- Threads, reactions, forums, boosts, store, bots.
- Discord Rich Presence equivalents.
- End-to-end encryption.
- Federation.
- Scaling past roughly 30 concurrent users.

These are cut deliberately. Each one is a multi-week detour that does not
serve the four goals above.

## 4. Architecture

One Cargo workspace, four units with narrow interfaces.

```
crates/proto         wire types, serde only, shared by server and client
crates/server        axum + SQLite: auth, chat, presence, uploads, token issuing
crates/client-core   media pipeline and chat client, no UI, no Tauri
apps/desktop         Tauri v2 shell: webview UI + native video surface
```

Runtime, one VPS, two processes:

```
browser-less desktop client
   |  HTTPS/WS (chat, auth)          UDP + TCP (media)
   v                                   v
myapp-server (axum, :443) -------> livekit-server (SFU)
   |                                   |
   +-- issuers of LiveKit JWTs         +-- forwards RTP, never transcodes
   |
SQLite file + attachment dir
```

The important property: **media never passes through `myapp-server`.** The
server only signs a short-lived LiveKit token naming a room and a permission
set. That is why 15 participants fit on 2 vCPU.

### 4.1 Unit boundaries

| Unit | Does | Depends on |
| --- | --- | --- |
| `proto` | Serialize/deserialize messages, events, payloads | nothing |
| `server` | Persistence, auth, authorization, event fan-out, LiveKit token signing | `proto`, axum, sqlx, sqlite |
| `client-core` | Media capture/encode/publish, subscribe/decode, chat transport | `proto`, livekit rust sdk |
| `apps/desktop` | Render UI, render remote video, wire input | `client-core`, Tauri |

`client-core` is UI-agnostic on purpose. It is the piece a future native shell
(Slint or egui) could reuse if the webview's memory cost ever becomes the
complaint.

## 5. Data model

SQLite, WAL mode, `sqlx` with compile-time checked queries.

```sql
create table users (
  id            integer primary key,
  username      text not null unique collate nocase,
  display_name  text not null,
  password_hash text not null,        -- argon2id
  bio           text not null default '',
  avatar_path   text,                 -- null = initials placeholder
  banner_path   text,
  created_at    integer not null
);

create table sessions (
  token_hash text primary key,        -- sha256 of the bearer token
  user_id    integer not null references users(id) on delete cascade,
  created_at integer not null,
  expires_at integer not null
);

create table guilds (
  id   integer primary key,
  name text not null
);

create table channels (
  id       integer primary key,
  guild_id integer not null references guilds(id) on delete cascade,
  name     text not null,
  kind     text not null check (kind in ('text', 'voice'))
);

create table messages (
  id         integer primary key,     -- global monotonic, also the sync cursor
  channel_id integer not null references channels(id) on delete cascade,
  author_id  integer not null references users(id),
  body       text not null,
  created_at integer not null
);

create table attachments (
  id        integer primary key,
  message_id integer not null references messages(id) on delete cascade,
  path      text not null,
  mime      text not null,
  bytes     integer not null
);

create table members (
  guild_id integer not null references guilds(id) on delete cascade,
  user_id  integer not null references users(id) on delete cascade,
  role     text not null check (role in ('admin', 'member')),
  primary key (guild_id, user_id)
);

create table read_state (
  user_id    integer not null references users(id) on delete cascade,
  channel_id integer not null references channels(id) on delete cascade,
  last_read  integer not null,
  primary key (user_id, channel_id)
);

create virtual table messages_fts using fts5(
  body, content='messages', content_rowid='id'
);

create index messages_channel_id on messages(channel_id, id);
create index attachments_message_id on attachments(message_id);
create index sessions_user_id on sessions(user_id);
```

`messages_channel_id` covers the only hot query shape, history pagination
ordered by `id` within a channel.

Message `id` doubles as the sync cursor: a client reconnects with
`since=<last_id>` and receives everything newer. No per-channel sequence
needed at this scale.

Authorization is per request: every channel-scoped operation joins through
`members` to confirm the caller belongs to the guild owning the channel.
Hiding a button in the UI is not authorization.

## 6. Protocol

### REST (JSON, bearer token)

```
POST /auth/register        {username, display_name, password}
POST /auth/login           -> {token, user}
POST /auth/logout
GET  /me
PATCH /me                  {display_name?, bio?, avatar?, banner?}
GET  /guilds               -> guilds with channels and members
GET  /channels/:id/messages?before=<id>&limit=50
POST /channels/:id/messages {body, attachment_ids[]}
GET  /channels/:id/messages/search?q=
POST /upload               multipart -> {id, url}
POST /channels/:id/join    -> {livekit_url, token, room}
```

### WebSocket `/events`

Server to client, one envelope:

```json
{ "type": "message.new", "data": { ... } }
```

Event types: `message.new`, `message.update`, `message.delete`,
`presence.update`, `typing`, `member.join`, `member.leave`, `channel.update`.

Client to server over the same socket: `typing`, `subscribe`, `unsubscribe`.
Writing messages goes over REST so retries are idempotent.

Presence and typing are ephemeral, in-memory only, never persisted.

## 7. Authentication and trust boundaries

- Passwords: argon2id, default parameters.
- Session tokens: 32 random bytes, returned once, stored as SHA-256 in
  `sessions`. Revocation is a row delete, which is why this beats JWTs here.
- Rate limits per IP and per user on login, register, upload and message send.
- Uploads: size cap, MIME allowlist, random filenames on disk, served through
  axum with `Content-Disposition` and no path interpolation from user input.
- All SQL through `sqlx` bound parameters. No string-built queries.
- The LiveKit token is scoped to one room, one identity, short TTL, and a
  permission set granting publish only for sources the user is allowed.

## 8. Media path

This is the risky half of the project and the reason to build at all.

- **Transport:** LiveKit server, self-hosted, Apache-2.0.
- **Client:** LiveKit Rust SDK, used from `client-core`, not the webview.
- **Capture, Windows:** Windows Graphics Capture for screen, WASAPI for mic.
- **Capture, Linux:** PipeWire through `xdg-desktop-portal` for screen, the
  portal or direct ALSA/PipeWire for mic.
- **Encoding:** hardware where available, NVENC and VAAPI on Linux, Media
  Foundation on Windows. Software x264 is the fallback, not the plan.
- **Rendering remote video:** native child surface inside the Tauri window,
  driven by wgpu, composited alongside the webview. This is the fiddliest
  integration in the project and the first thing to prove.

Webview `getDisplayMedia` is not used anywhere. It is unreliable on WebKitGTK
and would make the Linux half behave differently from the Windows half.

Targets: 1080p60 at roughly 4-6 Mbps per sharer, 48 kHz Opus voice with Opus
DTX and built-in noise suppression. Simulcast on so viewers on bad links drop
resolution instead of stalling.

Each sharer's upload is the real bottleneck, not the server.

## 9. Personalization

All of it is webview-side and needs no Rust:

- **Theme:** a theme JSON maps tokens to CSS custom properties. Ships with
  three built-ins; users can drop a JSON file in the data dir.
- **Fonts:** `@font-face` from bundled and user-supplied font files. Per-user
  font family, size and color override the theme defaults.
- **Avatars and banners:** static or animated WebP/GIF, served from disk. An
  `image` element with an animated source gives animation for free.
- **Profile:** `bio` rendered as plain text with newlines. Not Markdown, to
  avoid a renderer and an injection surface.

Per-user theme overrides apply only to that user's own client. There is no
shared server theme in v1.

## 10. Deployment and cost

- **Host:** Hetzner CX22 (2 vCPU, 4 GB, 20 TB) at roughly 4 EUR/month, or
  Oracle Cloud Always Free ARM (4 OCPU, 24 GB) at zero cost to start. Oracle's
  free tier reclaims idle instances, so expect to keep it warm.
- **Ports:** 443 for Caddy in front of the app. LiveKit takes 7880 (signaling),
  7881 (TURN over TCP) and a UDP media range, 50000-60000 by default; narrow
  the range to roughly 100 ports and confirm against the generated config
  before opening the firewall.
- **Bandwidth:** 10 viewers of a 4 Mbps stream is about 18 GB per hour of
  viewing. 20 TB is not a constraint.
- **Backups:** nightly `sqlite3 .backup` to a dated file, plus an offsite copy.
  Attachments are the bulk; lose them and history is half-broken.
- **TLS:** Caddy in front of both, automatic certificates.

## 11. Licensing

Kithos ships under the Kithos Personal Use License v1.0, verbatim in `LICENSE`.
It is source-available and proprietary, not OSI open source, and the spec and
README must say exactly that rather than "open source".

Practical consequences that affect the design:

1. **Public hosting of the source conflicts with the license.** GitHub's terms
   grant every user the right to fork and reproduce public repository content
   through the service. KPUL section 3.1 forbids making the source available
   outside the official repository, so the two cannot both hold for a public
   repo. Keep the repository private and grant access per person, or accept
   that section 3.1 is decorative.
2. **Binary distribution needs the license bundled.** KPUL section 4.4 requires
   distributed binaries to include a copy of the license. The build packages
   `LICENSE` as a client resource and the About screen renders it, so every
   build handed to a friend is compliant without a separate acceptance flow.
3. **Package metadata.** No SPDX identifier exists for KPUL, so the Rust crates
   use `license-file = "LICENSE"` rather than `license`, and SPDX-aware tooling
   sees `LicenseRef-KPUL-1.0`.
4. **Infrastructure costs are carved out.** KPUL section 5 states that paying
   for servers and bandwidth is not Commercial Use. That matches the hosting
   model in section 10 exactly, so renting the VPS is fine.

**The GPL trap in the media path.** `libx264` is GPL-2.0. Linking it makes the
distributed binary GPL, which overrides KPUL for the combined work and hands
every recipient source rights, defeating section 3.2 outright. Build FFmpeg
without `--enable-gpl` and without `--enable-libx264`, and encode through the
hardware paths plus `openh264` (BSD-2-Clause) for the software fallback. This
is a build-configuration decision, so it belongs in P0, not in cleanup.

Otherwise the dependency set is compatible: LiveKit Apache-2.0, Tauri
MIT/Apache-2.0, WebKitGTK LGPL-2.1 (dynamically linked). Third-party notices go
in `THIRD_PARTY_NOTICES` as KPUL section 7 anticipates.

## 12. Risks

| Risk | Likelihood | Mitigation |
| --- | --- | --- |
| Native video surface inside Tauri is harder than expected | High | Spike it first, before any other code |
| Linux capture varies by distro and portal version | Medium | Only support the distros the group actually runs; document them |
| 1080p60 software encode taxes weak CPUs | Medium | Hardware encode paths; accept 1080p30 or 720p60 per client setting |
| Webview memory creeps to Discord levels | Low | `client-core` is shell-agnostic; swap the shell if it happens |
| LiveKit upgrade breaks the Rust SDK | Low | Pin versions; LiveKit is actively maintained |
| SQLite write contention under concurrent uploads | Low | WAL, and this is 15 people |

## 13. Phases

**P0, media spike (the go/no-go).** A Tauri app with no chat at all: capture the
Linux screen natively, publish it, connect a second client on Windows, see
1080p60. If this cannot be made to work acceptably, the whole project changes
shape and the spec is revised before more is built.

**P1, server skeleton.** `proto` types, SQLite schema, register/login/session,
guilds and channels, message send and history pagination, WebSocket fan-out,
uploads. Verified with integration tests against a temp database.

**P2, client UI.** Chat view, history paging, theming, profile editing, avatars
and banners, and an About screen that renders the bundled `LICENSE`. No media
yet.

**P3, media integration.** Voice channels, join/leave, device selection,
screen share toggle, remote video surface, mute and deafen.

**P4, deploy.** Caddy, systemd units, backups, a `runbook.md` with the restore
procedure, and one rehearsed restore.

## 14. Verification per phase

- P0: record frame rate and CPU on both ends; the spike is a measurement, not a
  demo.
- P1: `cargo test` covering auth, authorization on a foreign channel, message
  pagination boundaries, and upload rejection paths.
- P2: RAM measurement (RSS) with a 500-message history open, compared against
  Discord on the same machine.
- P3: two-client call across the internet: CPU, bitrate, packet loss
  behaviour, and behaviour when a sharer has 5 Mbps upload.
- P4: restore from backup on a clean VPS.

## 15. Open questions

1. Does the group want message editing and deletion in v1? Cheap, but it adds
   event types and FTS triggers.
2. Voice channel capacity: does anyone expect more than 15 concurrent in one
   room?
3. Who holds the domain and the VPS account, and what happens if the author is
   unavailable for a week?
