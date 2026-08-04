use anyhow::{Result, bail};
use openhp1_render::render_to_unreal;

use super::{Command, Graphics};

pub(super) const COMMAND: Command = Command::new(
    "here",
    "here",
    "Move the player to the current fly camera position.",
    execute,
);

fn execute(graphics: &mut Graphics, arguments: &str) -> Result<String> {
    if !arguments.is_empty() {
        bail!("usage: {}", COMMAND.usage);
    }
    if !graphics.fly_camera_active {
        bail!("`here` requires fly camera mode");
    }
    let location = render_to_unreal(graphics.camera.position).to_array();
    let (placed, actions) = graphics.runtime.place_actor(graphics.player, location)?;
    if !placed {
        bail!("the player cannot be placed at the fly camera position");
    }
    graphics.apply_actions(actions);
    Ok(String::new())
}
