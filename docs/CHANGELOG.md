# Changelog

## 0.1.0 — Unreleased
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
