use anyhow::{Context, Result, bail, ensure};
use openhp1_runtime::ConsoleCommandHost;

use super::{Command, Graphics};

pub(super) const COMMAND: Command = Command::new(
    "respawn",
    "respawn",
    "Respawn from the latest save point in this level.",
    execute,
);

fn execute(graphics: &mut Graphics, arguments: &str) -> Result<String> {
    respawn(
        &mut graphics.console,
        graphics.player,
        graphics.last_save_slot,
        arguments,
    )
}

fn respawn(
    console: &mut dyn ConsoleCommandHost,
    player: usize,
    last_save_slot: Option<u32>,
    arguments: &str,
) -> Result<String> {
    if !arguments.is_empty() {
        bail!("usage: respawn");
    }
    let slot = last_save_slot.context("no save point has been reached in this level")?;
    let response = console.console_command(player, "PlayerPawn", &format!("open save{slot}.usa"));
    ensure!(response.handled, "could not queue the save-point load");
    Ok("Respawning from the latest save point.".to_owned())
}

#[cfg(test)]
mod tests {
    use openhp1_runtime::ConsoleCommandResponse;

    use super::*;

    #[derive(Default)]
    struct RecordingConsole(String);

    impl ConsoleCommandHost for RecordingConsole {
        fn console_command(
            &mut self,
            _actor: usize,
            _class: &str,
            command: &str,
        ) -> ConsoleCommandResponse {
            self.0 = command.to_owned();
            ConsoleCommandResponse {
                handled: true,
                ..Default::default()
            }
        }
    }

    #[test]
    fn queues_the_latest_save_point_through_the_runtime_console() {
        let mut console = RecordingConsole::default();

        assert_eq!(
            respawn(&mut console, 7, Some(12), "").unwrap(),
            "Respawning from the latest save point."
        );
        assert_eq!(console.0, "open save12.usa");
        assert_eq!(
            respawn(&mut console, 7, Some(12), "now")
                .unwrap_err()
                .to_string(),
            "usage: respawn"
        );
    }
}
