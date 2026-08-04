use anyhow::Result;

use super::{Command, Graphics, unavailable};

pub(super) const COMMAND: Command = Command::new(
    "reset",
    "reset",
    "Restart the current level from the beginning.",
    execute,
);

fn execute(_graphics: &mut Graphics, _arguments: &str) -> Result<String> {
    unavailable("reset")
}
