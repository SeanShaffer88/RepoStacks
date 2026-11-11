# Compatibility

- Target: Windows 10 and Windows 11, x64. (Windows runtime behaviour is
  Unverified until the window slice runs on a real Windows machine.)
- The CLI core is portable Rust and is also exercised on Linux in CI style
  local gates, but releases ship for Windows only.
- Data file: JSON catalogue, `schema_version` 1, upward migrations will be
  explicit and tested per the engineering contract.
