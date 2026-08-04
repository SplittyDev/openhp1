use anyhow::Result;

use super::{Command, Graphics, unavailable};

pub(super) const COMMAND: Command = Command::new(
    "here",
    "here",
    "Move the player to the current fly camera position.",
    execute,
);

fn execute(_graphics: &mut Graphics, _arguments: &str) -> Result<String> {
    unavailable("here")
}
