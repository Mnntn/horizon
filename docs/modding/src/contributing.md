# Where each doc lives

The book follows [Diátaxis](https://diataxis.fr): a page is one of four kinds, and does only that job.

| Kind | Job | Lives in |
|---|---|---|
| Tutorial | Take a newcomer to a working result | Start |
| Explanation | Say how the pieces fit, with no steps | Concepts |
| How-to | One task, steps only | How-to, Injections |
| Reference | Facts, complete and dry | Reference, and the rustdoc of `horizon-mod-api` |

- **A hook, event or `Host` method:** its `///` lines in the declaration under `crates/horizon-mod-vocab/src/hooks/`.
  The generator writes it as `///` on the generated item; `cargo doc -p horizon-mod-api` shows it. A hook without
  a `doc` fails the build.
- **A hook whose game target has many parameters:** a `native_*` target takes at most 7 parameters (receiver included;
  `MAX_TARGET_PARAMETERS` in `crates/horizon-mod-api-gen/src/dispatch.rs`). A hook with more declares its parameters
  on the hook (`fn update_shape(dimension: LevelDimension, ...);`, as written in `hooks/*.rs`) and the target takes them
  as one generated `<Name>Call` (`native_update_shape(&self, ticks, call: UpdateShapeCall<'_>)`),
  handed to `dispatch_<name>(host, call)`. The generator refuses a flat target over the limit, and a target never carries
  `#[allow(clippy::too_many_arguments)]`. The hooks that were already over the limit are listed in
  `FLAT_TARGETS_OVER_THE_LIMIT` in the same file; that list only shrinks (convert a hook, delete its line) and a new hook
  is never added to it. The carrier is host-internal plain data, never on wasm, and no modder sees it.
- **Hook notes:** keep longer prose in a `#[notes = r#"..."#]` attribute on its hook or helper declaration,
  heading included. General catalog prose lives in `hooks/notes.rs`. `mdbook build docs/modding` renders
  the Hook notes chapter from those attributes without writing a Markdown source page. To preview just
  that chapter, run `cargo run --quiet -p xtask -- mod-api notes`; do not commit its output.
- **A guide page:** `docs/modding/src/`. Code in a page is `{{#include}}`d from `docs/modding/examples`, which
  compiles, so a change to the API that breaks an example fails the build.
- **A mod API change** updates its fragment `doc` and any page it affects in the same PR.
- **A new concept or how-to page:** add the file directly under `concepts/` or `how-to/`; the book adds it
  to its part, after the pages `SUMMARY.md` lists, in order of its `#` title. Don't add it to `SUMMARY.md`
  unless it must come first or sit in another part. Any other new page needs its `SUMMARY.md` line: the
  build fails on a page the book doesn't render, and on a `SUMMARY.md` link to a missing file.
- **Trait-mod and native-mod concepts:** edit one source in `concepts/trait-mods/` or
  `how-to/native-mods/`. Sorted filenames set reading order; the leading `<!-- chapter: Title -->`
  supplies the navigation title. New files are discovered automatically, without a shared list edit.
  The old pages are assembled from these sources to preserve published links and heading anchors.
  Do not append prose to the aggregate pages or remove an existing heading anchor.
  Each `legacy-anchors.txt` freezes the pre-split published anchors; never extend it per new concept.
  Optional `<!-- separator: N -->` metadata preserves original inter-section blank lines in aggregates
  without trailing blank lines in source files; new pages default to one blank line.
  Use relative links (inline or reference-style); root-relative links are refused.
- `src/llms.txt` is the index for agents; keep it in step with `SUMMARY.md`.

Build the book with `mdbook build docs/modding` and the reference with `cargo doc -p horizon-mod-api --no-deps`.

# What a PR needs to show

Code review, CI and your own focused tests are the proof. You don't need to run the game.

A reviewer runs your change in game at your head only when a test can't cover one of these, and says which:

- rendering or UI, what the player sees;
- client-server sync: LAN guests, a dedicated server, packets;
- when or in what order something happens compared with vanilla, such as a hook's firing point.

If your PR changes or removes something mods already use, the reviewer also runs
[horizon-machine-sample](https://github.com/arismoko/horizon-machine-sample) against it. A new item's `///` doc is its
reference.

## A catalog point and the wasm fixture

A new or changed catalog point is proven by the wasm fixture: real wasm mods (`crates/horizon-mod-wasm/tests/fixture`,
`library-fixture` and `groups`) built with `cargo horizon-mod build --wasm` and loaded by the real loader. The
`wasm-mods` CI job does all of it on a PR that touches `crates/cargo-horizon-mod/`, a `crates/horizon-mod-*` crate or
`Cargo.lock`, once the PR is ready for review (drafts skip it). That job is the answer; you don't need a local build.

To run it yourself, on Linux or Windows, follow the steps of `.github/workflows/wasm-mods.yml` from the repository
root; the tool installs its pinned toolchain through rustup. One wasm compile of `horizon-mod-points` takes about
10 GiB of memory, so set `CARGO_BUILD_JOBS=1` on a 16 GiB machine. To check one change, build only the member that
uses the point and run its test. The build rewrites files in the fixture folders; don't commit them.
