# Summary

[Introduction](introduction.md)

# Start

- [Your first native mod](start/first-mod.md)
- [Installing and using mods](start/installing.md)

# Concepts

<!-- The concept-pages preprocessor adds every other page of concepts/ (and of how-to/ under How-to)
     after the ones listed here, in title order, and fails the build on a page the book doesn't render.
     List a page here only to place it first or to show it in a part outside its folder. Aggregate
     chapters keep their published anchors; their child chapters are discovered too. -->

- [Wire types](how-to/wire-types.md)
- [Trait mods, Parts and attachments](concepts/trait-mods.md)
- [Where the mod API is going](concepts/design.md)
- [Design: vehicle health layout](concepts/vehicle-health-layout.md)
- [Design: per-type batched block entity ticks](concepts/block-entity-batching.md)
- [Design: Source-validated guest point artifact](concepts/guest-point-artifact.md)
- [Design: typed cauldron interactions](concepts/cauldron-interactions.md)
- [Design: villager professions and trades](concepts/villager-professions.md)
- [Villager profession resources and clients](concepts/villager-professions-client.md)

# How-to

- [Making a content mod](how-to/content-mods.md)
- [Native mods](how-to/native-mods.md)
- [Injections](injections.md)
- [Mixins](mixins.md)

# Reference

- [NeoForge data files](reference/neoforge-data.md)
- [NeoForge extension coverage](reference/neoforge-extensions.md)
- [The manifest and the package](reference/manifest.md)
- [The registry index and `.hpack`](reference/registry-index.md)
- [The mod API (rustdoc)](reference/api.md)
<!-- The wasm-coverage preprocessor renders this chapter from the generator at its published URL. -->
- [Wasm boundary coverage]()
<!-- The hook-notes preprocessor supplies this chapter and its published URL from catalog notes. -->
- [Hook notes]()
<!-- The counterparts preprocessor renders the current declarations at the published URL. -->
- [NeoForge counterparts]()

# For agents

- [Rules for agents](agents.md)

# Contributing

- [Where each doc lives](contributing.md)
- [Design: scoped whole-stack rollback](reference/design-stack-snapshot.md)
