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
| Download engine (Range, resume, checksum) | no | no | — | download slice, not started |
| Token storage (DPAPI) | no | no | — | lands with token setting |
| Window (Tauri) | no | no | no | dark ledger approved; owner's screen concepts on file |
| Installer + portable release | no | no | no | first release slice |
