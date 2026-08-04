use anyhow::{Context, Result, bail};
use openhp1_scene::Rotator;
use winit::dpi::PhysicalSize;

use super::{Command, Graphics};
use crate::app::camera_from_player_view;

pub(super) const COMMAND: Command = Command::new(
    "play",
    "play",
    "Return from the fly camera to the player camera.",
    execute,
);

fn execute(graphics: &mut Graphics, arguments: &str) -> Result<String> {
    if !arguments.is_empty() {
        bail!("usage: play");
    }
    if !graphics.fly_camera_active {
        return Ok(String::new());
    }

    let player = graphics
        .scene
        .actors
        .get(graphics.player)
        .context("the player disappeared from the scene")?;
    let location = player.location.to_array();
    let rotation = [
        player.rotation.pitch,
        player.rotation.yaw,
        player.rotation.roll,
    ];
    let (view, actions) = graphics.runtime.player_view(location, rotation)?;
    graphics.apply_actions(actions);
    graphics.view_actor = view.actor;
    graphics.camera = camera_from_player_view(
        view,
        PhysicalSize::new(graphics.config.width, graphics.config.height),
        graphics.camera.far,
    );
    if graphics.scene.update_sprite_billboards(Rotator {
        pitch: view.rotation[0],
        yaw: view.rotation[1],
        roll: view.rotation[2],
    }) {
        graphics.vertices_dirty = true;
    }
    graphics.fly_camera_active = false;
    Ok(String::new())
}
