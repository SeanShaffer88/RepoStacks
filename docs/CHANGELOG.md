# Changelog

## 0.10.0 — 2026-11-11

- A built-in benchmark: the new CLI command `bench` times the core
  operations — catalogue save and load, the rich search, the facet
  counts, the Atom feed parser, and the parallel release-check
  pipeline (with the network replaced by a stub, so it measures the
  app's own work) — over a synthetic catalogue of 5,000 volumes
  (`--repos N` changes the size, `--json` gives machine-readable
  output). The generator is fixed-seed, so runs are comparable over
  time. The first recorded run and the method behind it are in
  `docs/PERFORMANCE.md`: on the recording machine a 5,000-volume
  catalogue (2.7 MB on disk) loads in about 57 ms, searches take
  single-digit milliseconds, and a release check costs about
  250 µs of the app's own work per repo.

## 0.9.0 — 2026-11-11

- A standing API-usage meter: every REST call RepoStacks counts —
  metadata refreshes, release asset lists, release-check fallbacks,
  and account imports — is recorded in a small local ledger
  (`usage.json`, kept with the same backup-and-recovery discipline
  as the other state files). The new CLI command `usage` prints the
  current hour against the hourly limit, today (UTC), and lifetime
  totals, each split by purpose, with `--json` for scripts; `doctor`
  reports the ledger's health. The Librarian screen shows the same
  meter with a bar. The meter is honest about what it is: it counts
  this app's own calls only — it is not a live GitHub reading, and
  the free Atom release checks are never counted.
- CI and release workflows moved to `actions/checkout@v5` (the
  Node 24 runtime), clearing the Node 20 deprecation warnings.

## 0.8.0 — 2026-11-11

- One instance only: a second launch now steps aside with a short
  explanation instead of sharing the data folder, where two copies'
  saves would silently overwrite each other's edits (named-mutex
  guard, no new dependencies).
- Downloads run on a background worker: Download, Run queue, Retry,
  shelf updates, and the Librarian's standing orders no longer hold
  the app's state lock for the length of a transfer, so the window
  stays responsive throughout — and Acquisitions shows live
  progress, speed, and ETA from the partial file while a download
  runs (previously the progress view could not even refresh during
  a window-started run). Retried jobs now start on their own.

## 0.7.0 — 2026-11-11

- Release checks degrade gracefully when a repo has no releases
  feed (the project's standing rule): the tags feed answers first —
  still quota-free — and only a repo with neither feed costs one
  REST call, which is counted and reported in the activity log and
  CLI output instead of being hidden. Tags-only projects (for
  example torvalds/linux) now check successfully instead of
  erroring on every pass.
- QUALITY.md brought up to date with six releases of evidence (it
  still described the foundation slice).

## 0.6.0 — 2026-11-11

- The volume page checks its own repo when opened (one free Atom
  call) and re-renders with the fresh tag, so its badges, Download
  button, and asset list are never behind its own release history.
- Fork counts are real: metadata refresh and account imports record
  them, and the volume page shows ⑂ with the number instead of a dash.
- Release history rows with notes get a Notes button that expands the
  release's notes, read from the Atom feed's content — no API cost.
- Catalogue Select mode: tick volumes in any layout, then Add to
  shelf, Check selected, or Remove selected (armed) from the bulk bar.
- `import-org` (CLI) and an Organization account type (window) import
  an organization's projects with the same filters as user imports,
  minus ownership (an org listing spans many owners).
- Every shelf has an Update button (window) and the CLI gains
  `shelf update <name>`: members are checked in parallel, then the
  outdated ones download.
- Acquisitions shows live transfer speed and estimated time left for
  running downloads.
- The Librarian screen backs the catalogue up to
  `repostacks-catalogue-export.json` in the downloads folder and can
  restore it (armed; the replaced catalogue stays on disk as a
  backup file).
- Closing the window parks RepoStacks in the system tray: the
  Librarian keeps its schedule, the tray tooltip and window title
  carry the waiting-updates count, and the tray menu opens the
  window, checks for updates, or quits.
- An installed copy whose program folder is not writable now keeps
  its library in %LOCALAPPDATA%\RepoStacks\data instead of failing
  to save; portable copies keep `data\` beside the exe as before.

## 0.5.0 — 2026-11-11
- Asset rules suggest themselves: a volume page with no saved rules now
  fills the Asset rules box from the release's filename scheme — each
  program file's name with its version generalized to `*` (so
  `App-v0.2.0-x64-Portable.zip` becomes `App-*-x64-Portable.zip` and
  keeps matching future releases), auto-pick first, checksum and
  signature files excluded. A "Suggest from assets" button refills the
  box at any time. Nothing is stored until Save rules is clicked. The
  CLI `assets` command prints the same suggestions, ready to paste into
  `rules`.

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
