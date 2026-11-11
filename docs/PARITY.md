# PARITY.md — capability matrix (evidence only)

| Capability | Core op | CLI | Window | Evidence |
|---|---|---|---|---|
| Add/remove repo (owner/repo or URL) | yes | yes | — | repo_ref + catalog tests |
| Shelves (create by use, multi-shelf) | yes | yes | — | catalog tests |
| Faceted filter + facet counts | yes | yes | — | catalog tests |
| Atom feed parse (releases/tags) | yes | yes (saved file) | — | atom tests |
| Feed-missing fallback (tags feed → 1 REST call) | yes | yes | yes | net fallback tests + live run (torvalds/linux → v7.4-rc2) |
| Corrected rate-limit policy | yes | — | — | cache_policy tests |
| Asset glob rules | yes | yes | — | glob_match tests |
| Download queue state machine | yes | — | — | queue tests |
| Live feed fetching (Atom) | yes | yes | — | net tests + live CLI run 2026-11-10 |
| Metadata refresh under cache policy | yes | yes | — | net tests + live CLI run 2026-11-10 |
| Rich search (fields, ranges, status, sort) | yes | yes | — | search tests |
| Saved searches | yes | yes | — | view settings tests + CLI smoke |
| Layouts / themes / density / grouping / columns | yes | yes | — | view tests (render per layout) |
| Bulk import with preview | yes | yes | — | catalog import tests + CLI smoke |
| Download engine (Range, resume, checksum) | yes | yes | — | download tests + live verified download 2026-11-10 |
| Release asset listing + per-volume rules | yes | yes | — | net/download tests + live run |
| Queue persistence | yes | yes | — | queue round-trip tests |
| Standing orders (flag) | yes | yes | — | catalog field + CLI |
| Token storage (DPAPI) | no | no | yes | window dpapi module; runtime unverified |
| Window (Tauri) | no | no | yes | window crate; CI build; runtime unverified |
| Installer + portable release | yes | — | yes | v0.1.0 release, hash-verified 2026-11-10 |
| Corruption-proof state files (backup + quarantine recovery) | yes | yes | yes | persist tests |
| Parallel release checks | yes | yes | yes | net parallel tests + live CLI run |
| Failed-job retry | yes | yes | yes | queue tests |
| Download URL trust + size verification | yes | yes | yes | download tests |
| Explainable auto-pick (score + reason) | yes | yes | yes | download tests + live `assets` run |
| JSON output (`--json`) | yes | yes | — | CLI smoke 2026-11-10 |
| `doctor` health check | yes | yes | — | CLI smoke 2026-11-10 |
| Settings validation | yes | yes | yes | view tests |
| Librarian metadata refresh (quota-governed) | yes | — | yes | window librarian pass |
| Import a user's projects with filters | yes | yes | yes | user_import tests + live CLI run (312-repo listing, paginated) |
| Search negation + is:due/is:never | yes | yes | yes (find box) | search tests with fixed clock |
| Export catalogue (list / JSON) | yes | yes | — (CLI) | live CLI run |
| Clean stale downloads | yes | yes | — (CLI) | download tests (temp tree) + live CLI run |
| Parallel metadata refresh | yes | yes | yes | net helper shared by all three |
| Open on GitHub / download folder | — | — | yes | window commands (validated RepoRef / own layout) |
| Check on start (setting) | yes (settings) | — | yes | Settings round-trip |
| Saved searches in the window | yes | yes | yes | saved_list/save/delete commands |
| Import an organization's projects | yes | yes | yes | user_import tests + live CLI run (tauri-apps: 114 listed) |
| Shelf update (check + download) | yes | yes | yes | live CLI run (downloaded v0.5.0 via a shelf) |
| Bulk select / shelf / check / remove | — | — | yes | window bulk commands |
| Catalogue backup export/import | yes | yes (export --json) | yes | window commands (downloads-folder file) |
| Release notes from the feed | yes | — | yes | atom body parse tests |
| Fork counts | yes | yes (--json) | yes | metadata + import parse tests |
| Download speed / ETA | — | — | yes | window queue sampling |
| Tray presence + updates count | — | — | yes | window tray; runtime unverified |
| Data-home fallback (LOCALAPPDATA) | — | — | yes | window choose_data_dir probe |
