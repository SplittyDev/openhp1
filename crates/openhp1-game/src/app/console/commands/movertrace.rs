use anyhow::{Result, bail};

use super::{Command, Graphics};

pub(super) const COMMAND: Command = Command::new(
    "movertrace",
    "movertrace <on|off>",
    "Capture moving-brush collision decisions in gameplay reports.",
    execute,
);

fn execute(graphics: &mut Graphics, arguments: &str) -> Result<String> {
    let enabled = match arguments.trim().to_ascii_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => bail!("usage: movertrace <on|off>"),
    };
    graphics.runtime.set_mover_trace_enabled(enabled);
    Ok(format!(
        "Mover trace {}.",
        if enabled { "enabled" } else { "disabled" }
    ))
}
