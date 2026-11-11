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
- **Downloads that land verified** — the engine picks the right file by the
  volume's glob rules or a Windows-first default, resumes interrupted downloads,
  and checks SHA-256 against the digest GitHub publishes before the file counts
  as installed. The Acquisitions queue persists across restarts.
- **The window** — a dark ledger desk for the library: faceted filter sidebar,
  four switchable layouts (ledger, card covers, compact list, shelf diagram),
  four themes, book-page volume details, and the Librarian — a built-in
  scheduler that checks the shelves on your interval and downloads standing
  orders automatically.
- **No login required** — everything above works with public data. An optional
  personal access token (stored protected by Windows DPAPI, never in plain
  text) raises the API limit and makes metadata refresh cheaper.

## Status

v0.1.0: the core library, command line, live network fetching, download engine,
and the window are built. Windows runtime behaviour is unverified until it runs
on a real Windows machine — the first field pass decides what gets polished
next. Screenshots will be added from a real run.

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
repostacks find "lang:rust stars:>10000 sort:stars"
repostacks check-all
repostacks refresh --all
repostacks download casey/just
repostacks update-all
repostacks queue list
repostacks view layout cards
```

The window is the same library on a desk: run `RepoStacks.exe`, or build it
with `cargo build --release --features custom-protocol --manifest-path window/Cargo.toml`.

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
