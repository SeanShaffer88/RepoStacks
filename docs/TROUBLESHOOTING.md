# Troubleshooting

- **`expected owner/repo`** — pass a repository as `owner/repo`, or paste its
  full GitHub URL; profile URLs and search URLs are not repositories.
- **Catalogue fails to load with a schema message** — the data file was made
  by a newer RepoStacks. Update RepoStacks; your volumes are not lost.
- **A feed check shows no releases** — the project may publish tags without
  GitHub Releases; try the tags feed, or the repo may genuinely have none.
- **Rate limit reached (anonymous)** — release checks use Atom feeds and do
  not spend API quota; metadata refresh is the bounded spender. Adding a
  token raises the limit from 60 to 5,000 requests per hour.
