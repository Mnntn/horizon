//! Owned data for client play-packet submission; protocol decoding stays in the game.

use crate::Identifier;

/// A serverbound play packet encoded in the connected game's vanilla protocol.
///
/// The game resolves the type name and decodes its body once before sending it. This is not
/// a connection or a protocol implementation, and sending does not apply effects locally.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ServerboundPlayPacket {
    /// Packet type, for example `minecraft:pong` or `minecraft:custom_payload`.
    pub id: Identifier,
    /// Vanilla packet body, without the packet id, length prefix or compression framing.
    pub body: Vec<u8>,
}

impl ServerboundPlayPacket {
    /// Construct a packet envelope; the game validates the body when it is submitted.
    pub fn new(id: Identifier, body: Vec<u8>) -> Self {
        Self { id, body }
    }
}

/// Why the client could not submit a play packet.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SendPacketError {
    /// There is no server connection.
    NotConnected,
    /// The player has not joined, or the connection is reconfiguring.
    NotInPlay,
    /// This protocol has no serverbound packet with the given type name.
    UnknownPacket {
        /// The submitted type name.
        id: Identifier,
    },
    /// The body does not decode completely as the named play packet.
    InvalidBody,
    /// A custom payload exceeds vanilla's size limit.
    PayloadTooLarge {
        /// Number of payload bytes, excluding the channel name.
        size: u64,
        /// Maximum number of payload bytes vanilla accepts.
        limit: u64,
    },
}
