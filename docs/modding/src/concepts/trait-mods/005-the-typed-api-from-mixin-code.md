<!-- chapter: The typed API from mixin code -->
<!-- separator: 1 -->

### The typed API from mixin code

Every `impl Hooks` must have `#[mod_api::horizon_mod]`. The attribute derives
subscriptions from the methods implemented in that block, so dispatch knows them at installation.
