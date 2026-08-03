use std::{
    cell::RefCell,
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    rc::Rc,
};

use openhp1_package::{ConfigEntry, PackageStore};

use crate::{ConsoleCommandHost, ConsoleCommandResponse};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConsoleCommandAction {
    Exit,
    Open(String),
    SaveGame { slot: u32 },
    OpenSave { slot: u32 },
    Screenshot { snapshot: Option<u32> },
    SetResolution { width: u32, height: u32 },
    SetMusicVolume(u8),
    SetSoundVolume(u8),
}

/// The command surface shared by the windowed game and headless corpus runs.
///
/// It shares PackageStore's writable settings overlay and queues operations
/// requiring the game window or platform owner.
#[derive(Clone)]
pub struct ConsoleCommands {
    state: Rc<RefCell<ConsoleState>>,
}

struct ConsoleState {
    packages: PackageStore,
    pending_config: BTreeMap<(String, String, String), String>,
    persist: bool,
    resolution: (u32, u32),
    resolutions: Vec<(u32, u32)>,
    actions: Vec<ConsoleCommandAction>,
}

impl ConsoleCommands {
    pub fn production(
        game_root: impl AsRef<Path>,
        resolution: (u32, u32),
        resolutions: Vec<(u32, u32)>,
    ) -> io::Result<Self> {
        Self::from_game_root(game_root.as_ref(), resolution, resolutions, true)
    }

    pub fn headless(game_root: impl AsRef<Path>) -> io::Result<Self> {
        Self::from_game_root(
            game_root.as_ref(),
            (640, 480),
            vec![(512, 384), (640, 480), (800, 600), (1024, 768)],
            false,
        )
    }

    pub fn settings_dir(&self) -> PathBuf {
        self.state.borrow().packages.settings_dir().to_path_buf()
    }

    pub fn take_actions(&self) -> Vec<ConsoleCommandAction> {
        std::mem::take(&mut self.state.borrow_mut().actions)
    }

    fn from_game_root(
        game_root: &Path,
        resolution: (u32, u32),
        resolutions: Vec<(u32, u32)>,
        persist: bool,
    ) -> io::Result<Self> {
        Self::from_packages(
            PackageStore::scan_game_root(game_root)
                .map_err(|error| io::Error::other(error.to_string()))?,
            resolution,
            resolutions,
            persist,
        )
    }

    #[cfg(test)]
    fn production_with_settings_dir(
        game_root: &Path,
        settings_dir: &Path,
        resolution: (u32, u32),
        resolutions: Vec<(u32, u32)>,
    ) -> io::Result<Self> {
        Self::from_packages(
            PackageStore::scan_game_root_with_settings_dir(game_root, settings_dir)
                .map_err(|error| io::Error::other(error.to_string()))?,
            resolution,
            resolutions,
            true,
        )
    }

    fn from_packages(
        packages: PackageStore,
        resolution: (u32, u32),
        mut resolutions: Vec<(u32, u32)>,
        persist: bool,
    ) -> io::Result<Self> {
        resolutions.push(resolution);
        resolutions.sort_unstable();
        resolutions.dedup();
        Ok(Self {
            state: Rc::new(RefCell::new(ConsoleState {
                packages,
                pending_config: BTreeMap::new(),
                persist,
                resolution,
                resolutions,
                actions: Vec::new(),
            })),
        })
    }
}

