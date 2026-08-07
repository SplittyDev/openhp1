use std::{
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::{ErrorKind, Write as _},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use openhp1_scene::render_to_unreal;

use super::{Command, Graphics};

const NEARBY_RADIUS: f32 = 2_048.0;

pub(super) const COMMAND: Command = Command::new(
    "report",
    "report <issue>",
    "Write a compact gameplay and capability-debug report.",
    execute,
);

fn execute(graphics: &mut Graphics, arguments: &str) -> Result<String> {
    let issue = parse_issue(arguments)?;
    let captured = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?;
    let reports_dir = graphics.console.settings_dir().join("Reports");
    fs::create_dir_all(&reports_dir)
        .with_context(|| format!("could not create {}", reports_dir.display()))?;
    let report = report_text(graphics, issue, captured);

    let mut collision = 0;
    loop {
        let path = reports_dir.join(report_filename(captured, collision));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(report.as_bytes())
                    .with_context(|| format!("could not write {}", path.display()))?;
                return Ok(format!("saved report to {}", path.display()));
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                collision = collision
                    .checked_add(1)
                    .context("too many reports share this timestamp")?;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("could not create {}", path.display()));
            }
        }
    }
}

fn parse_issue(arguments: &str) -> Result<&str> {
    let issue = arguments.trim();
    if issue.is_empty() {
        bail!("usage: report <issue>");
    }
    let quoted = [('"', '"'), ('\'', '\'')]
        .into_iter()
        .find(|&(open, close)| issue.starts_with(open) || issue.ends_with(close));
    let Some((open, close)) = quoted else {
        return Ok(issue);
    };
    if !issue.starts_with(open) || !issue.ends_with(close) || issue.len() < 2 {
        bail!("issue has an unmatched quote");
    }
    let issue = issue[1..issue.len() - 1].trim();
    if issue.is_empty() {
        bail!("usage: report <issue>");
    }
    Ok(issue)
}

fn report_filename(captured: Duration, collision: u32) -> String {
    let suffix = if collision == 0 {
        String::new()
    } else {
        format!("-{collision}")
    };
    format!(
        "report-{}-{:09}{suffix}.md",
        captured.as_secs(),
        captured.subsec_nanos()
    )
}

