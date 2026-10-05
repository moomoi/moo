# Releasing Moo

Versions come from conventional commits ([CONTRIBUTING.md](../CONTRIBUTING.md)) through
[`tishlang/sem`](https://github.com/tishlang/sem). Merging to `main` builds a signed, notarized,
universal Moo and publishes it as a prerelease in this private repo. Promoting the prerelease
publishes it to everyone. Same pattern as dune-ide and popcraft-desktop.

| Repo | Visibility | Holds | Gets |
|------|-----------|-------|------|
| `knoeone/moo` | private | source, CI, all workflows | a prerelease for every release-worthy push to `main` |
| `moomoi/moo` | public | README, LICENSE and release downloads, no source | full releases (notes since the last public one) |
| `moomoi/homebrew-moo` | public | the Homebrew tap (`Casks/moo.rb`) | regenerated on every public release |

```
PR               ci.yml       build + unit tests; "Release check" says what the merge releases
merge to main    release.yml  version (sem dry run) -> build-mac -> release
                              build-mac: arm64 + x86_64 builds, lipo, Developer ID + hardened
                              runtime, notarize + staple the app, DMG + zip, notarize + staple
                              the DMG, verify. release: sem tags vX.Y.Z, publishes the prerelease
promote          publish-public.yml  copy to moomoi/moo, then repository_dispatch to the tap
                 homebrew-moo update-formulas.yml  rewrite Casks/moo.rb with the new sha256
```

Assets, under fixed names so `releases/latest/download/…` links never change:

- `Moo-macos-universal.dmg`: Moo.app and an Applications link. The DMG and the app are both stapled.
- `Moo-macos-universal.zip`: the stapled app.
- `SHA256SUMS`

`https://moo.moi/download` redirects to the latest DMG on `moomoi/moo`.

## One-time setup

1. **Signing secrets on `knoeone/moo`.** Run `bash scripts/setup-apple-signing.sh`. It uploads
   `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_TEAM_ID`,
   `APPLE_ID` and `APPLE_PASSWORD` (an app-specific password). The Developer ID is Knoeone LLC's,
   the same identity as popcraft-desktop and dune.
2. **Release GitHub App.** The private repo's `GITHUB_TOKEN` can't write to `moomoi/*`.
   1. At <https://github.com/organizations/moomoi/settings/apps>, choose **New GitHub App**. Any
      name ("Moo Release"), no webhook, **Repository → Contents: Read and write**.
   2. Note the **App ID** and generate a **private key** (`.pem`).
   3. **Install App** → `moomoi` → only `moomoi/moo` and `moomoi/homebrew-moo`.
   4. Set the secrets:

      ```sh
      gh secret set MOO_RELEASE_APP_ID --repo knoeone/moo --body "<app id>"
      gh secret set MOO_RELEASE_APP_PRIVATE_KEY --repo knoeone/moo < moo-release.private-key.pem
      ```
3. **Public repos.** `moomoi/moo` and `moomoi/homebrew-moo` must be public, with a first commit on
   `main`: the README and LICENSE in one, the tap workflow in the other.
4. **Merges.** On `knoeone/moo`, allow squash merging only, and require the `Build and test` and
   `Release check` checks on `main`.
5. **First release.** There are no tags yet, and the history before CI isn't conventional, so sem
   would propose 1.0.0. Run **Release** from the Actions tab (workflow_dispatch) with `force` set
   to `0.1.0`. From then on every `feat`, `fix` or `perf` merge releases on its own.

## Every release

1. **Merge** a PR whose title is `feat:`, `fix:` or `perf:` (or breaking). The PR's Release check
   already said which version that makes.
2. **Wait for Release.** Its summary links the prerelease `vX.Y.Z` on `knoeone/moo`. It takes about
   20 minutes warm; notarization is usually 2 to 10 of them.
3. **Test the prerelease DMG.** `bash scripts/verify-release.sh ~/Downloads/Moo-macos-universal.dmg`
   checks the signature, entitlements, both architectures and the notarization tickets.
4. **Promote:** Releases → vX.Y.Z → Edit → uncheck **Set as a pre-release** → Update release.
   `publish-public.yml` mirrors it to `moomoi/moo` and bumps the cask.
5. **Check:** `brew update && brew upgrade --cask moomoi/moo/moo`, and `https://moo.moi/download`.

## Re-running

- **Rebuild a prerelease:** re-run the failed Release jobs. sem replaces same-named assets.
- **Re-mirror a public release:** run **Publish public release** with the tag (`v0.1.0`).
- **Re-bump Homebrew:** run **Update formulas** in `moomoi/homebrew-moo` with the version.
- **Release without a feat or fix commit:** run **Release** with `force` set to `patch`, `minor`,
  `major` or an exact version.

## Building a release locally

```sh
bash scripts/build-universal.sh                 # dist/universal/{moo,plugins/}
VERSION=0.1.0 bash scripts/package-release.sh   # dist/Moo.app, dist/release/*
```

Without `SIGN_IDENTITY` the app is signed ad hoc. Without `APPLE_ID`, `APPLE_PASSWORD` and
`APPLE_TEAM_ID`, notarization is skipped with a warning. `verify-release.sh` still checks the rest.

## What CI builds with

- **Runner:** `macos-26` (Swift 6.3, macOS 26 SDK), stable Rust with both Mac targets.
- **Compiler:** `scripts/ci-toolchain.sh` clones `tishlang/tish` at `TISH_REF` (`toolchain.env`,
  tish 3.12.2), applies `TISH_PATCHES`, and builds it at `../../tish/tish-nimble` from the repo,
  the path `packages/moo-macos/Cargo.toml` names. It vendors `tishlang/tish-apple` at `APPLE_REF`
  against that copy. It refuses to touch a directory it didn't create, so running it on a dev
  machine can't reset a real checkout.
- **Patches:**
  - `tish-embedder-plugins.patch` is the `~/Projects/tish/tish-nimble` working tree. Regenerate it
    whenever that tree changes, or CI builds with stale compiler changes.
  - `tish-native-target.patch` adds `TISH_NATIVE_CARGO_TARGET`, which builds a desktop binary or
    plugin for a named triple and not for the build machine's CPU.

  Both should go upstream into `tishlang/tish`. Then `TISH_REF` becomes a release and the patches
  go away.
- **Universal:** `scripts/build-universal.sh` builds each native artifact (the app and Tier B
  `.lib` plugins) once per triple and combines them with `lipo`. Bytecode plugins (`.tishc`) are
  the same on both. On Intel, Apple Intelligence reports unavailable and Ask AI offers another
  model.

## Bundle identity

Release builds are `moi.moo.launcher`, signed with the Developer ID. Local dev builds
(`app/build.sh`) keep signing as `dev.moo.launcher` with an Apple Development certificate. The
two are different apps to macOS: each has its own Accessibility and Contacts grants and its own
Keychain access. Keys saved by a dev build ask for the login password once in a release build,
or can be entered again.