impl ConsoleCommandHost for ConsoleCommands {
    fn console_command(
        &mut self,
        _actor: usize,
        _class: &str,
        command: &str,
    ) -> ConsoleCommandResponse {
        let mut state = self.state.borrow_mut();
        let mut words = command.split_ascii_whitespace();
        let Some(name) = words.next() else {
            return ConsoleCommandResponse::default();
        };
        let name = name.to_ascii_lowercase();
        let remaining = words.collect::<Vec<_>>();
        let mut response = ConsoleCommandResponse {
            handled: true,
            ..Default::default()
        };
        match name.as_str() {
            "getres" => {
                response.output = state
                    .resolutions
                    .iter()
                    .map(|(width, height)| format!("{width}x{height}"))
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            "getcolordepths" => response.output = "32 16".to_owned(),
            "getcurrentres" => {
                response.output = format!("{}x{}", state.resolution.0, state.resolution.1)
            }
            "getcurrentcolordepth" => response.output = "32".to_owned(),
            "getping" | "getloss" => response.output = "0".to_owned(),
            "keyname" if remaining.len() == 1 => {
                response.output = remaining[0]
                    .parse::<u8>()
                    .ok()
                    .map(key_name)
                    .unwrap_or_default()
                    .to_owned();
            }
            "keybinding" if remaining.len() == 1 => {
                response.output = config_value_for(&state, "User", "Engine.Input", remaining[0]);
            }
            "get"
                if remaining.len() == 2 && remaining[0].eq_ignore_ascii_case("udpserveruplink") =>
            {
                response.output = "False".to_owned()
            }
            "get" if remaining.len() == 2 => {
                response.output = config_value(&state, remaining[0], remaining[1])
            }
            "set" if remaining.len() >= 2 && remaining[0].eq_ignore_ascii_case("input") => {
                let value = remaining[2..].join(" ");
                set_config_value(&mut state, "User", "Engine.Input", remaining[1], value);
            }
            "set" if remaining.len() >= 3 => {
                let section = config_section(&state, remaining[0]);
                let value = remaining[2..].join(" ");
                set_config_value(&mut state, "System", &section, remaining[1], value.clone());
                match remaining[1].to_ascii_lowercase().as_str() {
                    "musicvolume" => value.parse().ok().map(|value| {
                        state
                            .actions
                            .push(ConsoleCommandAction::SetMusicVolume(value))
                    }),
                    "soundvolume" => value.parse().ok().map(|value| {
                        state
                            .actions
                            .push(ConsoleCommandAction::SetSoundVolume(value))
                    }),
                    _ => None,
                };
            }
            "setres" if remaining.len() == 1 => {
                if let Some((width, height)) = resolution(remaining[0])
                    && state.resolutions.contains(&(width, height))
                {
                    state.resolution = (width, height);
                    state
                        .actions
                        .push(ConsoleCommandAction::SetResolution { width, height });
                } else {
                    response.handled = false;
                }
            }
            "flush" => response.handled = flush(&mut state),
            "savegame" if remaining.len() == 1 => match remaining[0].parse() {
                Ok(slot) => state.actions.push(ConsoleCommandAction::SaveGame { slot }),
                Err(_) => response.handled = false,
            },
            "snap" if remaining.len() == 1 => match remaining[0].parse() {
                Ok(snapshot) => state.actions.push(ConsoleCommandAction::Screenshot {
                    snapshot: Some(snapshot),
                }),
                Err(_) => response.handled = false,
            },
            "shot" if remaining.is_empty() => state
                .actions
                .push(ConsoleCommandAction::Screenshot { snapshot: None }),
            "open" | "start" if remaining.len() == 1 && save_slot(remaining[0]).is_some() => {
                state.actions.push(ConsoleCommandAction::OpenSave {
                    slot: save_slot(remaining[0]).expect("checked save slot"),
                });
            }
            "open" | "start" if remaining.len() == 1 && is_external_url(remaining[0]) => state
                .actions
                .push(ConsoleCommandAction::Open(remaining[0].to_owned())),
            "exit" | "quit" if remaining.is_empty() => {
                state.actions.push(ConsoleCommandAction::Exit)
            }
            _ => response.handled = false,
        }
        response
    }
}

fn is_external_url(value: &str) -> bool {
    [
        "http://",
        "https://",
        "ftp://",
        "telnet://",
        "gopher://",
        "mailto:",
    ]
    .iter()
    .any(|prefix| {
        value
            .get(..prefix.len())
            .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
    }) || value
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("www."))
}

fn config_value(state: &ConsoleState, target: &str, key: &str) -> String {
    config_value_for(state, "System", &config_section(state, target), key)
}

