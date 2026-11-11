# Contributing

- Read `AGENTS.md` (project rules) before changing anything.
- Every capability lands as a core operation first, then a surface (CLI/window).
- Run the full gate before proposing a change: `cargo test` (all targets)
  plus a Windows-target release link.
- Keep dependencies at zero unless the change explains why a new one is
  needed, its license, and how the project would exit it.
