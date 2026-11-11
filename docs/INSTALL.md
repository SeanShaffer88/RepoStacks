# Install

No installer exists yet — the first release has not been cut. From source:

```text
cargo build --release
```

The binary is `repostacks` (CLI). When the window slice ships, releases will
provide `RepoStacks-v{VERSION}-x64-Setup.exe` and
`RepoStacks-v{VERSION}-x64-Portable.zip` with `SHA256SUMS.txt`, x64 only,
unsigned (no code-signing certificate — installers say so honestly).
