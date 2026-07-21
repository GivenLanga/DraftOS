# draftos-desktop (Tauri 2)

The DraftOS GUI. A thin client: `src/main.rs` exposes Tauri commands that
delegate to the pipeline crates (`draftos-ingest`, `draftos-retrieval`,
`draftos-storage`, …); the frontend in `ui/` (plain HTML/CSS/JS, no build
step) only renders state and sends commands.

## Run

```bash
# from the workspace root
cargo run -p draftos-desktop            # dev
cargo build --release -p draftos-desktop # → target/release/draftos-desktop
```

Linux system dependencies (once):

```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
```

## Design constraints (from ../../CLAUDE.md)

- No business logic in the frontend — it renders state and sends commands.
- Ingestion and folder-watching run on background threads (one watcher thread
  per attached source), reporting progress to the UI via `source-updated` /
  `source-error` events. The window never blocks on indexing.
- Watchers are started for every attached source on launch (`setup`), and
  stopped when a source is detached or removed via an `AtomicBool` stop flag.
- App data (source registry + per-source index bundles) lives in the OS
  app-data dir; override with `DRAFTOS_DATA`.

## Layout

```
src/main.rs        Tauri commands + watcher lifecycle
tauri.conf.json    window + bundle config
build.rs           tauri-build
icons/icon.png     app icon (RGBA)
ui/index.html      markup
ui/styles.css      "paper, ink, and counsel's ribbon" theme
ui/main.js         command wiring + rendering
```
