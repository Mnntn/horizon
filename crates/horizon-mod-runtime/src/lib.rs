//! Engine-independent storage and registration machinery shared by the game and mod API.
pub mod attachments;
pub mod dispatch_failures;
/// Load-owned registry names and numeric slots, without game or API key types.
pub mod registry_keys;

pub mod block_definitions;
pub mod block_entities;
/// Immutable hook subscription bitsets, resolved once at installation.
pub mod subscribers;
