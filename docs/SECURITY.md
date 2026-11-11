# Security

- RepoStacks works with public GitHub data and needs no account.
- An optional personal access token, when the feature lands, is stored only
  in Windows DPAPI-protected storage — never in the catalogue file, config
  text, or logs.
- Downloads come from the release asset URLs GitHub publishes for a repo the
  user added. Checksum assets are verified when a release provides them.
- Nothing is executed after a download by default. Running a downloaded file
  or a user-defined command requires a per-repo explicit opt-in.
- No telemetry, no cloud sync, no selling data. The only network traffic is
  to github.com / api.github.com at the intervals the cache policy allows.
