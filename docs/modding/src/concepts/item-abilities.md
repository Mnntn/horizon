# Item abilities

An item's Part can change the answer to an ability question. The hook receives a read-only Host. Keep answers pure functions of the stack and ability so the client and server agree.

```rust,ignore
#[horizon_part]
impl ItemPart for CarvingTool {
    fn can_perform_action(&self, _: &dyn Host, event: &mut CanPerformActionEvent<'_>) {
        if event.ability == "minecraft:shears_carve" {
            event.can_perform = true;
        }
    }
}
```

Attach this Part to your item with the registrar's `item_part` method. This grants pumpkin carving only: `minecraft:shears_harvest` is a separate ability for mob and dispenser shearing. `minecraft:shield_block` permits blocking and uses the stack's blocking component, or the default component when absent. Leave unknown abilities unchanged. Tool abilities and other excluded consumers are not implemented; this is partial coverage, not a promise that every item-kind check is an ability query.
