# Villager profession resources and clients

Profession layers use the registered profession's qualified identity. For
`test:engineer`, provide
`assets/test/textures/entity/villager/profession/engineer.png` and
`assets/test/textures/entity/zombie_villager/profession/engineer.png`.
Nested identity paths remain nested texture paths. Vanilla professions use the
same resource lookup.

The texture's `.png.mcmeta` supplies its hat policy:

```json
{"villager":{"hat":"full"}}
```

`none` keeps the type layer's head visible, `full` hides it, and `partial`
hides it only when the type texture has a full hat. Missing metadata means
`none`. Pack reload refreshes the cached metadata. Unresolved saved professions
retain their names and render the neutral base without a profession layer.

Clients must have the same required mods and versions. The existing configuration
mod-list handshake refuses a missing mod by name before play and entity pairing.
Profession owners must be required on both sides. No profession-specific packet
or negotiation mechanism is added. Both sides resolve vanilla VillagerData's
integer profession index against their local deterministic table. Unresolved
server identities retain the existing `none` network projection without changing
the saved identity. Shared configuration registry synchronization is follow-up
work depending on #2882.
