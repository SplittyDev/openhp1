use anyhow::Result;

use super::{Command, Graphics, unavailable};

pub(super) const COMMAND: Command =
    Command::new("fly", "fly", "Enter no-clip fly camera mode.", execute);

fn execute(_graphics: &mut Graphics, _arguments: &str) -> Result<String> {
    unavailable("fly")
}
