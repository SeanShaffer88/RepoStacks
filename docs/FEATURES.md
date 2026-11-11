# Features

## Built (foundation slice)
- Catalogue add/remove with strict `owner/repo` validation (plain, host, and full URL forms).
- Shelves: named collections, a volume on many shelves, removal cleans shelves.
- Faceted filtering: text, language, topic, license, installed-only, has-release; language/topic facet counts.
- Atom feed parsing for release watching (tags, titles, times, links), tolerant of malformed entries.
- Rate-limit policy engine encoding GitHub's documented rules, including the anonymous-304 correction.
- Asset selection rules with `*`/`?` glob matching, case-insensitive.
- Download queue state machine with dedupe, pause/resume/cancel guards, progress.
- Catalogue persistence: JSON with `schema_version`, atomic saves, schema guard on load.
- Full command line for every core capability.

## Built (network + search/view slice)
- Live release checks over Atom feeds (`check`, `check-all`) — zero API quota, verified live.
- REST metadata refresh (`refresh`) under the cache policy: interval-gated when anonymous,
  ETag-aware, quota spend reported per run. Optional token via `REPOSTACKS_TOKEN` env var.
- HTTP transport is the system `curl` behind a trait — zero crate dependencies, testable offline.
- Rich search (`find`): field filters (`owner:` `lang:` `topic:` `license:` `shelf:"…"`),
  star ranges (`stars:>1000`, `stars:100..5000`), status filters (`has:update`, `is:installed`,
  `is:outdated`, `has:auto`), sorting (`sort:-stars`), `limit:`, quoted phrases, free text.
- Saved searches (`search save|list|run|delete`) in the settings file.
- Switchable presentations (`view`): layouts ledger / cards / compact / diagram; themes
  dark-ledger (default) / paper-ledger / clean-sheet / terminal; density; grouping by
  shelf / language / license / status; chosen ledger columns; facet panel toggle.
- Bulk import (`import`): one repo per line from a file or stdin, preview counts
  (valid / duplicate / invalid), optional shelf assignment. Repo identity is
  case-insensitive, matching GitHub.

## Built (download slice)
- Release asset listing (`assets`): names, sizes, and whether GitHub publishes a SHA-256
  digest — the one REST call a release costs, reported as such.
- Download engine (`download`, `update`, `update-all`, `queue run`): picks the asset by the
  volume's glob rules or a Windows-first default, downloads to a `.part` file with Range
  resume, verifies SHA-256 against the published digest (mismatch = file deleted, job
  failed with the reason), then lands the file under `<download root>/<owner>-<repo>/<tag>/`
  and marks the volume installed.
- Acquisitions queue persisted to disk (`queue list|run|pause|resume|cancel|clear`);
  a job interrupted mid-download requeues on load and resumes from its `.part`.
- Per-volume asset rules (`rules`) and standing orders (`auto on|off`).
- Librarian settings (`config`): download folder, concurrency, Atom/metadata poll intervals.

## Built (window slice)
- **The window** (Tauri, `window/`): the dark ledger design — catalogue with faceted
  filter sidebar (live facet counts), four switchable layouts (ledger table, card grid
  with language-colour covers, compact list, shelf diagram), four themes (dark ledger,
  paper ledger, clean sheet, terminal), density and group-by controls — all driven by
  the same saved `ViewConfig` the CLI uses.
- Screens adapted from the library concepts: Catalogue, Shelves (volumes as spines on
  planks), My Shelf (updates split from up-to-date, Update all), Acquisitions (queue
  with progress and pause/resume/cancel), the book-page volume detail (release history,
  asset list with the auto-pick marked, per-volume asset rules, standing-order toggle),
  the Add Volumes dialog (single + bulk import with preview report), and the Librarian.
- **The Librarian**: background scheduler on the saved poll intervals — Atom checks
  for the whole catalogue, standing-order volumes download new releases automatically,
  activity log in-app; a pass can also be run on demand.
- Token storage via DPAPI (Windows), protected at rest, unprotected only in memory;
  anonymous mode remains the default and fully functional.
- Post-install steps remain out of scope by design (see ARCHITECTURE): the library
  downloads and verifies; it never runs anything it fetched.

## Built (suggested-rules slice, v0.5.0)
- **Self-suggesting asset rules**: `suggest_asset_patterns` (core) turns a
  release's file names into rules — version generalized to `*`, sidecars
  skipped, auto-pick first. The volume page auto-fills the rules box when
  a volume has no rules yet (unsaved until Save rules), a Suggest button
  refills on demand, and CLI `assets` prints the same list.

## Built (round-2 enhancement slice, v0.4.0)
- Search negation (`-term` for every filter kind) and freshness filters
  (`is:due`, `is:never`); the query engine takes the clock explicitly.
- Parallel metadata refresh everywhere; set-based dedup for bulk imports.
- CLI `export` (list or full JSON) and `clean` (stale download folders +
  orphan `.part` files, dry-run first); `doctor` library-health lines.
- Window: Open on GitHub / Open download folder, check-on-start setting,
  saved-search management in the filter panel, ledger column picker,
  Retry all failed, failure-reporting toasts, scheduler panic guard.
- Accessibility: keyboard-openable entries in all layouts, Add-dialog
  focus trap, Ctrl+F search focus, live result count.
- Safety: fsync before atomic rename; asset-name/tag path validation in
  the planner and the engine.

## Built (user import slice, v0.3.0)
- **Import a GitHub user's projects** (CLI `import-user`, Add Volumes dialog):
  paginated listing (100 per page, one API call each, spend reported), filters
  for ownership (all/owned/member), forks (all/originals/forks), archived
  (skip/include/only), and visibility (all/public/private); preview counts
  first; entries arrive pre-filled with description, stars, language, license,
  and topics; duplicates are skipped.

## Built (hardening slice, v0.2.0)
- **Corruption-proof state**: catalogue, queue, and settings keep a backup
  generation; a damaged file is quarantined and the backup restored.
- **Parallel release checks** in CLI, window, and Librarian; the window stays
  responsive during checks and refreshes.
- **Safer downloads**: only https GitHub hosts are fetched; transferred size is
  verified against the listing; failed jobs retry (resuming from the partial
  file); the auto-pick explains its score and prefers the program over its
  setup program or MSI.
- **Search**: `tag:` filter, real added-date sorting, owner and license facets.
- **CLI**: `--json` output for the data commands, a `doctor` health check, and
  validated `config` values.
- **Librarian**: the metadata pass refreshes for real when due and logs spend.
- **Interface**: catalogue sort control, keyboard shortcuts, screen-reader
  semantics, confirm-before-remove, and an empty-library call to action.

## Planned
- API-usage meter fed by the cache policy's accounting on the Librarian screen
  (spend is already reported per refresh; a standing meter is polish).
- Tauri window over the core operations, built to the owner's eight screen concepts
  (adapted): catalogue grid with faceted filter sidebar, shelf view, book-page repo
  detail with release history and per-repo asset rules, acquisitions queue, dense table
  with multi-select batch actions, My Shelf (installed/updates), bulk-import dialog,
  Librarian dashboard. Default presentation is the dark ledger table.
- Token storage via DPAPI in the window slice (never in the catalogue file).
- Standing orders: auto-download per repo, Update All per shelf.
- Release: Setup.exe + Portable.zip + SHA256SUMS.txt (x64).