fn report_text(graphics: &Graphics, issue: &str, captured: Duration) -> String {
    let mut report = String::new();
    writeln!(report, "# OpenHP1 gameplay report\n").unwrap();
    writeln!(
        report,
        "Captured at Unix time `{}.{:09}`.\n",
        captured.as_secs(),
        captured.subsec_nanos()
    )
    .unwrap();
    writeln!(report, "## Issue\n").unwrap();
    write_issue(&mut report, issue);

    let level = graphics
        .scene
        .path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let player = graphics.scene.actors.get(graphics.player);
    let player_location = player.map(|actor| actor.location);
    let camera_location = render_to_unreal(graphics.camera.position);
    let camera_mode = if graphics.fly_camera_active {
        "fly"
    } else {
        "play"
    };
    writeln!(report, "\n## Game state\n").unwrap();
    writeln!(report, "- Level: `{level}`").unwrap();
    match player {
        Some(player) => {
            writeln!(
                report,
                "- Player: `{}` (actor #{}, class `{}`)",
                player.name, graphics.player, player.class_name
            )
            .unwrap();
            writeln!(report, "- Player location: `{}`", vec3(player.location)).unwrap();
            writeln!(
                report,
                "- Player rotation (pitch, yaw, roll): `{}, {}, {}`",
                player.rotation.pitch, player.rotation.yaw, player.rotation.roll
            )
            .unwrap();
        }
        None => writeln!(report, "- Player: missing actor #{}", graphics.player).unwrap(),
    }
    writeln!(
        report,
        "- Player runtime state: `{}`",
        graphics.runtime.player_state_name().unwrap_or("None")
    )
    .unwrap();
    writeln!(report, "- Camera mode: `{camera_mode}`").unwrap();
    writeln!(report, "- Camera/view actor: #{}", graphics.view_actor).unwrap();
    writeln!(
        report,
        "- Camera location (Unreal coordinates): `{}`",
        vec3(camera_location)
    )
    .unwrap();
    writeln!(
        report,
        "- Camera rotation (radians; yaw, pitch, roll): `{:.4}, {:.4}, {:.4}`",
        graphics.camera.yaw, graphics.camera.pitch, graphics.camera.roll
    )
    .unwrap();

    let settings = graphics.renderer.settings();
    writeln!(report, "\n## Renderer\n").unwrap();
    writeln!(report, "- Mode: `{:?}`", settings.mode).unwrap();
    writeln!(report, "- Tone mapper: `{:?}`", settings.tone_mapper).unwrap();
    writeln!(
        report,
        "- Ambient occlusion: `{:?}`",
        settings.ambient_occlusion
    )
    .unwrap();
    writeln!(report, "- Anti-aliasing: `{:?}`", settings.antialiasing).unwrap();
    writeln!(report, "- Bloom: `{}`", settings.bloom).unwrap();
    writeln!(
        report,
        "- Internal resolution: `{}x{}`",
        graphics.graphics_settings.resolution[0], graphics.graphics_settings.resolution[1]
    )
    .unwrap();
    writeln!(
        report,
        "- Color depth: `{:?}`",
        graphics.graphics_settings.color_depth
    )
    .unwrap();
    writeln!(
        report,
        "- Display brightness / contrast: `{:.3} / {:.3}`",
        graphics.display_settings.brightness, graphics.display_settings.contrast
    )
    .unwrap();
    writeln!(
        report,
        "- Draw calls: `{}`",
        graphics.render_stats.draw_calls
    )
    .unwrap();
    writeln!(
        report,
        "- Texture / lightmap memory: `{:.2} MiB / {:.2} MiB`",
        mib(graphics.render_stats.texture_memory_bytes),
        mib(graphics.render_stats.lightmap_memory_bytes)
    )
    .unwrap();
    writeln!(report, "- Frame time: `{:.2} ms`", graphics.frame_time_ms).unwrap();
    writeln!(
        report,
        "- Scene triangles: `{}`",
        graphics.scene.render.mesh.indices.len() / 3
    )
    .unwrap();

    writeln!(report, "\n## Runtime context\n").unwrap();
    writeln!(
        report,
        "- Active actors: `{}`",
        graphics.runtime.active_actor_count()
    )
    .unwrap();
    writeln!(
        report,
        "- Deferred runtime calls: `{}`",
        graphics.deferred_calls
    )
    .unwrap();
    writeln!(
        report,
        "- Last error: {}",
        graphics.last_error.as_deref().unwrap_or("None")
    )
    .unwrap();

    writeln!(report, "\n## Capability warnings seen so far\n").unwrap();
    let mut warnings = 0;
    for (index, actor) in graphics.scene.actors.iter().enumerate() {
        for diagnostic in &actor.diagnostics {
            warnings += 1;
            writeln!(
                report,
                "- Actor #{index} `{}` (`{}`): {diagnostic}",
                actor.name, actor.class_name
            )
            .unwrap();
        }
    }
    if warnings == 0 {
        writeln!(report, "- None").unwrap();
    }

    writeln!(report, "\n## Nearby named entities\n").unwrap();
    writeln!(
        report,
        "All named scene actors within `{NEARBY_RADIUS:.0}` Unreal units of the player are listed, nearest first.\n"
    )
    .unwrap();
    let mut nearby = graphics
        .scene
        .actors
        .iter()
        .enumerate()
        .filter_map(|(index, actor)| {
            let distance = actor.location.distance(player_location?);
            (index != graphics.player && !actor.name.trim().is_empty() && distance <= NEARBY_RADIUS)
                .then_some((distance, index, actor))
        })
        .collect::<Vec<_>>();
    nearby.sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.cmp(&right.1)));
    if nearby.is_empty() {
        writeln!(report, "- None").unwrap();
    }
    for (distance, index, actor) in nearby {
        writeln!(
            report,
            "### `{}` (actor #{index}, {:.1} units)\n",
            actor.name, distance
        )
        .unwrap();
        writeln!(
            report,
            "- Identity: package `{}`, export index `{}`; class `{}`",
            actor.id.package, actor.id.export_index, actor.class_name
        )
        .unwrap();
        writeln!(report, "- Location: `{}`", vec3(actor.location)).unwrap();
        writeln!(
            report,
            "- Rotation (pitch, yaw, roll): `{}, {}, {}`",
            actor.rotation.pitch, actor.rotation.yaw, actor.rotation.roll
        )
        .unwrap();
        writeln!(
            report,
            "- Visual state: hidden=`{}`, draw type=`{}`, mesh=`{}`",
            actor.hidden,
            actor.draw_type,
            actor.mesh_name.as_deref().unwrap_or("None")
        )
        .unwrap();
        if let Some(animation) = &actor.animation {
            writeln!(
                report,
                "- Animation: `{}` phase `{:.3}`, rate `{:.3}`",
                animation.sequence, animation.phase, animation.rate
            )
            .unwrap();
        }
        if actor.diagnostics.is_empty() {
            writeln!(report, "- Diagnostics: None").unwrap();
        } else {
            writeln!(report, "- Diagnostics: {}", actor.diagnostics.join("; ")).unwrap();
        }
    }
    report
}

fn vec3(value: glam::Vec3) -> String {
    format!("{:.1}, {:.1}, {:.1}", value.x, value.y, value.z)
}

fn write_issue(report: &mut String, issue: &str) {
    for line in issue.lines() {
        writeln!(report, "> {line}").unwrap();
    }
}

fn mib(bytes: usize) -> f32 {
    bytes as f32 / (1024.0 * 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_and_quoted_issue_text() {
        assert_eq!(parse_issue("moving pillar").unwrap(), "moving pillar");
        assert_eq!(
            parse_issue("  \"moving pillar after Flipendo\"  ").unwrap(),
            "moving pillar after Flipendo"
        );
        assert!(parse_issue("").is_err());
        assert!(parse_issue("\"unfinished").is_err());
        assert!(parse_issue("\"\"").is_err());
    }

    #[test]
    fn filename_is_timestamped_and_collision_safe() {
        let captured = Duration::new(1_726_000_001, 42);
        assert_eq!(
            report_filename(captured, 0),
            "report-1726000001-000000042.md"
        );
        assert_eq!(
            report_filename(captured, 2),
            "report-1726000001-000000042-2.md"
        );
    }

    #[test]
    fn issue_is_written_as_markdown_quote() {
        let mut text = String::new();
        write_issue(&mut text, "pillar moved\nthen reset");
        assert_eq!(text, "> pillar moved\n> then reset\n");
    }
}
