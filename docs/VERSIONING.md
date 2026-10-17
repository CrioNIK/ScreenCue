# Versioning policy

ScreenCue is an early prototype. Keep public versions in the `0.0.x` series until a deliberate milestone justifies changing the middle number.

- New prototype iterations increment the last number: `0.0.1`, `0.0.2`, `0.0.3`.
- Small follow-up revisions append lowercase Latin letters: `0.0.3a`, `0.0.3b`, and so on. A numeric patch is not required for every small fix.
- Use the same public label in release titles, tags (with a `v` prefix), and user-facing documentation.
- Where a package format requires Semantic Versioning, encode a letter revision with a hyphen, for example public `0.0.3f` corresponds to package version `0.0.3-f`. Do not put an invalid numeric version into Cargo or a client manifest.
- Keep native package and client integration versions synchronized when publishing.

These prototype labels do not establish Semantic Versioning upgrade order. A letter revision encoded as a prerelease sorts below its bare numeric base. Select the explicitly designated latest release rather than inferring recency from a SemVer comparison; ScreenCue currently has no automatic updater.

This policy starts with `0.0.3`. Earlier published `0.1.0` and `0.2.0` tags remain historical records; the numbering change does not imply a feature rollback. Do not rewrite existing release tags or replace their artifacts to renumber them.

Commit messages, release notes, documentation, and other public project text are written in English.
