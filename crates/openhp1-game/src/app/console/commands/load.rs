use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{Command, Graphics};

pub(super) const COMMAND: Command = Command::new(
    "load",
    "load <level>",
    "Load a level from the installed Maps directory.",
    execute,
);

fn execute(graphics: &mut Graphics, arguments: &str) -> Result<String> {
    let path = resolve_level(&graphics.scene.path, &graphics.scene.levels, arguments)?;
    let output = format!(
        "Loading {}.",
        path.file_name()
            .and_then(|name| name.to_str())
            .context("resolved level has no valid file name")?
    );
    graphics.pending_level_load = Some(path);
    Ok(output)
}

fn resolve_level(current: &Path, levels: &[PathBuf], argument: &str) -> Result<PathBuf> {
    let argument = argument.trim();
    let requested = Path::new(argument);
    let mut components = requested.components();
    if argument.contains(['/', '\\'])
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        bail!("level name must be a file in the installed Maps directory");
    }
    if requested
        .extension()
        .is_some_and(|extension| !extension.eq_ignore_ascii_case("unr"))
    {
        bail!("level must be an .unr map");
    }

    let requested = if requested.extension().is_some() {
        argument.to_owned()
    } else {
        format!("{argument}.unr")
    };
    let maps = current
        .parent()
        .filter(|directory| {
            directory
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("Maps"))
        })
        .context("current level is not inside the installed Maps directory")?;
    levels
        .iter()
        .find(|level| {
            level.parent() == Some(maps)
                && level
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("unr"))
                && level
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case(&requested))
        })
        .cloned()
        .with_context(|| format!("level `{argument}` was not found in {}", maps.display()))
}

pub(in crate::app) fn resolve_travel(
    current: &Path,
    levels: &[PathBuf],
    url: &str,
) -> Result<PathBuf> {
    resolve_level(
        current,
        levels,
        url.split_once('?').map_or(url, |(map, _)| map),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_only_case_insensitive_map_names_in_the_active_maps_directory() {
        let current = Path::new("game/Maps/Current.unr");
        let levels = [
            PathBuf::from("game/Maps/Lev_Tut1b.unr"),
            PathBuf::from("game/Maps/Readme.txt"),
            PathBuf::from("game/Textures/Outside.unr"),
        ];

        assert_eq!(
            resolve_level(current, &levels, "lev_tut1B").unwrap(),
            levels[0]
        );
        assert_eq!(
            resolve_level(current, &levels, "LEV_TUT1B.UNR").unwrap(),
            levels[0]
        );
        assert_eq!(
            resolve_travel(current, &levels, "Lev_Tut1b?peer").unwrap(),
            levels[0]
        );
        for invalid in ["", "../Lev_Tut1b", "..\\Lev_Tut1b", "Readme.txt", "Outside"] {
            assert!(
                resolve_level(current, &levels, invalid).is_err(),
                "{invalid}"
            );
        }
        assert!(resolve_level(Path::new("game/Textures/Current.unr"), &levels, "Outside").is_err());
    }
}