fn config_value_for(state: &ConsoleState, config_name: &str, section: &str, key: &str) -> String {
    state
        .pending_config
        .get(&(
            config_name.to_ascii_lowercase(),
            section.to_ascii_lowercase(),
            key.to_ascii_lowercase(),
        ))
        .cloned()
        .or_else(|| {
            state
                .packages
                .config_values(config_name, section, key)
                .into_iter()
                .next()
        })
        .unwrap_or_default()
}

fn set_config_value(
    state: &mut ConsoleState,
    config_name: &str,
    section: &str,
    key: &str,
    value: String,
) {
    state.pending_config.insert(
        (
            config_name.to_ascii_lowercase(),
            section.to_ascii_lowercase(),
            key.to_ascii_lowercase(),
        ),
        value,
    );
}

fn config_section(state: &ConsoleState, target: &str) -> String {
    let target = target
        .strip_prefix("ini:")
        .unwrap_or(target)
        .to_ascii_lowercase();
    match target.as_str() {
        "engine.engine.viewportmanager" => state
            .packages
            .config_value("Engine.Engine", "ViewportManager")
            .unwrap_or_else(|| "WinDrv.WindowsClient".to_owned())
            .to_ascii_lowercase(),
        "engine.engine.audiodevice" => state
            .packages
            .config_value("Engine.Engine", "AudioDevice")
            .unwrap_or_else(|| "Galaxy.GalaxyAudioSubsystem".to_owned())
            .to_ascii_lowercase(),
        _ => target,
    }
}

fn resolution(value: &str) -> Option<(u32, u32)> {
    let mut values = value.split('x');
    let width = values.next()?.parse().ok()?;
    let height = values.next()?.parse().ok()?;
    match values.next() {
        None => Some((width, height)),
        Some("16" | "32") if values.next().is_none() => Some((width, height)),
        _ => None,
    }
}

fn flush(state: &mut ConsoleState) -> bool {
    if !state.persist {
        // Corpus scanning must execute the same script path without writing
        // an overlay; its in-memory settings are deterministic for the run.
        state.pending_config.clear();
        return true;
    }
    let mut entries = BTreeMap::<String, Vec<ConfigEntry>>::new();
    for ((config, section, key), value) in &state.pending_config {
        entries
            .entry(config.clone())
            .or_default()
            .push(ConfigEntry {
                section: section.clone(),
                key: key.clone(),
                values: vec![value.clone()],
            });
    }
    if entries
        .iter()
        .all(|(config, entries)| state.packages.save_config(config, entries).is_ok())
    {
        state.pending_config.clear();
        true
    } else {
        false
    }
}

fn save_slot(value: &str) -> Option<u32> {
    let value = value.to_ascii_lowercase();
    let slot = value.strip_prefix("save")?.strip_suffix(".usa")?;
    (!slot.is_empty() && slot.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| slot.parse().ok())
        .flatten()
}

