---
name: release
description: Cut a vrl-tools release — bump the version, date the changelog, build and package the .vsix, tag and push so the Release workflow publishes it. Use when asked to "sacar versión", "release", "publicar", "subir una versión" or to tag the extension.
---

# Cutting a vrl-tools release

The repository releases by pushing a `v*` tag. `.github/workflows/release.yml`
then builds, tests, packages, checks the tag against the manifest, and creates
the GitHub release with the `.vsix` attached. **It stops there on purpose** —
see "The Marketplace" below.

Nothing here is automatic. Do each step, in order, and stop at the first one
that fails rather than working around it.

## Before anything

Everything must already be green on `main`:

```
cargo test --workspace
cargo clippy --workspace --all-targets     # zero warnings, not "few"
npm test                                   # six suites
```

If the working tree is dirty with work that is not part of the release, stop
and ask. A release commit should contain the version bump, the changelog date,
and nothing else.

## 1. Pick the number

Semver against what is in the changelog's `## Unreleased`:

- new behaviour, new settings, new views → **minor**
- corrections only, no new surface → **patch**

Say which you picked and why before doing it. The user has release plans that
live in memory and in `docs/PLAN.md`; a number may already be spoken for, and
moving it is their call, not yours.

## 2. Bump and date

Two files, nothing else:

- `editors/vscode/package.json` → `"version": "X.Y.Z"`
- `editors/vscode/CHANGELOG.md` → `## Unreleased` becomes `## X.Y.Z — YYYY-MM-DD`

The root `package.json` version is not the extension's and does not move.

## 3. Package locally first

```
npm run package
```

This is `build && test && package`: it regenerates the grammar and the licence
notices, rebuilds the wasm, compiles, runs every suite, and writes
`editors/vscode/vrl-tools-X.Y.Z.vsix`. It is what CI is about to do, so a
failure here is a failure there, five minutes sooner.

**Read the file list it prints.** `vsce` packs the working directory, not the
git index, so anything gitignored but present on disk ships unless
`.vscodeignore` also excludes it. That is how Git Bash crash dumps
(`*.stackdump`) ended up inside the 0.6.x packages. If something unexpected is
in the list, add it to `.vscodeignore` and repackage before going on.

If `npm run gen:notices` changed `THIRD-PARTY-NOTICES.md`, read the diff before
committing it: a new licence means a new crate arrived in what ships. Never
widen `accepted` in `about.toml` to make it pass.

## 4. Commit, tag, push

```
git commit -m "Release X.Y.Z" ...
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin main
git push origin vX.Y.Z
```

The tag must be `v` plus exactly the manifest version — the workflow compares
them and fails the release otherwise, which is deliberate: a tag that disagrees
with the manifest produces a release nobody asked for.

Push `main` before the tag, so the release is built from a commit that exists
on the branch.

## 5. Watch it

```
gh run list --limit 3
gh run watch <id> --exit-status
gh release view vX.Y.Z --json assets
```

The Release job takes about seven minutes, most of it the wasm. Confirm the
`.vsix` is attached before telling the user it is out.

## The Marketplace

**The `.vsix` is uploaded by hand**, from
<https://marketplace.visualstudio.com/manage>. Tell the user this is the one
step left to them; do not offer to automate it.

This is a settled decision, not an omission. `vsce publish` needs an Azure
DevOps PAT scoped to "All accessible organizations" — a global PAT — and global
PATs are retired on 1 December 2026. The replacement is Entra ID with workload
identity federation: a service connection, a federated credential, and an
identity enrolled in the publisher. Adding a `VSCE_PAT` secret would work for a
few weeks and then fail at the worst moment. If releases ever get frequent
enough to be worth automating, build the federation.

## Afterwards

The user installs with:

```
code --install-extension editors/vscode/vrl-tools-X.Y.Z.vsix
```

Check whether the release invalidates anything in memory — a planned version
number that just got used is the usual one — and update it rather than leaving
it to be read as true next session.
