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

## Planned
- Live Atom polling on a bounded interval with graceful REST fallback.
- REST metadata fetch under the cache policy; optional DPAPI-stored token.
- Download engine: concurrency limit, HTTP Range resume, checksum verification.
- Tauri window: catalogue grid, filter panel, shelves, acquisitions queue, repo detail.
- Standing orders: auto-download per repo, Update All per shelf.
- Release: Setup.exe + Portable.zip + SHA256SUMS.txt (x64).
