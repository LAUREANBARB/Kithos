# Third-Party Notices

Kithos itself is licensed under the Kithos Personal Use License v1.0 (see
`LICENSE`). That license covers only original Kithos materials. The components
below remain under their own licenses and terms, as provided in KPUL section 7.

This file is currently maintained by hand, because no dependencies are vendored
yet. **It must be regenerated from `Cargo.lock` before the first distributed
binary.** See "Keeping this file correct" at the bottom.

## Rust crates

Direct dependencies as of the design phase. Versions are the current releases
at the time of writing, not pins; licenses were read from crates.io metadata.

| Crate | License |
| --- | --- |
| `tauri` | Apache-2.0 OR MIT |
| `axum` | MIT |
| `tokio` | MIT |
| `tower-http` | MIT |
| `tracing` | MIT |
| `sqlx` | MIT OR Apache-2.0 |
| `serde` | MIT OR Apache-2.0 |
| `argon2` | MIT OR Apache-2.0 |
| `jsonwebtoken` | MIT |
| `rand` | MIT OR Apache-2.0 |
| `reqwest` | MIT OR Apache-2.0 |
| `uuid` | Apache-2.0 OR MIT |
| `wgpu` | MIT OR Apache-2.0 |
| `image` | MIT OR Apache-2.0 |
| `openh264` | BSD-2-Clause |
| `livekit` | Apache-2.0 |

Transitive dependencies are not listed yet. They typically number in the
hundreds for a Tauri application and must come from the lockfile, not from
memory.

## Non-Rust components

| Component | Role | License |
| --- | --- | --- |
| LiveKit server | SFU, run as a separate service | Apache-2.0 |
| SQLite | database engine | Public domain |
| WebKitGTK 4.1 | Linux webview, dynamically linked system library | LGPL-2.1-or-later |
| GTK 3 | Linux windowing, dynamically linked | LGPL-2.1-or-later |
| FFmpeg | encoding and decoding, if used at all | LGPL-2.1-or-later, but only when built without `--enable-gpl` |
| Caddy | TLS termination on the server | Apache-2.0 |

WebKitGTK and GTK are used as dynamically linked system libraries supplied by
the user's distribution. Distributing Kithos does not redistribute them, and
LGPL linking obligations are satisfied by that dynamic linking.

## Deliberately excluded

**`libx264` is not used and must not be added.** It is GPL-2.0, and linking it
would place the whole binary under GPL terms, which overrides KPUL for the
combined work and hands every recipient source rights that KPUL section 3.2
forbids granting. FFmpeg must therefore be built without `--enable-gpl` and
without `--enable-libx264`. Encoding uses the hardware paths plus `openh264`
for the software fallback.

**No AGPL components.** Any dependency that turns out to be AGPL-3.0 is a
blocker for distribution and needs replacing, not documenting.

## The openh264 caveat

`openh264` reports BSD-2-Clause, but the binary that Cisco publishes carries
additional terms: Cisco covers the AVC patent royalties only for the unmodified,
correctly named binary it distributes. If Kithos ships a compiled artifact that
bundles openh264, confirm those terms at packaging time rather than assuming
plain BSD-2-Clause behaviour. This matters the first time a build is handed to
someone.

## Keeping this file correct

Once dependencies exist:

```bash
cargo install cargo-about cargo-deny
cargo about generate about.hbs -o THIRD_PARTY_NOTICES.md
cargo deny check licenses
```

`cargo deny` is the enforcement half: it fails the build when a new dependency
sneaks in under a license that this file would not permit. Without it, the
notices file drifts the first time someone adds a crate.
