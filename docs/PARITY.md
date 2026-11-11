# PARITY.md — capability matrix (evidence only)

| Capability | Core op | CLI | Window | Evidence |
|---|---|---|---|---|
| Add/remove repo (owner/repo or URL) | yes | yes | — | repo_ref + catalog tests |
| Shelves (create by use, multi-shelf) | yes | yes | — | catalog tests |
| Faceted filter + facet counts | yes | yes | — | catalog tests |
| Atom feed parse (releases/tags) | yes | yes (saved file) | — | atom tests |
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
