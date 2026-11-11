# Install

Download the latest release from GitHub Releases. Two files, x64 only:

- `RepoStacks-v{VERSION}-x64-Setup.exe` — per-user installer. Installs to
  `%LOCALAPPDATA%\RepoStacks`, adds Start-menu shortcuts, and registers an
  uninstaller. Your `data` folder (catalogue, settings, downloads) is left in
  place when you uninstall.
- `RepoStacks-v{VERSION}-x64-Portable.zip` — unzip anywhere and run.
  Contains `RepoStacks.exe` (the window), `repostacks-cli.exe` (the command
  line), and `LICENSE`.

Verify your download against `SHA256SUMS.txt`, published with the release.

The binaries are unsigned (no code-signing certificate), so Windows
SmartScreen may warn on first run — choose "More info" then "Run anyway" if
you trust the source.

From source:

```text
cargo build --release
```

This builds the CLI (binary name `repostacks` in the source tree; releases
ship it as `repostacks-cli.exe` so it can sit beside `RepoStacks.exe` —
the two names differ only by letter case, which Windows treats as one
filename). The window builds with
`cargo build --release --features custom-protocol --manifest-path window/Cargo.toml`.
