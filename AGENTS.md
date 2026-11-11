# RepoStacks — Project Rules

Prompt in force: The Elite Software Master Prompt v1.1 · Annexes: none
Purpose in one line: A Windows library app for GitHub repositories — catalogue, shelves, quota-free release watching, download queue.

## Project Manifest
- **Tier:** t3 — rich-render catalogue app (Tauri 2.x sanctioned alternate: system WebView2, bundled local frontend, no remote content)
- **Rationale:** the product surface is a browsable card catalogue; the scheduler/queue are operations in the core library, driven identically from CLI and window.
- **Stack:** Rust core + CLI now (zero dependencies); Tauri shell lands after the design mockups are approved. New external dependencies each need the owner's explicit approval before introduction.
- **Release shape:** `RepoStacks-v{VERSION}-x64-Setup.exe` + `RepoStacks-v{VERSION}-x64-Portable.zip` + `SHA256SUMS.txt`. No MSI, no winget, x64 only, binaries unsigned.

Frozen foundation: `src/` single crate — `repostacks_core` library (all operations) + `repostacks` CLI (thin surface). Modules: repo_ref, atom, cache_policy, catalog, glob_match, queue, json.
Build: `cargo build` · Full gate: `cargo test` (all targets) + a linked Windows binary. Local Windows link route on the dev VM (no MSVC): `RUSTFLAGS="-C linker=rust-lld -L $HOME/llvm-mingw/x86_64-w64-mingw32/lib -C link-arg=$HOME/llvm-mingw/x86_64-w64-mingw32/lib/crt2.o" cargo build --release --target x86_64-pc-windows-gnullvm` — verify the .exe is fresh (size + mtime) and `file` reports PE32+. Test corpus: unit tests in each module, incl. the cache-policy regression guard.
Budgets: metadata REST spend = 0 for release checks (Atom only); 1 REST call per actual new release (assets); anonymous metadata refresh >= 24h.
Coverage floors: 85% core / 70% overall / 100% safety paths (Elite defaults).
Special rules for this project:
- The blueprint's ETag claim was wrong for anonymous callers — never reintroduce "304s are free without a token" (see docs/ARCHITECTURE.md and cache_policy tests).
- A pasted personal access token is stored with DPAPI only, never in the catalogue JSON, never logged.
- Post-download actions are download-only by default; running anything needs a per-repo explicit opt-in.
- Atom feeds are undocumented GitHub behaviour — the watcher must degrade gracefully (REST fallback within budget) if a feed 404s.
Known limitations live at: docs/PARITY.md · Decision log: docs/ARCHITECTURE.md · QUALITY.md: QUALITY.md
