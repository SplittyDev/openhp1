use anyhow::Result;

use super::{Command, Graphics};

pub(super) const COMMAND: Command = Command::new(
    "help",
    "help [command]",
    "List commands or describe one command.",
    execute,
);

fn execute(_graphics: &mut Graphics, arguments: &str) -> Result<String> {
    super::help(arguments)
}
