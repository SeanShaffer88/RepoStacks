# Performance — measured evidence

RepoStacks ships a built-in benchmark, `repostacks bench`, so the
performance claims in QUALITY.md rest on measurements anyone can
re-run, not on impressions. The bench times the core operations over a
synthetic catalogue of a stated size (default 5,000 volumes), using the
same code paths the app itself uses:

- catalogue save / load, through the real persistence layer;
- the rich search (one plain-text query, one field-filtered query);
- the facet counts behind the filter panel;
- the Atom feed parser over a 30-entry feed;
- the parallel release-check pipeline with the network replaced by a
  stub fetcher, at concurrency 1 and at concurrency 8 — this measures
  RepoStacks' own work per check (thread handling, parsing, comparing),
  never network speed.

The synthetic catalogue and feed are generated from a fixed seed, so
runs are comparable over time. `repostacks bench --json` prints the
same results in machine-readable form; `--repos N` changes the
catalogue size.

## Recorded run — 2026-11-11 (v0.10.0)

Machine: shared cloud VM, 4 visible vCPUs (AMD EPYC 9D25), Linux.
Build: release profile. Catalogue: 5,000 volumes, 2,703 KB on disk.
Figures are medians of the stated repetitions; a second run the same
day landed within noise of every figure except the check pipeline,
which is the most sensitive to shared-CPU load.

| Scenario | Repetitions | Median |
|---|---|---|
| CLI startup (print version and exit) | 5 | 1.4 ms |
| Catalogue save (5,000 volumes) | 5 | 45.6 ms |
| Catalogue load (5,000 volumes) | 5 | 57.0 ms |
| Search, plain text ("ledger", 500 hits) | 50 | 3.5 ms |
| Search, filtered ("lang:rust stars:>500", 614 hits) | 50 | 0.16 ms |
| Facet counts (language, topic, license, owner) | 50 | 2.0 ms |
| Feed parse (30 entries) | 200 | 0.16 ms |
| Release check, concurrency 1 (300 repos) | 3 | 249 µs per repo |
| Release check, concurrency 8 (300 repos) | 3 | 107 µs per repo |

The bench process peaked at 29 MB of memory across the whole run
(5,000-volume catalogue held in memory, every scenario included).

## What is not measured here

The window's startup time and idle memory on a real Windows machine
are not covered by this record — the bench measures the core library
and the CLI. Those figures join this file when they are captured on
Windows; until then QUALITY.md keeps them listed as Unverified.
