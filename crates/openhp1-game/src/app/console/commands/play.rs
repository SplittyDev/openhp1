use anyhow::Result;

use super::{Command, Graphics, unavailable};

pub(super) const COMMAND: Command = Command::new(
    "play",
    "play",
    "Return from the fly camera to the player camera.",
    execute,
);

fn execute(_graphics: &mut Graphics, _arguments: &str) -> Result<String> {
    unavailable("play")
}