fn key_name(key: u8) -> &'static str {
    const NAMES: [&str; 256] = [
        "None",
        "LeftMouse",
        "RightMouse",
        "Cancel",
        "MiddleMouse",
        "Unknown05",
        "Unknown06",
        "Unknown07",
        "Backspace",
        "Tab",
        "Unknown0A",
        "Unknown0B",
        "Unknown0C",
        "Enter",
        "Unknown0E",
        "Unknown0F",
        "Shift",
        "Ctrl",
        "Alt",
        "Pause",
        "CapsLock",
        "Unknown15",
        "Unknown16",
        "Unknown17",
        "Unknown18",
        "Unknown19",
        "Unknown1A",
        "Escape",
        "Unknown1C",
        "Unknown1D",
        "Unknown1E",
        "Unknown1F",
        "Space",
        "PageUp",
        "PageDown",
        "End",
        "Home",
        "Left",
        "Up",
        "Right",
        "Down",
        "Select",
        "Print",
        "Execute",
        "PrintScrn",
        "Insert",
        "Delete",
        "Help",
        "0",
        "1",
        "2",
        "3",
        "4",
        "5",
        "6",
        "7",
        "8",
        "9",
        "Unknown3A",
        "Unknown3B",
        "Unknown3C",
        "Unknown3D",
        "Unknown3E",
        "Unknown3F",
        "Unknown40",
        "A",
        "B",
        "C",
        "D",
        "E",
        "F",
        "G",
        "H",
        "I",
        "J",
        "K",
        "L",
        "M",
        "N",
        "O",
        "P",
        "Q",
        "R",
        "S",
        "T",
        "U",
        "V",
        "W",
        "X",
        "Y",
        "Z",
        "Unknown5B",
        "Unknown5C",
        "Unknown5D",
        "Unknown5E",
        "Unknown5F",
        "NumPad0",
        "NumPad1",
        "NumPad2",
        "NumPad3",
        "NumPad4",
        "NumPad5",
        "NumPad6",
        "NumPad7",
        "NumPad8",
        "NumPad9",
        "GreyStar",
        "GreyPlus",
        "Separator",
        "GreyMinus",
        "NumPadPeriod",
        "GreySlash",
        "F1",
        "F2",
        "F3",
        "F4",
        "F5",
        "F6",
        "F7",
        "F8",
        "F9",
        "F10",
        "F11",
        "F12",
        "F13",
        "F14",
        "F15",
        "F16",
        "F17",
        "F18",
        "F19",
        "F20",
        "F21",
        "F22",
        "F23",
        "F24",
        "Unknown88",
        "Unknown89",
        "Unknown8A",
        "Unknown8B",
        "Unknown8C",
        "Unknown8D",
        "Unknown8E",
        "Unknown8F",
        "NumLock",
        "ScrollLock",
        "Unknown92",
        "Unknown93",
        "Unknown94",
        "Unknown95",
        "Unknown96",
        "Unknown97",
        "Unknown98",
        "Unknown99",
        "Unknown9A",
        "Unknown9B",
        "Unknown9C",
        "Unknown9D",
        "Unknown9E",
        "Unknown9F",
        "LShift",
        "RShift",
        "LControl",
        "RControl",
        "UnknownA4",
        "UnknownA5",
        "UnknownA6",
        "UnknownA7",
        "UnknownA8",
        "UnknownA9",
        "UnknownAA",
        "UnknownAB",
        "UnknownAC",
        "UnknownAD",
        "UnknownAE",
        "UnknownAF",
        "UnknownB0",
        "UnknownB1",
        "UnknownB2",
        "UnknownB3",
        "UnknownB4",
        "UnknownB5",
        "UnknownB6",
        "UnknownB7",
        "UnknownB8",
        "UnknownB9",
        "Semicolon",
        "Equals",
        "Comma",
        "Minus",
        "Period",
        "Slash",
        "Tilde",
        "UnknownC1",
        "UnknownC2",
        "UnknownC3",
        "UnknownC4",
        "UnknownC5",
        "UnknownC6",
        "UnknownC7",
        "Joy1",
        "Joy2",
        "Joy3",
        "Joy4",
        "Joy5",
        "Joy6",
        "Joy7",
        "Joy8",
        "Joy9",
        "Joy10",
        "Joy11",
        "Joy12",
        "Joy13",
        "Joy14",
        "Joy15",
        "Joy16",
        "UnknownD8",
        "UnknownD9",
        "UnknownDA",
        "LeftBracket",
        "Backslash",
        "RightBracket",
        "SingleQuote",
        "UnknownDF",
        "JoyX",
        "JoyY",
        "JoyZ",
        "JoyR",
        "MouseX",
        "MouseY",
        "MouseZ",
        "MouseW",
        "JoyU",
        "JoyV",
        "UnknownEA",
        "UnknownEB",
        "MouseWheelUp",
        "MouseWheelDown",
        "Unknown10E",
        "Unknown10F",
        "JoyPovUp",
        "JoyPovDown",
        "JoyPovLeft",
        "JoyPovRight",
        "UnknownF4",
        "UnknownF5",
        "Attn",
        "CrSel",
        "ExSel",
        "ErEof",
        "Play",
        "Zoom",
        "NoName",
        "PA1",
        "OEMClear",
        "",
    ];
    NAMES[usize::from(key)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn production_host_uses_the_settings_overlay_and_emits_owned_actions() {
        let root = std::env::temp_dir().join(format!("openhp1-console-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let system = root.join("System");
        let settings = root.join("settings");
        fs::create_dir_all(&system).unwrap();
        fs::write(
            system.join("Default.ini"),
            "[Core.System]\nPaths=*.u\n[Engine.Engine]\nViewportManager=WinDrv.WindowsClient\nAudioDevice=Galaxy.GalaxyAudioSubsystem\n[WinDrv.WindowsClient]\nBrightness=0.400000\n[Galaxy.GalaxyAudioSubsystem]\nMusicVolume=128\n",
        )
        .unwrap();
        fs::write(
            system.join("DefUser.ini"),
            "[Engine.Input]\nW=MoveForward\n",
        )
        .unwrap();
        let mut commands = ConsoleCommands::production_with_settings_dir(
            &root,
            &settings,
            (640, 480),
            vec![(640, 480), (1024, 768)],
        )
        .unwrap();

        assert_eq!(
            commands.console_command(0, "Actor", "GetRes").output,
            "640x480 1024x768"
        );
        assert_eq!(
            commands.console_command(
                0,
                "PlayerPawn",
                "get ini:Engine.Engine.ViewportManager Brightness",
            ),
            ConsoleCommandResponse {
                output: "0.400000".to_owned(),
                handled: true,
            },
        );
        assert_eq!(
            commands
                .console_command(0, "PlayerPawn", "KEYBINDING W")
                .output,
            "MoveForward"
        );
        commands.console_command(
            0,
            "PlayerPawn",
            "set ini:Engine.Engine.AudioDevice MusicVolume 200",
        );
        commands.console_command(0, "PlayerPawn", "SET Input W Jump");
        commands.console_command(
            0,
            "PlayerPawn",
            "set ini:Engine.Engine.ViewportManager Brightness 0.65",
        );
        assert!(commands.console_command(0, "Console", "FLUSH").handled);
        let overlay = fs::read_to_string(settings.join("OpenHP1.ini")).unwrap();
        assert!(overlay.contains("[Core.System]\nPaths=*.u"));
        assert!(overlay.contains("[WinDrv.WindowsClient]\nbrightness=0.65"));
        assert!(overlay.contains("[Galaxy.GalaxyAudioSubsystem]\nmusicvolume=200"));
        assert_eq!(
            fs::read_to_string(settings.join("User.ini")).unwrap(),
            "[Engine.Input]\nw=Jump\n",
        );
        commands.console_command(0, "Console", "exit");
        assert_eq!(
            commands.take_actions(),
            [
                ConsoleCommandAction::SetMusicVolume(200),
                ConsoleCommandAction::Exit,
            ],
        );
        assert!(!system.join("User.ini").exists());
        assert_eq!(
            commands.console_command(0, "PlayerPawn", "SaveGame 9"),
            ConsoleCommandResponse {
                output: String::new(),
                handled: true,
            },
        );
        commands.console_command(0, "PlayerPawn", "Snap 3");
        commands.console_command(0, "PlayerPawn", "Shot");
        commands.console_command(0, "PlayerPawn", "open save9.usa");
        assert_eq!(
            commands.take_actions(),
            [
                ConsoleCommandAction::SaveGame { slot: 9 },
                ConsoleCommandAction::Screenshot { snapshot: Some(3) },
                ConsoleCommandAction::Screenshot { snapshot: None },
                ConsoleCommandAction::OpenSave { slot: 9 },
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn key_names_keep_the_binary_numeric_boundaries() {
        assert_eq!(key_name(0x30), "0");
        assert_eq!(key_name(0x37), "7");
        assert_eq!(key_name(0x38), "8");
        assert_eq!(key_name(0x39), "9");
        assert_eq!(key_name(0x3a), "Unknown3A");
        assert_eq!(key_name(0x5a), "Z");
        assert_eq!(key_name(0x60), "NumPad0");
        assert_eq!(key_name(0xec), "MouseWheelUp");
        assert_eq!(key_name(0xff), "");
    }
}
