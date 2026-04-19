# foundation-share-bridge

[![CI](https://github.com/Ravonus/foundation-share-bridge/actions/workflows/ci.yml/badge.svg)](https://github.com/Ravonus/foundation-share-bridge/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](./LICENSE)

Small cross-platform Rust service for handing website-triggered Foundation rescue shares to a local IPFS node.

This repo is the public source home for the desktop bridge. GitHub releases are
not published yet, so the Foundation Archive site links here for now instead of
to packaged downloads.

## What it does

- Runs a localhost HTTP service.
- Accepts a lightweight session handshake from your website.
- Pins supplied CIDs through a local or remote Kubo node.
- Persists a forever-watch list of rescued CIDs in `bridge-state.json`.
- Re-checks watched pins on a repair cadence and re-pins anything missing.
- Gives the Foundation archive website a clean place to hand off "share this work" or "share this profile" actions.
- Registers a desktop app link so the archive site can open the installed app directly.
- Exposes a simple local status UI and work-confirm page for local verification.

## Why this exists

The archive website is great for discovery, queueing, and verification. It should not be the only place someone depends on for durable access. This bridge makes it possible for artists or collectors to click a share button on the site and pin the same CIDs through their own IPFS-connected environment.

The important UX goal is simple: a non-technical artist can connect the bridge once, leave it running, and trust it to keep re-checking and re-pinning rescued roots over time.

## Desktop app link

When installed, the bridge registers this custom app URL:

```text
foundationsharebridge://pair?relay_server_url=...&pairing_code=...&device_name=...
```

The archive site can open that link directly. The protocol handler wakes the
local bridge if needed, forwards the pairing request into `POST /relay/link`,
and then opens the local status page for confirmation.

## Endpoints

- `GET /`
- `GET /health`
- `GET /sessions`
- `GET /session/:session_id`
- `GET /pins`
- `POST /session/connect`
- `POST /session/disconnect`
- `POST /pins/repair`
- `POST /ipfs/pin`
- `POST /share/work`
- `GET /share/work/view`
- `POST /share/work/form`
- `POST /share/profile`

## Environment

- `BRIDGE_HOST`
  Default: `127.0.0.1`
- `BRIDGE_PORT`
  Default: `43128`
- `IPFS_API_URL`
  Default: `http://127.0.0.1:5001`
- `IPFS_API_AUTH_HEADER`
  Optional auth header for remote or protected Kubo APIs.
- `SELF_REPAIR_INTERVAL_SECONDS`
  Default: `900`
- `BRIDGE_STATE_FILE`
  Default: `./bridge-state.json`
- `BRIDGE_CONFIG_FILE`
  Default: `./bridge-config.yaml`
- `BRIDGE_RELAY_SERVER_URL`
  Default: `https://foundation.agorix.io`
- `BRIDGE_DOWNLOAD_ROOT_DIR`
  Optional default export/sync folder.
- `BRIDGE_SYNC_ENABLED`
  Optional default auto-sync toggle.
- `LOCAL_IPFS_GATEWAY_BASE_URL`
  Default: `http://127.0.0.1:8080`
- `PUBLIC_IPFS_GATEWAY_BASE_URL`
  Default: `https://ipfs.io`

## Run

```bash
cargo run
```

With a custom IPFS API:

```bash
IPFS_API_URL=http://127.0.0.1:5001 cargo run
```

With a shorter repair loop while testing:

```bash
SELF_REPAIR_INTERVAL_SECONDS=120 cargo run
```

## Install from the CLI

Nothing is installed on a machine until someone explicitly runs an installer.

### macOS and Linux

```bash
./scripts/install/install.sh
```

Remove it later:

```bash
./scripts/uninstall/uninstall.sh
```

Delete the local watched-pin state and bundled Kubo repo too:

```bash
./scripts/uninstall/uninstall.sh --purge-data
```

### Windows

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install\install.ps1
```

Remove it later:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\uninstall\uninstall.ps1
```

Delete the local watched-pin state and bundled Kubo repo too:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\uninstall\uninstall.ps1 --purge-data
```

## What each installer sets up

### macOS

- per-user `LaunchAgent`
- per-user menu bar companion
- runtime under `~/Library/Application Support/FoundationShareBridge`
- starts after login
- container runtime expected: Docker Desktop or Colima

Useful runtime paths:

- LaunchAgent plist:
  `~/Library/LaunchAgents/com.ravonus.foundation-share-bridge.plist`
- Menu bar plist:
  `~/Library/LaunchAgents/com.ravonus.foundation-share-bridge.menu.plist`
- Runtime dir:
  `~/Library/Application Support/FoundationShareBridge`
- Logs:
  `~/Library/Application Support/FoundationShareBridge/logs`
- Persistent watched-pin state:
  `~/Library/Application Support/FoundationShareBridge/bridge-state.json`
- Editable bridge config:
  `~/Library/Application Support/FoundationShareBridge/bridge-config.yaml`
- Persistent Kubo repo:
  `~/Library/Application Support/FoundationShareBridge/data/kubo`

### Linux

- `systemd --user` service named `foundation-share-bridge.service`
- runtime under `${XDG_DATA_HOME:-~/.local/share}/foundation-share-bridge`
- starts after login
- container runtime expected: Docker or Podman

If you want it to keep running after logout too:

```bash
loginctl enable-linger "$USER"
```

### Windows

- user Task Scheduler item named `FoundationShareBridge`
- runtime under `%LOCALAPPDATA%\FoundationShareBridge`
- starts at logon
- container runtime expected: Docker Desktop

## Build installer artifacts

### macOS command bundle

```bash
./scripts/package/package-macos-installer.sh
```

That creates:

- `dist/FoundationShareBridge-<version>-macos-bundle`
- `dist/FoundationShareBridge-<version>-macos-bundle.tar.gz`

### macOS pkg flow

```bash
./scripts/package/package-macos-pkg.sh
```

That creates:

- `dist/FoundationShareBridge-<version>-macos.pkg`

If you also want a signed pkg, provide a signing identity first:

```bash
MACOS_INSTALLER_SIGNING_IDENTITY="Developer ID Installer: Your Name (TEAMID)" \
  ./scripts/package/package-macos-pkg.sh
```

That additionally creates:

- `dist/FoundationShareBridge-<version>-macos-signed.pkg`

The pkg installs CLI helpers at:

- `/usr/local/bin/foundation-share-bridge-install`
- `/usr/local/bin/foundation-share-bridge-uninstall`

and tries to auto-run the per-user background install for the currently logged-in user.

### Linux bundle

Run this on Linux, or provide a prebuilt Linux binary:

```bash
./scripts/package/package-linux-bundle.sh
./scripts/package/package-linux-bundle.sh --binary /path/to/foundation-share-bridge
```

That creates:

- `dist/FoundationShareBridge-<version>-linux`
- `dist/FoundationShareBridge-<version>-linux.tar.gz`

### Windows bundle

Run this on Windows, or provide a prebuilt Windows binary:

```bash
./scripts/package/package-windows-bundle.sh --binary /path/to/foundation-share-bridge.exe
```

That creates:

- `dist/FoundationShareBridge-<version>-windows`
- `dist/FoundationShareBridge-<version>-windows.zip`

### Build everything you can from one command

```bash
./scripts/package/package-release-bundles.sh
```

On the current host OS it builds native artifacts automatically. You can also
feed in prebuilt foreign binaries:

```bash
FOUNDATION_SHARE_BRIDGE_BINARY_LINUX=/path/to/foundation-share-bridge \
FOUNDATION_SHARE_BRIDGE_BINARY_WINDOWS=/path/to/foundation-share-bridge.exe \
./scripts/package/package-release-bundles.sh
```

If you want to cross-compile foreign binaries instead of providing prebuilt ones,
you still need the matching C toolchains on the build machine. For example:

- Linux musl on macOS needs `x86_64-linux-musl-gcc`
- Windows GNU on macOS needs `x86_64-w64-mingw32-gcc` and `x86_64-w64-mingw32-dlltool`

## Website flow

1. The website creates a pairing code and desktop app link like `foundationsharebridge://pair?...`.
2. The installed protocol handler opens the local bridge and forwards the pairing into `POST /relay/link`.
3. Once paired, the website can queue pin jobs over the lightweight relay socket.
4. The bridge pins supplied CIDs through Kubo, adds them to the forever-watch list, and keeps re-checking them on each repair cycle.
5. If sync mode is enabled, the bridge also exports those watched roots into the configured download folder and reports local/public IPFS links back through the status APIs.

## Config and sync

The bridge now has a persistent config file with:

- `download_root_dir`
- `sync_enabled`
- `local_gateway_base_url`
- `public_gateway_base_url`

You can drive it from the Foundation archive site’s `/desktop` board or directly through the bridge API:

- `GET /config`
- `POST /config`
- `POST /sync/run`

With `sync_enabled=false`, the bridge still pins and watches roots forever. It just waits for a manual sync run before exporting files into the download folder.

The helper settings page also includes a quick-fill flow for external gateway links:

- If you have a hostname or DDNS name, type it once and the helper will build the external pinned gateway URL for you.
- If you do not have a hostname yet, the helper will try to detect your public IPv4 address and offer a direct IP gateway URL as a fallback.
- Inventory cards keep that external "Open pinned" route separate from the public IPFS fallback link.

## Local pages and fallback flows

- The bridge still exposes `GET /share/work/view` if you want a local browser confirmation page.
- The macOS menu bar app can open the local UI, reveal the YAML config, and update the relay target plus desktop name while the bridge is running.
- The desktop app link is the preferred pairing flow for installed apps.
- For same-machine debugging, the site can still talk to the bridge directly over localhost.

## Example: connect a session

```bash
curl -X POST http://127.0.0.1:43128/session/connect \
  -H 'content-type: application/json' \
  -d '{
    "website_origin": "https://archive.example.com",
    "account_address": "0x1234...",
    "profile_username": "waambat",
    "client_name": "foundation-archive-site"
  }'
```

## Example: share a work

```bash
curl -X POST http://127.0.0.1:43128/share/work \
  -H 'content-type: application/json' \
  -d '{
    "session_secret": "paste-session-secret",
    "title": "Deep Linking",
    "contract_address": "0x3b3ee1931dc30c1957379fac9aba94d1c48a5405",
    "token_id": "38901",
    "foundation_url": "https://foundation.app/mint/eth/0x3B3ee1931Dc30C1957379FAc9aba94D1C48a5405/38901",
    "metadata_cid": "QmVHcT4y2Hhawbp3tdyH5FDSWnDuDN4qMca2miYmwcSUYK",
    "media_cid": "QmaiwyfXxF6zRByzZTaJ4KDfzg7s1WNmP6Rt5rLPYmPXsg",
    "artist_username": "waambat"
  }'
```

## Example: inspect watched pins

```bash
curl http://127.0.0.1:43128/pins
```

## Example: trigger a repair cycle immediately

```bash
curl -X POST http://127.0.0.1:43128/pins/repair
```

## Example: share a profile batch

```bash
curl -X POST http://127.0.0.1:43128/share/profile \
  -H 'content-type: application/json' \
  -d '{
    "session_secret": "paste-session-secret",
    "account_address": "0xabc...",
    "username": "waambat",
    "label": "waambat archive batch",
    "cids": [
      "QmVHcT4y2Hhawbp3tdyH5FDSWnDuDN4qMca2miYmwcSUYK",
      "QmaiwyfXxF6zRByzZTaJ4KDfzg7s1WNmP6Rt5rLPYmPXsg"
    ]
  }'
```

## Development

```sh
cargo build
cargo run
```

### Lint before committing

```sh
bash scripts/lint/run-all.sh
```

Runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo deny check` (if installed), plus repo-level guards for file size (<=600 lines), folder fanout (<=6 children), and monolith heuristic.

### Code conventions (enforced by CI)

- Files: no `.rs` over 600 lines (warn at 400)
- Functions: no function over 80 lines
- Arguments: no function with more than 4 parameters
- Folders: no directory under `src/` or `scripts/` with more than 6 direct children
- No `unwrap` / `panic!` / `dbg!` / `unsafe`; `expect` warn-only with an invariant comment

See [CONTRIBUTING.md](./CONTRIBUTING.md) for the full guide.

## License

Apache-2.0 — see [LICENSE](./LICENSE).
