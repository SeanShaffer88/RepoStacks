# Changelog

## 0.4.0 — 2026-11-11
- Search grows negation and freshness: prefix any term with `-` to
  exclude it (`-lang:c`, `-topic:games`, `-is:installed`), and find
  volumes that are due for a check with `is:due` / `is:never`.
- Metadata refresh is parallel (CLI and window), bounded by the
  concurrency setting — large catalogues no longer pay one network
  round-trip per repo in sequence. Bulk imports deduplicate against a
  set instead of scanning, keeping big imports fast.
- New CLI commands: `export` (the catalogue as an importable list, or
  full JSON with `--json`) and `clean` (removes download folders of
  repos no longer catalogued and orphaned `.part` files; dry run
  unless `--yes`). `doctor` also reports library health: volumes never
  checked, volumes due, failed jobs, and what a clean would reclaim.
- Window: volume pages gain "Open on GitHub" and "Open download
  folder"; the Librarian can run a pass when the app opens (a new
  setting, off by default); saved searches are manageable from the
  filter panel; the ledger's columns are choosable in the View section;
  Acquisitions gains "Retry all failed"; check and refresh results now
  report failures in the toast instead of only in the log.
- Accessibility: catalogue entries open by keyboard (Enter/Space) in
  every layout, the Add dialog traps Tab while open, Ctrl+F also
  focuses search, and the result count is announced to screen readers.
- Reliability & safety: state saves flush to disk before the atomic
  rename; the Librarian's scheduler survives a failing pass and logs
  it; asset names and release tags from the network are validated as
  plain file names before they become paths (in the planner and again
  in the engine, since the queue file is editable text).

## 0.3.0 — 2026-11-11
- Import a GitHub user's projects: the Add Volumes dialog and the CLI
  (`import-user`) list everything a user has on GitHub and import it with
  filters — only owned or member-of, forks or original projects, archived
  skipped/included/only, public or private. A preview shows the counts
  before anything is added; imports arrive with description, stars,
  language, license, and topics already filled in, and duplicates are
  skipped. One API call per 100 repos, and the spend is reported.

## 0.2.1 — 2026-11-11
- No more flashing command-prompt windows: every background curl call
  (release checks, metadata refreshes, asset listings, downloads) now runs
  without a console window attached.
- Fixed the Standing order checkbox on the volume page stacking above its
  label instead of sitting beside it.

## 0.2.0 — 2026-11-10
- Reliability: state files (catalogue, queue, settings) keep a backup generation;
  a corrupt file is quarantined and the backup restored instead of losing the
  library. Downloads verify the transferred size against the listing (a partial
  file stays for resume), and fetches retry once on a dropped connection.
- Security: download URLs must be https on GitHub hosts, checked when a job is
  planned and again when it runs.
- Performance: release checks run in parallel (bounded by the concurrency
  setting) in the CLI, the window, and the Librarian; the window no longer
  freezes while checks or metadata refreshes run.
- Downloads: failed jobs can be retried (CLI `queue retry`, Acquisitions ↻);
  the auto-pick explains its score and prefers the program over its setup
  program or MSI package.
- Search: `tag:` filter; `sort:added` now uses the real date a volume was
  added (new `added_unix` field, older catalogues read as 0); owner and
  license facets join language and topic (CLI `facets`, window filter panel).
- CLI: `--json` output for list/find/updates/facets/queue list; `doctor`
  health check; `config` validates values before saving.
- Window: the Librarian's metadata pass now actually refreshes when due and
  logs its API spend; state-load failures appear in the activity log;
  settings saves are validated.
- Interface: catalogue sort control (remembered between launches); keyboard
  shortcuts (Ctrl+K or / for search, Esc closes the Add dialog, which now
  takes and returns focus properly); screen-reader semantics (aria-current
  navigation, live-region toasts, labelled controls, progress bars);
  removing a volume asks for a confirming second click; an empty library
  shows an Add call to action; the asset list shows why the auto-pick
  chose what it chose.

## 0.1.0 — 2026-11-10
- Released as `RepoStacks-v0.1.0-x64-Setup.exe` + `RepoStacks-v0.1.0-x64-Portable.zip`
  + `SHA256SUMS.txt`. The command line ships as `repostacks-cli.exe` so it can sit
  beside `RepoStacks.exe` (the two names differ only by letter case, which Windows
  treats as one filename).
- Window slice: Tauri app with the dark ledger design (four layouts, four themes,
  faceted sidebar), Shelves / My Shelf / Acquisitions / book-page detail / Add Volumes
  screens, the Librarian scheduler with standing orders and activity log, DPAPI token
  storage. NSIS per-user installer script under `scripts/`.
- Download slice: release asset listing, download engine with Range resume and SHA-256
  verification against published digests, persisted acquisitions queue, per-volume asset
  rules, standing-order flags, Librarian settings (download folder, concurrency, polls).
- Network + search/view slice: live Atom release checks (zero quota), policy-governed metadata
  refresh with quota reporting, rich search language with saved searches, switchable
  layouts/themes/density/grouping/columns, bulk import with preview counts, case-insensitive
  repo identity.
- Foundation slice: core library + CLI (catalogue, shelves, facets, Atom parsing, corrected rate-limit policy, asset rules, queue state, persistence).
- Project classified t3; rule sheet, quality scorecard, and parity matrix created.
