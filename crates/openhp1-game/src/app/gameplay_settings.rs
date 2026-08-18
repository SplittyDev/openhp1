use std::io;

use openhp1_runtime::ConsoleCommands;

use super::graphics_settings::parse_bool;

const CONFIG: &str = "OpenHP1";
const SECTION: &str = "OpenHP1.Gameplay";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct GameplaySettings {
    pub(super) skip_intro: bool,
    pub(super) jump_skips_cutscenes: bool,
    pub(super) auto_learn_spells: bool,
    pub(super) instant_pickup_wizard_cards: bool,
}

impl GameplaySettings {
    pub(super) fn load(console: &ConsoleCommands) -> Self {
        let defaults = Self::default();
        Self {
            skip_intro: setting(console, "SkipIntro").unwrap_or(defaults.skip_intro),
            jump_skips_cutscenes: setting(console, "JumpSkipsCutscenes")
                .unwrap_or(defaults.jump_skips_cutscenes),
            auto_learn_spells: setting(console, "AutoLearnSpells")
                .unwrap_or(defaults.auto_learn_spells),
            instant_pickup_wizard_cards: setting(console, "InstantPickupWizardCards")
                .unwrap_or(defaults.instant_pickup_wizard_cards),
        }
    }

    pub(super) fn save(self, console: &ConsoleCommands) -> io::Result<()> {
        console.save_config_values(
            CONFIG,
            SECTION,
            &[
                ("SkipIntro", self.skip_intro.to_string()),
                ("JumpSkipsCutscenes", self.jump_skips_cutscenes.to_string()),
                ("AutoLearnSpells", self.auto_learn_spells.to_string()),
                (
                    "InstantPickupWizardCards",
                    self.instant_pickup_wizard_cards.to_string(),
                ),
            ],
        )
    }
}

fn setting(console: &ConsoleCommands, key: &str) -> Option<bool> {
    console
        .config_value(CONFIG, SECTION, key)
        .and_then(|value| parse_bool(&value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_the_authored_game_flow() {
        assert_eq!(
            GameplaySettings::default(),
            GameplaySettings {
                skip_intro: false,
                jump_skips_cutscenes: false,
                auto_learn_spells: false,
                instant_pickup_wizard_cards: false,
            }
        );
    }
}
