use anyhow::Result;

use super::{Command, Graphics, unavailable};

pub(super) const COMMAND: Command = Command::new(
    "report",
    "report <issue>",
    "Write a compact gameplay and capability-debug report.",
    execute,
);

fn execute(_graphics: &mut Graphics, _arguments: &str) -> Result<String> {
    unavailable("report")
}
