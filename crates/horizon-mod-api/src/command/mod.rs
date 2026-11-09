//! Parsed native command input and the source that executes it.

use crate::{DimensionKey, EntityRef, Identifier, ItemStack, PermissionLevel, PlayerRef};

/// The origin of a registered mod command, independent of `execute as`'s executor.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum CommandSource {
    /// A player's command; their state is swapped in for the listener.
    Player {
        /// The player who submitted the command.
        player: PlayerRef,
        /// The execution permission, including function overrides.
        permission: PermissionLevel,
    },
    /// The dedicated server console or RCON.
    Console {
        /// The execution permission.
        permission: PermissionLevel,
    },
    /// A command block, including when `execute` moves its execution position.
    CommandBlock {
        /// The block's position, not the execution position.
        position: [i32; 3],
        /// The block's dimension.
        level: DimensionKey,
        /// The execution permission.
        permission: PermissionLevel,
    },
    /// A command-block minecart.
    CommandBlockMinecart {
        /// The minecart that submitted the command.
        entity: EntityRef,
        /// The execution permission.
        permission: PermissionLevel,
    },
    /// The innermost executing function; feedback still follows its caller.
    Function {
        /// The function's namespaced identifier.
        id: Identifier,
        /// The function's execution permission.
        permission: PermissionLevel,
    },
}

impl CommandSource {
    /// The effective permission checked before parsing and argument resolution.
    pub fn permission(&self) -> PermissionLevel {
        match self {
            Self::Player { permission, .. }
            | Self::Console { permission }
            | Self::CommandBlock { permission, .. }
            | Self::CommandBlockMinecart { permission, .. }
            | Self::Function { permission, .. } => *permission,
        }
    }
}

/// One validated native command argument, resolved against its command source.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum CommandArgumentValue {
    /// A `word` or greedy `text` argument, exactly as the dispatcher parsed it.
    String(String),
    /// A signed `integer` argument.
    Integer(i32),
    /// A finite double-precision `number` argument.
    Number(f64),
    /// A `boolean` argument.
    Boolean(bool),
    /// The single online player selected by a `player` argument.
    Player(PlayerRef),
    /// An `item` argument with its components applied and a count of one.
    Item(ItemStack),
    /// A `block_pos` argument resolved from world, relative or local coordinates.
    /// Resolution does not load chunks or require the block to be loaded.
    BlockPos([i32; 3]),
    /// A custom argument type's canonical value: the registered type's id
    /// (`mymod:machine`) and what its parser returned for the typed text.
    Custom {
        /// The id of the registered `CommandArgumentType`.
        kind: String,
        /// The value `CommandArgumentType::parse` returned.
        value: String,
    },
}

/// An argument's registered name and its validated value.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct CommandArgument {
    /// The name from the command's usage, without its kind or brackets.
    pub name: String,
    /// The value produced by parsing and source resolution.
    pub value: CommandArgumentValue,
}

impl CommandArgument {
    /// Pair a registered argument name with its parsed value.
    pub fn new(name: String, value: CommandArgumentValue) -> Self {
        Self { name, value }
    }
}

/// A native command parsed once by the server's dispatcher, never a raw line.
///
/// Missing optional arguments are absent. Player selectors must resolve to one
/// online player before any listener runs; a failed parse or resolution produces
/// command feedback instead. Every source receives this same data.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct CommandInvocation {
    /// The literal path, including the registered command name and subcommands.
    pub literals: Vec<String>,
    /// Present arguments in parse order; omitted optional arguments have no entry.
    pub arguments: Vec<CommandArgument>,
}

impl CommandInvocation {
    /// Build an invocation from its parsed literal path and resolved arguments.
    pub fn new(literals: Vec<String>, arguments: Vec<CommandArgument>) -> Self {
        Self {
            literals,
            arguments,
        }
    }

    /// The last argument registered under `name`, or `None` when it is absent.
    pub fn argument(&self, name: &str) -> Option<&CommandArgumentValue> {
        self.arguments
            .iter()
            .rev()
            .find(|argument| argument.name == name)
            .map(|argument| &argument.value)
    }
}

pub(crate) mod command_argument;
pub(crate) mod suggestion;
pub(crate) mod types;
