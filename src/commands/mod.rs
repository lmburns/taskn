pub(crate) mod edit;
pub(crate) mod interactive;
pub(crate) mod order;
#[cfg(target = "macos")]
pub(crate) mod remind;

use std::str::FromStr;
use anyhow::Result;
use clap::Clap;

use crate::opt::Opt;

/// Available subcommands
#[derive(Clap, Debug, Clone, PartialEq, Copy)]
pub(crate) enum Command {
    /// Edit or create the `taskn` notes
    Edit,
    /// Open an interactive viewer of `task` reminders
    Interactive,
    /// WTF?
    Order,
    /// Set a reminder on `macOS`
    #[cfg(target = "macos")]
    Remind,
}

impl Default for Command {
    fn default() -> Self {
        Self::Edit
    }
}

impl Command {
    /// Does the main work of the program by executing each subcommand with its options
    pub(crate) fn execute(self, opt: &Opt) -> Result<()> {
        match self {
            Self::Edit => edit::execute(opt),
            Self::Interactive => interactive::execute(opt),
            Self::Order => order::execute(opt),
            #[cfg(target = "macos")]
            Self::Remind => remind::execute(opt),
        }
    }
}

impl FromStr for Command {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "edit" => Ok(Self::Edit),
            "interactive" => Ok(Self::Interactive),
            "order" => Ok(Self::Order),
            #[cfg(target = "macos")]
            "remind" => Ok(Self::Remind),
            _ => Err(format!("failed to parse command from '{}'", s)),
        }
    }
}
