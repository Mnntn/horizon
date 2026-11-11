<!-- chapter: Machine-readable SDK catalog -->

## Machine-readable SDK catalog

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

The JSON document has `schema_version: 1`, `api_version`, `hooks`.
Consumers must refuse unknown schema versions. The API version is the mod
manifest API version; it is independent of the catalog format version.

Hooks are sorted by `id` and describe `side`, nullable `scope` (Part `kind` and
`key`), `contract`, ordered `arguments` with API `name` and `type`, and nullable
`output`. Types are Rust API event field types, including their borrow lifetimes.


The build generator owns this catalog. Output has no timestamps or absolute
paths. A format change requires a schema version change; tools should use the
catalog accompanying the build they target. CI regenerates the catalog in-test and compares the two documents for determinism.
The full file ships in the prebuilt SDK and fork Nightly release;
no committed fingerprint needs refreshing when independently authored hooks land.
