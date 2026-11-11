# QUALITY.md — RepoStacks

Scored from evidence only. Any Fail = not done.

| Dimension | Score (1-5) | Evidence |
|---|---|---|
| Purpose fit | 4 | Core operations map 1:1 to the blueprint's Phase 1 (catalogue, shelves, filters, feed parsing, asset rules); window and download engine are later slices by design. |
| Correctness | 4 | Cache policy encodes GitHub's documented conditional-request rule with a regression test for the anonymous-304 case. |
| Reliability | 4 | Catalogue writes are atomic; malformed feed entries are skipped, never fatal; schema_version is checked on load. |
| Security | 4 | No credentials in the data file by design; token storage rule (DPAPI) recorded before any token feature exists. Post-download execution is opt-in only. |
| Performance | 3 | Release checks cost zero REST calls; metadata cost is policy-bounded. No measurements yet — Unverified. |
| Accessibility / UX | 2 | CLI only so far; window UX is gated on approved mockups. |
| Documentation | 4 | README, architecture with the corrected rate-limit design, parity matrix, standard docs set. |

Overall: foundation passes the gate for a first slice; window, network, and
download dimensions are explicitly Unverified, not scored as done.
