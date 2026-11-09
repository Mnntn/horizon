pub use content::config_files::{
    CONFIG_FILE_MAX, CONFIG_LIST_MAX, ConfigEntry, ConfigEntryKind, ConfigFileError, ConfigFiles,
    ConfigPath, ConfigStat,
};
mod __private {
    mod built {
        pub use crate::content::config_files::built::{ConfigEntry, ConfigStat};
    }
}
