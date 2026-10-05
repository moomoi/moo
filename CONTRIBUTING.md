# Contributing to Moo

Building from a clone: [docs/building.md](docs/building.md). Releasing: [docs/release.md](docs/release.md).

## Conventional commits and releases

Every commit, and every PR title (PRs squash-merge, so the title becomes the commit), is a
[Conventional Commit](https://www.conventionalcommits.org): `type(scope): imperative summary`,
lowercase. Releases are automated from them. [`tishlang/sem`](https://github.com/tishlang/sem)
reads the commits since the last release on every `main` push, so the commit type is the release
decision:

| Type | Effect |
|------|--------|
| `feat:` | minor release |
| `fix:`, `perf:` | patch release |
| `feat!:`, or a `BREAKING CHANGE:` footer | major release |
| `chore:`, `docs:`, `ci:`, `refactor:`, `test:`, `style:`, `build:` | no release |

- **Scope with the part you touched:** `feat(slack):`, `fix(ai):`, `feat(plugins):`, `fix(files):`,
  `fix(clipboard):`, `feat(web):`, `ci(release):`. Bare types are fine for repo-wide changes.
- **One coherent change per commit.** Squash fix-ups before pushing.
- **Bodies explain why.** Reference issues and PRs where they exist.
- **No AI or tool attribution:** no `Co-Authored-By` lines naming an assistant, no "Generated
  with" footers.
- A push of only `ci:`, `docs:` or `chore:` commits releases nothing on purpose. If users should
  get the change, it is a `fix:` or `feat:`.

Every PR gets a **Release check** notice saying which version its merge would release, if any.

Examples:

```
feat(slack): send a message to a channel or person
fix(ai): resume the question after signing in to Hypery
perf(files): skip the index rebuild when nothing changed
feat(plugins)!: commands declare arguments in moo.json

BREAKING CHANGE: the `args` manifest field is now `arguments`.
```

## Before opening a PR

```sh
bash plugins/build.sh
(cd packages/moo-macos && cargo test --lib)
bash app/build.sh
```

CI runs the same three steps on macOS 26 (`.github/workflows/ci.yml`).
