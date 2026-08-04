use anyhow::Result;

use super::{Command, Graphics, unavailable};

pub(super) const COMMAND: Command = Command::new(
    "respawn",
    "respawn",
    "Respawn from the latest save point in this level.",
    execute,
);

fn execute(_graphics: &mut Graphics, _arguments: &str) -> Result<String> {
    unavailable("respawn")
}
