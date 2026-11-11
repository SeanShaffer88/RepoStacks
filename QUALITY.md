# QUALITY.md — RepoStacks

Scored from evidence only. Any Fail = not done.

| Dimension | Score (1-5) | Evidence |
|---|---|---|
| Purpose fit | 5 | Every blueprint Phase-1 capability is shipped (catalogue, shelves, faceted search, Atom watcher, download queue, standing orders), plus user/org import, bulk actions, and a tray-resident Librarian through v0.6.0. |
| Correctness | 4 | Cache policy encodes GitHub's documented conditional-request rule with a regression test for the anonymous-304 case; the Atom-404 fallback chain (tags feed, then one reported REST call) is unit-tested and live-verified against a tags-only repo (torvalds/linux). |
| Reliability | 4 | State writes are atomic with fsync + `.bak` recovery and quarantine; malformed feed entries are skipped, never fatal; schema_version checked on load; the Librarian scheduler survives a panicking pass; jobs caught mid-download re-queue and resume; a named-mutex guard keeps a second instance from racing the data folder, and downloads run on a self-recovering background worker instead of under the state lock (v0.8.0, runtime Unverified). |
| Security | 4 | Token is DPAPI-protected in the window / env-only in the CLI, never in the catalogue JSON, never logged; download URLs are restricted to GitHub HTTPS at plan and execution time; asset names and tags are path-validated; post-download execution does not exist (downloads only). DPAPI round-trip still field-Unverified. |
| Performance | 3 | Release checks cost zero REST calls (fallback excepted, and reported); checks and metadata refresh run in parallel bounded by the concurrency setting. A persistent usage ledger now accounts for every counted REST call by purpose (v0.9.0), so spend is auditable after the fact, not only per run. No formal measurements yet — Unverified. |
| Accessibility / UX | 4 | Field-verified on a real Windows machine across six releases; keyboard-openable entries, Add-modal focus trap, ARIA labelling, arm-to-confirm destructive actions. The v0.6.0 tray and bulk UI await their field pass. |
| Documentation | 4 | README, architecture with the corrected rate-limit design, parity matrix kept current per slice, standard docs set. README screenshots predate v0.5.0 — fresh captures owed from the owner's machine. |

Overall: passes the gate. Remaining Unverified items are named, not
hidden: installer run, DPAPI round-trip, window downloads, Librarian
firing on schedule, and the v0.6.0 additions on a real machine.
