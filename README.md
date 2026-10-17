# jellydownloader

A Jellyfin downloader in pure Rust. Browse your libraries, pick a quality, download.

- **Login** with your Jellyfin credentials (any local or public server URL)
- **Original quality** — direct file copy with HTTP Range, so pause/resume is exact
- **Lower qualities** — Jellyfin transcodes (HLS); we download segments, resume per
  segment for a real progress bar, and ffmpeg remuxes to a clean `.mp4` at the end
- Parallel jobs (3 by default), pause / resume / stop / restart / remove
- Web UI in [Leptos](https://leptos.dev) (WASM), backend in Axum

## Prerequisites

- Rust stable + `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`
- `ffmpeg` on PATH (only needed for transcoded downloads)
- [trunk](https://trunkrs.dev) + wasm-bindgen CLI for the frontend:
  ```
  cargo install --locked trunk wasm-bindgen-cli
  ```
  (wasm-bindgen-cli version must match the `wasm-bindgen` crate version in `Cargo.lock`)

## Build & run

From the repo root:

```sh
# 1. frontend (generates frontend/dist)
cd frontend && trunk build && cd ..

# 2. backend (serves the API and the frontend)
cargo run -p backend
```

Then open **http://localhost:3001**, enter your Jellyfin URL, sign in, browse, download.

The backend binary also serves the compiled frontend, so there is only one process
to run in production.

### Testing without a Jellyfin server

A fake Jellyfin is included (generates test media with ffmpeg, serves the same API
endpoints, throttles the original-stream endpoint so you can exercise pause/resume):

```sh
cargo run -p backend --bin mock-jellyfin   # listens on 127.0.0.1:8095
```

Log into it from the UI with any username/password.

## Configuration

`config.json` is created at the repo root on first run:

| key                 | default          | notes                                   |
|---------------------|------------------|-----------------------------------------|
| `port`              | `3001`           | HTTP port                                |
| `downloads_dir`     | `downloads`      | finished files land here                 |
| `data_dir`          | `data`           | job state + sessions (survive restarts)  |

(With the shipped compose, the app's paths *inside the container* should
mirror the host drive: `downloads_dir: /data/jellydownloader/downloads` and
`data_dir: /data/jellydownloader/data` — one namespaced home per app, no
bare-generic directories on a shared drive.)
| `parallel_downloads`| `3`              | max concurrent jobs                      |
| `cleanup_hours`     | `24`             | auto-delete finished files after N hours; `0` disables |
| `frontend_dist`     | `../frontend/dist`| override with `JD_DIST` env var         |

## Deployment (exposed externally)

Deploy it next to Jellyfin and share **one** URL (the app's). At login, users should
enter the **local** Jellyfin URL (e.g. `http://192.168.1.100:8096`) — the backend
talks to Jellyfin locally (fast, no hairpin NAT), and everything else is served
through the app itself: posters are proxied via `/api/image/...`, so they work for
external users too, and Jellyfin tokens never appear in image URLs.

```caddy
# Caddyfile — gives you HTTPS + Secure cookies automatically
jdl.example.com {
    reverse_proxy 127.0.0.1:3001
}
```

**Files land on the server.** The item modal and episode picker include a
*"Save to my device when done"* checkbox (on by default): when the job completes
and the user's browser is still open, the browser starts saving the file
automatically. If the tab was closed in the meantime, the save icon on the
completed row does the same manually. The transfer streams through the app and
supports HTTP Range, so the browser's download manager gets native
pause/resume.

Note: the server does the transcoding work twice in a sense — Jellyfin transcodes,
then ffmpeg remuxes — so watch `parallel_downloads` on weak hardware.

## Layout

```
backend/   axum server, Jellyfin client, download queue + engine, mock server
frontend/  leptos CSR app (trunk), hand-rolled dark UI
shared/    API types shared by both sides
```

Requesting the same item + quality twice never re-transcodes: identical jobs are
deduplicated at creation (completed ones are reused instantly, anything in-flight
is pointed at instead). Finished files are wiped 24 h after completion unless
`cleanup_hours` says otherwise.

Paused/stopped jobs keep their partial data and continue where they left off;
state is persisted across app restarts. Transcoded jobs are resumable per segment
(Jellyfin kills its transcode session on disconnect, so segments already received
are kept and the remainder is re-requested).
