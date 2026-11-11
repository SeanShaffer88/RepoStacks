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
| Live feed fetching | no | no | — | network slice, not started |
| Download engine (Range, resume, checksum) | no | no | — | download slice, not started |
| Token storage (DPAPI) | no | no | — | lands with token setting |
| Window (Tauri) | no | no | no | gated on approved mockups |
| Installer + portable release | no | no | no | first release slice |
