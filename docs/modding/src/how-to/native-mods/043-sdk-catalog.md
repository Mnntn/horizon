<!-- chapter: Machine-readable SDK catalog -->

## Machine-readable SDK catalog

Tools discover a build's hooks and injection points through `mod-catalog.json`,
shipped at the root of each prebuilt SDK and beside `points.json` in a developer
source build's catalog directory. Converters, linters and IDE helpers must read
this file rather than source paths. Internal source layout is not a tool API.

Fork Nightlies also attach `mod-catalog.json` as a standalone release asset,
listed in the signed `SHA256SUMS`. Download the current
[catalog asset](https://github.com/arismoko/horizon/releases/download/nightly/mod-catalog.json)
from the [fork Nightly release](https://github.com/arismoko/horizon/releases/tag/nightly),
or use the corresponding dated release for an older pinned build.
It is generated from the Nightly's exact published commit using the same
generator as the SDK. Verify it with that
release's checksums and build-specific signature; do not mix the latest catalog
with an older pinned build. The catalog follows the release when it rotates to
a dated Nightly tag and has the same retention as the other Nightly assets.

The JSON document has `schema_version: 1`, `api_version`, `hooks` and `points`.
Consumers must refuse unknown schema versions. The API version is the mod
manifest API version; it is independent of the catalog format version.

Hooks are sorted by `id` and describe `side`, nullable `scope` (Part `kind` and
`key`), `contract`, ordered `arguments` with API `name` and `type`, and nullable
`output`. Types are Rust API event field types, including their borrow lifetimes.
Points are sorted by name and target; a target distinguishes equal names.
Each includes its supported `access` values (`api`, `internals`), API `record`
and ordered `args`, return contract and type, raw shape when available, and
nullable `api_refusal`. Refused and native-only points remain in the file.

The build generator owns this catalog. Output has no timestamps or absolute
paths. A format change requires a schema version change; tools should use the
catalog accompanying the build they target. CI regenerates the catalog in-test and compares the two documents for determinism.
The full file ships in the SDK, developer source build and fork Nightly release;
no committed fingerprint needs refreshing when independently authored hooks land.
