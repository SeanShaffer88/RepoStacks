# Architecture

## Shape

One Rust crate, two surfaces over a single operations library:

- `repostacks_core` — every capability as an operation: catalogue, shelves,
  faceted filter, Atom feed parsing, asset glob rules, download-queue state,
  rate-limit policy, catalogue persistence (`schema_version` 1, atomic writes).
- `repostacks` CLI — a thin surface over the library. The Tauri window will be
  a second thin surface over the same operations; nothing lives only in a
  click handler.

## The corrected rate-limit design

The original blueprint claimed conditional requests (ETag / `304 Not
Modified`) do not count against GitHub's rate limit. GitHub's documentation
says that is true **only when the request was correctly authorized**; an
unauthenticated 304 still decrements the 60/hour anonymous quota. RepoStacks
is designed around the corrected rule:

1. **Release detection: Atom feeds only.** `releases.atom` is served from
   github.com, outside the REST API, so polling it spends no REST quota. It
   carries tags and times but no asset lists, and it includes pre-releases.
2. **Assets: one REST call per actual new release** (`/releases/latest`),
   never per poll.
3. **Metadata (stars, description, topics):** cached in the catalogue.
   Anonymous refresh is on-demand and at most daily; ETags save bandwidth,
   not quota, without a token. With a token (5,000/hour), ETag polling is
   genuinely free on a 304, so the refresh interval shortens.
4. **Downloads** use the asset's direct URL — ordinary HTTPS, no API quota.
5. **Token storage:** DPAPI-protected on Windows, never in the catalogue
   file, never logged.

These rules live in code in `src/cache_policy.rs`, with tests that fail if
the anonymous-304 behaviour is ever "simplified" back to the blueprint's
version.

## Data

`data/catalog.json` (JSON, `schema_version`, atomic rename-on-save). SQLite
arrives with the Tauri slice if the catalogue outgrows a single file; the
store sits behind the `Catalog` type so the swap touches one module.

## Decisions

- 2026-11-10 — Windows-only v1 (owner's pick); cross-platform deferred.
- 2026-11-10 — Name: RepoStacks (owner's pick; zero same-name repos on GitHub).
- 2026-11-10 — Core-first, zero-dependency foundation; Tauri/SQLite/HTTP
  dependencies are introduced slice by slice, each with owner approval.
- 2026-11-10 — TesselGUI design standard does not apply to RepoStacks (owner's call). Window design approved the same day: the **dark ledger** — spreadsheet-style tables on a dark ground (mockups v2).
- 2026-11-10 — HTTP transport is the system `curl` behind a `Fetcher` trait: zero crate
  dependencies, works on Windows 10 1803+ / Windows 11 out of the box, and every network
  operation stays testable with a scripted fetcher. A bundled HTTP crate can replace it
  later without touching the operations.
- 2026-11-10 — The owner supplied eight window screen concepts as ideas to adapt (grid
  catalogue with facet sidebar, shelf view, book-page detail, acquisitions, dense table,
  My Shelf, bulk-import dialog, Librarian dashboard). They shape the window slice; the
  approved default presentation remains the dark ledger table, with the other layouts
  one switch away — the same `ViewConfig` the CLI already renders from.
- 2026-11-10 — Known feed characteristic: a repo that publishes releases for several
  components (e.g. BurntSushi/ripgrep also releases its `ignore` crate) surfaces the
  newest component release in its Atom feed. The book-page detail's release history is
  where a user picks the release they actually track.
- 2026-11-10 — The window is a separate crate (`window/`) over the core library, built
  with Tauri's `custom-protocol` feature so the UI ships inside the binary; the
  installer is a project-owned NSIS script (not the Tauri bundler), which keeps the
  release shape identical to the house standard: Setup.exe + Portable.zip +
  SHA256SUMS.txt. The window stores its data beside the executable (`data/`, the
  portable rule) and honours the CLI's environment overrides.
- 2026-11-10 — The token is stored only as a DPAPI-protected blob (base64) in the
  settings file and unprotected in memory on demand; the CLI keeps using the
  `REPOSTACKS_TOKEN` environment variable. Neither surface ever writes a plain token.
- 2026-11-10 — Downloads land via a `.part` file in the volume's own folder and are
  renamed into place only after SHA-256 verification against the digest GitHub
  publishes for the asset; a mismatch deletes the bytes and fails the job with the
  reason. A `.part` left by an interrupted run is resumed (HTTP Range), and a job
  found mid-download on load requeues — crash recovery falls out of the state
  machine instead of being a special case.
- 2026-11-10 — The engine runs jobs on scoped threads (bounded by the concurrency
  setting) from immutable job snapshots and applies outcomes serially, so the
  queue and catalogue never need locks.
- 2026-11-10 — Download-only by default; post-download execution is a
  per-repo explicit opt-in, never automatic.
