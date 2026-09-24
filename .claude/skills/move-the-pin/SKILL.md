---
name: move-the-pin
description: Move vrl-tools onto a newer vrl crate and Vector release — pick a pinnable pair, update the pin, regenerate what derives from it and re-check the claims about Vector. Use when asked to update vrl, bump Vector, support a newer Vector release, or when a new Vector version is out.
---

# Moving the vrl / Vector pin

The whole extension is "what the pinned compiler says", so this changes what
every diagnostic, completion and graph finding means. It is the highest-risk
routine change in the repository. Go slowly and stop when something does not
match; a half-moved pin checks programs against one release's standard library
and another release's Vector functions.

## The one thing to get right first

**A Vector release number is not a `vrl` crate version, and not every Vector
release can be pinned to.** Some depend on `vrl` from git `branch = main` and
have no crate version at all: 0.50, 0.51 and 0.57.0 are like that. The `vrl`
crate is on 0.x and has never published a 0.58.0.

So do not pick a number. Find a pair:

1. Open Vector's `Cargo.lock` at the tag you are considering —
   `https://github.com/vectordotdev/vector/blob/v<RELEASE>/Cargo.lock` — and
   find the `[[package]] name = "vrl"` entry.
2. If it has a plain `version` and no `source = "git+..."`, the pair is
   pinnable. If it comes from git, that release is not a candidate; try the
   next one.
3. Confirm the crate version exists on crates.io.

The map as of Vector 0.58.0 is in `CLAUDE.md`. Add the new pair to it.

## What to change

Four places, and a test that checks three of them agree:

1. `Cargo.toml` (workspace) — `vrl = { version = "=X.Y.Z", ... }`
2. `Cargo.toml` — `tag = "v<RELEASE>"` on **both** `vector-vrl-functions` and
   `enrichment`. The tag moves with the pin.
3. `crates/vrl-check-core/src/lib.rs` — `VRL_VERSION` and `VECTOR_RELEASE`.
   These are the single source of the pair; the extension asks the wasm module
   for them rather than keeping its own copy.
4. `CLAUDE.md` — the release-to-crate map.

`cargo test -p vrl-check-core version_matches_the_pin` asserts the manifest and
the constants agree. It will not tell you the pair is real — only Vector's
`Cargo.lock` does that.

## Then regenerate, do not hand-edit

```
cargo test --workspace          # the stdlib dump is asserted against the crate
npm run gen:grammar             # walks vrl::stdlib::all() via the vrl-stdlib bin
npm run gen:notices             # the crate graph that actually ships changed
npm test                        # six suites; test:diagnostics compiles the corpus
```

Read the diffs:

- **`gen:grammar`** — functions added or removed between the two versions show
  up here. Added is normal. **Removed is a decision**: anything the repo shows
  as valid VRL has to be accepted by the pinned compiler, so a removed function
  may mean corpus files or snippets to change.
- **`gen:notices`** — a new licence section means a new crate arrived in what
  ships. Read what it is. Never widen `accepted` in `about.toml` to make the
  build pass.
- **`test:diagnostics`** failures are the pin telling you the language moved.
  Fix the corpus to the new compiler, never the other way round.

## Re-check the claims about Vector

Moving the pin moves the Vector checkout too, and the repository makes claims
about Vector's source that go stale with it:

```
cargo test -p vector-topology --test against_vector
```

That re-reads every output port name and config flag from
`CARGO_HOME/git/checkouts/vector-*/<commit>/` — the checkout the git
dependency leaves behind, identified by the version in its `Cargo.toml`. If it
fails, a port was renamed or a flag removed, and
`crates/vector-topology/src/outputs.rs` needs the new reading from `fn outputs`
in the file the failure names.

It skips silently when there is no checkout, so run a build first.

Also re-read, by hand, in that checkout:

- `src/config/validation.rs` — `check_shape`, `check_names`, `warnings`: the
  graph's findings are these, and Vector adds to them.
- `src/config/compiler.rs` — `expand_globs`, for how an input pattern resolves.
- Any component whose outputs the table claims to know.

## Before using any `vrl` API

Verify the signature on docs.rs **for the newly pinned version**. The crate
surface still changes between 0.x releases, and a signature that compiled
before is not evidence.

## Finally

The status bar surfaces the pair, so the user sees it change. Say in the
changelog which Vector release the extension now matches — that is the thing
someone running Vector in production needs from the entry.
