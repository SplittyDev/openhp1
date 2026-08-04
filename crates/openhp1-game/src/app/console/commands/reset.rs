use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::{Command, Graphics};

pub(super) const COMMAND: Command = Command::new(
    "reset",
    "reset",
    "Restart the current level from the beginning.",
    execute,
);

fn execute(graphics: &mut Graphics, arguments: &str) -> Result<String> {
    queue_reset(
        &mut graphics.pending_level_load,
        &graphics.scene.path,
        arguments,
    )
}

fn queue_reset(pending: &mut Option<PathBuf>, current: &Path, arguments: &str) -> Result<String> {
    if !arguments.is_empty() {
        bail!("usage: reset");
    }
    *pending = Some(current.to_owned());
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queues_the_current_level_without_arguments() {
        let path = Path::new("Maps/Lev_Tut1.unr");
        let mut pending = None;

        assert_eq!(queue_reset(&mut pending, path, "").unwrap(), "");
        assert_eq!(pending.as_deref(), Some(path));
        assert!(queue_reset(&mut pending, path, "now").is_err());
    }
}
