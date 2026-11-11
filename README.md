# RepoStacks

## TL;DR

RepoStacks is a library for your GitHub programs. Think of each repository as a
book: you keep the ones you care about on shelves, the app quietly watches for
new editions (releases), and when one appears it fetches the right copy for
your computer — all without needing a GitHub account.

## What it does

- **Catalogue** — add repositories by `owner/repo` or GitHub URL and browse
  them as volumes, with search and faceted filters (language, topic, license,
  stars, installed).
- **Shelves** — group volumes into collections; a repo can sit on many shelves.
- **Quota-free release watching** — new releases are detected from GitHub's
  public Atom feeds, which do not use the 60-requests-per-hour anonymous API
  limit. See `docs/ARCHITECTURE.md` for the corrected rate-limit design.
- **Acquisitions queue** — downloads with per-repo asset rules (glob patterns),
  pause/resume, and checksum fields (engine lands in the download slice).
- **Standing orders** — per-repo auto-download toggles with an update badge
  when a newer release than the installed one exists.
- **No login required** — everything above works with public data. An optional
  personal access token (stored protected by Windows DPAPI, never in plain
  text) raises the API limit and makes metadata refresh cheaper.

## Status

Foundation slice: the core library and command line are built and tested.
The window (Tauri), live network fetching, and the download engine arrive in
later slices, in that order — the design is approved before window code is
written. Windows runtime behaviour is unverified until it runs on a real
Windows machine.

## Command line

Every capability is a command first. Examples:

```text
repostacks add rust-lang/rust
repostacks shelf add "Dev Tools" rust-lang/rust
repostacks list
repostacks search compiler
repostacks facets
repostacks feed-url rust-lang/rust
repostacks check-feed releases.atom
repostacks match "*-win64.zip" app-win64.zip app.dmg
repostacks updates
```

The catalogue lives in `data/catalog.json` (override with the `REPOSTACKS_DATA`
environment variable), carries a `schema_version`, and is written atomically
so a crash never loses it.

## Build

```text
cargo test
cargo build --release
```

## Screenshots

Captures will be added from real runs on Windows once the window exists —
none are shown yet because there is no window to capture.

## License

MIT — see `LICENSE`.
