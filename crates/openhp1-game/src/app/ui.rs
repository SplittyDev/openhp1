use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use egui::{Align2, Color32, FontId, Id, LayerId, Order, Pos2, Rect, Sense, TextureHandle, Vec2};
use openhp1_audio::AudioClip;
use openhp1_package::{ConfigEntry, ObjectReference, PackageStore, ResolvedObject};
use openhp1_runtime::PlayerUiState;
use openhp1_texture::{Palette, Texture};

const REFERENCE_SIZE: Vec2 = Vec2::new(640.0, 480.0);
const AUTHORED_RESOLUTIONS: [(u32, u32); 4] = [(512, 384), (640, 480), (800, 600), (1024, 768)];
const STORY_MIN_TIME_ON_PAGE: Duration = Duration::from_secs(2);
const STORY_SOUND_ADVANCE: Duration = Duration::from_millis(100);
const STORY_TRAILING_TIME: Duration = Duration::from_secs(1);
const QUIDDITCH_FIXTURES: [QuidditchFixture; 6] = [
    QuidditchFixture::new(0, 3, 1, 2, "Quid_SlythA.unr"),
    QuidditchFixture::new(0, 1, 2, 3, "Quid_RavenA.unr"),
    QuidditchFixture::new(0, 2, 1, 3, "Quid_HuffleA.unr"),
    QuidditchFixture::new(3, 0, 2, 1, "Quid_SlythB.unr"),
    QuidditchFixture::new(1, 0, 3, 2, "Quid_RavenB.unr"),
    QuidditchFixture::new(2, 0, 3, 1, "Quid_HuffleB.unr"),
];
const QUIDDITCH_FINAL_LEVELS: [&str; 4] =
    ["", "Quid_RavenC.unr", "Quid_HuffleC.unr", "Quid_SlythC.unr"];
const NEW_GAME_STORY: [(&str, &str); 14] = [
    ("3_1_", "StoryBook1"),
    ("3_2_", "StoryBook2"),
    ("3_3_", "StoryBook3"),
    ("3_3_", "StoryBook52"),
    ("3_4_", "StoryBook4"),
    ("3_5_", "StoryBook5"),
    ("3_5_", "storybook_new_20"),
    ("3_6_", "storybook50"),
    ("6_6_", "storybook_new_21"),
    ("3_7_", "StoryBook7"),
    ("3_7_", "StoryBook53"),
    ("3_7_", "StoryBook54"),
    ("3_7_", "StoryBook55"),
    ("3_7_", "StoryBook49"),
];
const WIZARD_CARDS: [(i32, &str, &str); 25] = [
    (101, "Dumbledore", "wizard_card_new_04b"),
    (2, "Cornelius", "wizard_card_new_10"),
    (69, "Bertie", "wizard_card_new_23"),
    (17, "Morgan", "wizard_card_new_07"),
    (41, "Godric", "wizard_card_new_17"),
    (72, "Helga", "wizard_card_new_24"),
    (49, "Elladora", "wizard_card_new_20"),
    (1, "Merlin", "wizard_card_new_01"),
    (10, "Burdock", "wizard_card_new_02"),
    (18, "Uric", "wizard_card_new_08"),
    (57, "Gifford", "wizard_card_new_21"),
    (83, "Roderic", "wizard_card_new_27"),
    (100, "Harry", "wizard_card_new_03"),
    (82, "Rowena", "wizard_card_new_26"),
    (19, "Newt", "wizard_card_new_09"),
    (8, "Derwent", "wizard_card_new_25"),
    (48, "Salizar", "wizard_card_new_19"),
    (47, "Edgar", "wizard_card_new_18"),
    (28, "Tilly", "wizard_card_new_12"),
    (37, "Cassandra", "wizard_card_new_15"),
    (24, "Adalbert", "wizard_card_new_11"),
    (62, "Ignatia", "wizard_card_new_22"),
    (96, "Hengist", "wizard_card_new_28"),
    (35, "Bowman", "wizard_card_new_14"),
    (11, "Herpo", "wizard_card_new_05"),
];

pub(super) enum Action {
    Exit,
    LoadSave(u32),
    LoadLevel(String),
    NewGame(u32),
    PlayUiSound(AudioClip),
    Resume,
    SetBrightness(f32),
    SetMusicVolume(u8),
    SetResolution(u32, u32),
    SetSoundVolume(u8),
}

pub(super) struct OptionsState {
    pub(super) resolution: (u32, u32),
    pub(super) resolutions: Vec<(u32, u32)>,
    pub(super) brightness: f32,
    pub(super) music_volume: f32,
    pub(super) sound_volume: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Main,
    Slots,
    Options,
    Quidditch,
    Report,
    Folio,
    StoryBook,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QuidditchScreen {
    Start,
    Instructions,
    Matchup,
    Results,
    FinalResults,
}

#[derive(Clone, Copy)]
struct QuidditchFixture {
    home: usize,
    visitor: usize,
    other_home: usize,
    other_visitor: usize,
    level: &'static str,
}

impl QuidditchFixture {
    const fn new(
        home: usize,
        visitor: usize,
        other_home: usize,
        other_visitor: usize,
        level: &'static str,
    ) -> Self {
        Self {
            home,
            visitor,
            other_home,
            other_visitor,
            level,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct QuidditchScore {
    home: i32,
    visitor: i32,
    other_home: i32,
    other_visitor: i32,
}

#[derive(Clone, Copy, Default)]
struct QuidditchTeam {
    wins: i32,
    losses: i32,
    points: i32,
}

#[derive(Clone)]
struct QuidditchLeague {
    screen: QuidditchScreen,
    current_game: usize,
    finals: bool,
    final_teams: [usize; 2],
    scores: [QuidditchScore; 7],
    teams: [QuidditchTeam; 4],
    random: u64,
}

impl Default for QuidditchLeague {
    fn default() -> Self {
        Self {
            screen: QuidditchScreen::Start,
            current_game: 0,
            finals: false,
            final_teams: [0, 1],
            scores: [QuidditchScore::default(); 7],
            teams: [QuidditchTeam::default(); 4],
            random: 1,
        }
    }
}

impl QuidditchLeague {
    fn restart(&mut self) {
        *self = Self {
            screen: QuidditchScreen::Instructions,
            random: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(1, |duration| duration.as_nanos() as u64)
                .max(1),
            ..Self::default()
        };
    }

    fn fixture(&self) -> QuidditchFixture {
        if !self.finals {
            return QUIDDITCH_FIXTURES[self.current_game];
        }
        let [home, visitor] = self.final_teams;
        let opponent = if home == 0 { visitor } else { home };
        QuidditchFixture::new(
            home,
            visitor,
            home,
            visitor,
            if home == 0 || visitor == 0 {
                QUIDDITCH_FINAL_LEVELS[opponent]
            } else {
                ""
            },
        )
    }

    fn finish(&mut self, team0_score: i32, opponent_score: i32) {
        let fixture = self.fixture();
        let score_index = self.current_game;
        let team0_played = fixture.home == 0 || fixture.visitor == 0;
        if team0_played {
            let (home, visitor) = if fixture.home == 0 {
                (team0_score, opponent_score)
            } else {
                (opponent_score, team0_score)
            };
            self.scores[score_index].home = home;
            self.scores[score_index].visitor = visitor;
            self.record_result(fixture.home, fixture.visitor, home, visitor);
        }
        if !self.finals || !team0_played {
            let mut home = (6 + self.rand8()) * 10;
            let mut visitor = (6 + self.rand8()) * 10;
            if home == visitor {
                home += 10;
            }
            if home > visitor {
                home += 150;
            } else {
                visitor += 150;
            }
            self.scores[score_index].other_home = home;
            self.scores[score_index].other_visitor = visitor;
            self.record_result(fixture.other_home, fixture.other_visitor, home, visitor);
        }
        if self.finals {
            self.screen = QuidditchScreen::FinalResults;
        } else {
            self.current_game += 1;
            if self.current_game == QUIDDITCH_FIXTURES.len() {
                self.finals = true;
                let standings = self.sort_teams();
                self.final_teams = [standings[0], standings[1]];
            }
            self.screen = QuidditchScreen::Results;
        }
    }

    fn record_result(&mut self, home: usize, visitor: usize, home_score: i32, visitor_score: i32) {
        if home_score > visitor_score {
            self.teams[home].wins += 1;
            self.teams[visitor].losses += 1;
        } else {
            self.teams[visitor].wins += 1;
            self.teams[home].losses += 1;
        }
        self.teams[home].points += home_score;
        self.teams[visitor].points += visitor_score;
    }

    fn sort_teams(&mut self) -> [usize; 4] {
        let mut standings = [usize::MAX; 4];
        for team in 0..self.teams.len() {
            let mut position = 0;
            for other in 0..self.teams.len() {
                if team == other {
                    continue;
                }
                if self.teams[team].wins < self.teams[other].wins
                    || (self.teams[team].wins == self.teams[other].wins
                        && self.teams[team].points < self.teams[other].points)
                {
                    position += 1;
                } else if self.teams[team].wins == self.teams[other].wins
                    && self.teams[team].points == self.teams[other].points
                {
                    self.teams[team].points += 10;
                }
            }
            standings[position] = team;
        }
        for team in 0..self.teams.len() {
            if !standings.contains(&team)
                && let Some(empty) = standings.iter_mut().find(|place| **place == usize::MAX)
            {
                *empty = team;
            }
        }
        standings
    }

    fn rand8(&mut self) -> i32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random % 8) as i32
    }
}

struct CardTextures {
    big: TextureHandle,
    small: TextureHandle,
}

struct StoryPage {
    art: [TextureHandle; 4],
    text: String,
    sound: AudioClip,
    duration: Duration,
}

struct UiTextures {
    main_background: Vec<TextureHandle>,
    logo: Vec<TextureHandle>,
    save_background: Vec<TextureHandle>,
    empty_slot: TextureHandle,
    back: TextureHandle,
    back_hover: TextureHandle,
    options_background: Vec<TextureHandle>,
    option_bar: TextureHandle,
    option_bar_open: TextureHandle,
    combo_list_small: TextureHandle,
    combo_list_large: TextureHandle,
    combo_list_selection: TextureHandle,
    quidditch_background: Vec<TextureHandle>,
    broomstick_practice_locked: TextureHandle,
    broomstick_practice: TextureHandle,
    quidditch_league_locked: TextureHandle,
    quidditch_league: TextureHandle,
    quidditch_back: TextureHandle,
    quidditch_back_hover: TextureHandle,
    quidditch_team_logos: [[TextureHandle; 3]; 4],
    quidditch_vs: TextureHandle,
    quidditch_vs_small: TextureHandle,
    story_background: Vec<TextureHandle>,
    report_background: Vec<TextureHandle>,
    report_badges: [TextureHandle; 3],
    report_sand: [TextureHandle; 4],
    report_buttons: [[TextureHandle; 2]; 3],
    folio_background: Vec<TextureHandle>,
    folio_harry_background: Vec<TextureHandle>,
    folio_cards: HashMap<i32, CardTextures>,
    folio_missing_big: TextureHandle,
    folio_missing_small: TextureHandle,
    folio_right: TextureHandle,
    folio_right_hover: TextureHandle,
    hud_health_full: TextureHandle,
    hud_health_empty: TextureHandle,
    hud_counters: [TextureHandle; 4],
    hud_bean_piles: [TextureHandle; 4],
    slider_track: TextureHandle,
    slider_knob: TextureHandle,
    checkbox_off: TextureHandle,
    checkbox_on: TextureHandle,
}

struct OptionValues {
    resolution: usize,
    resolutions: Vec<(u32, u32)>,
    brightness: f32,
    mouse_speed: f32,
    music_volume: f32,
    sound_volume: f32,
    texture_detail: usize,
    object_detail: usize,
    auto_jump: bool,
    invert_broom: bool,
}

pub(super) struct GameUi {
    open: bool,
    startup: bool,
    page: Page,
    options_return: Page,
    confirm_exit: bool,
    confirm_quit_game: bool,
    confirm_replace: bool,
    selected_slot: Option<usize>,
    action: Option<Action>,
    save_slots: [bool; 6],
    labels: Labels,
    option_labels: OptionLabels,
    options: OptionValues,
    open_combo: Option<usize>,
    game_root: PathBuf,
    settings_dir: PathBuf,
    quidditch_unlocked: u8,
    quidditch: QuidditchLeague,
    player: PlayerUiState,
    player_seen: bool,
    hud_until: [Option<Instant>; 4],
    folio_page: usize,
    selected_card: Option<i32>,
    story_pages: Vec<StoryPage>,
    story_page: usize,
    story_slot: Option<u32>,
    story_sound_at: Option<Instant>,
    story_deadline: Option<Instant>,
    card_descriptions: HashMap<i32, String>,
    harry_card_objective: String,
    textures: UiTextures,
}

struct OptionLabels {
    title: String,
    video: String,
    controls: String,
    resolution: String,
    color_depth: String,
    texture_detail: String,
    object_detail: String,
    brightness: String,
    mouse_speed: String,
    detail: [String; 5],
    audio: String,
    music_volume: String,
    sound_volume: String,
    keys: [String; 8],
    auto_jump: String,
    invert_broom: String,
    broomstick_practice: String,
    quidditch_league: String,
    quidditch_instructions_title: String,
    quidditch_instructions: String,
    quidditch_round: String,
    quidditch_round_results: String,
    quidditch_wins: String,
    quidditch_losses: String,
    quidditch_points: String,
    quidditch_final: String,
    quidditch_final_results: String,
    quidditch_champion: String,
}

struct Labels {
    start: String,
    options: String,
    quidditch: String,
    exit: String,
    select_game: String,
    new_game: String,
    load_game: String,
    replace_game: String,
    confirm_replace: String,
    confirm_exit: String,
    yes: String,
    no: String,
    quit_game: String,
    resume_game: String,
    folio: String,
    confirm_quit_game: String,
}

impl GameUi {
    pub(super) fn load(
        context: &egui::Context,
        game_root: &Path,
        map: &Path,
        save_dir: &Path,
        options: OptionsState,
    ) -> Result<Self> {
        let mut packages = PackageStore::scan_game_root_with_settings_dir(game_root, save_dir)?;
        let textures = UiTextures {
            main_background: (1..=6)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("MenuArt.MoonTitle{index}"),
                        false,
                    )
                })
                .collect::<Result<_>>()?,
            save_background: (1..=6)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("HPMenu.Icons.FESaveBackTexture{index}"),
                        false,
                    )
                })
                .collect::<Result<_>>()?,
            empty_slot: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.SaveSlotEmptyTexture",
                true,
            )?,
            back: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FELeftReturnUpIcon",
                true,
            )?,
            back_hover: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FELeftReturnOverIcon",
                true,
            )?,
            options_background: (1..=6)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("HPMenu.Icons.FEOptionsBackTexture{index}"),
                        false,
                    )
                })
                .collect::<Result<_>>()?,
            option_bar: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEOverOption3Texture",
                true,
            )?,
            option_bar_open: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEOverOptionTexture",
                true,
            )?,
            combo_list_small: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEComboListSmall",
                true,
            )?,
            combo_list_large: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEComboListLarge",
                true,
            )?,
            combo_list_selection: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEComboListBox",
                true,
            )?,
            quidditch_background: (1..=6)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("HPMenu.Icons.FEQuidBackTexture{index}"),
                        false,
                    )
                })
                .collect::<Result<_>>()?,
            broomstick_practice_locked: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.BroomstickPracticeLockedTexture",
                true,
            )?,
            broomstick_practice: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.BroomstickPracticeTexture",
                true,
            )?,
            quidditch_league_locked: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.QuidLeagueLockedTexture",
                true,
            )?,
            quidditch_league: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.QuidLeagueTexture",
                true,
            )?,
            quidditch_back: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FELeftArrowUpIcon",
                true,
            )?,
            quidditch_back_hover: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FELeftArrowOverIcon",
                true,
            )?,
            quidditch_team_logos: [
                [
                    load_texture(context, &mut packages, "HPMenu.Icons.FEGrifLogoMed", true)?,
                    load_texture(context, &mut packages, "HPMenu.Icons.FEGrifLogoSmall", true)?,
                    load_texture(context, &mut packages, "HPMenu.Icons.FEGrifLogoTiny", true)?,
                ],
                [
                    load_texture(context, &mut packages, "HPMenu.Icons.FERaveLogoMed", true)?,
                    load_texture(context, &mut packages, "HPMenu.Icons.FERaveLogoSmall", true)?,
                    load_texture(context, &mut packages, "HPMenu.Icons.FERaveLogoTiny", true)?,
                ],
                [
                    load_texture(context, &mut packages, "HPMenu.Icons.FEHuffLogoMed", true)?,
                    load_texture(context, &mut packages, "HPMenu.Icons.FEHuffLogoSmall", true)?,
                    load_texture(context, &mut packages, "HPMenu.Icons.FEHuffLogoTiny", true)?,
                ],
                [
                    load_texture(context, &mut packages, "HPMenu.Icons.FESlytLogoMed", true)?,
                    load_texture(context, &mut packages, "HPMenu.Icons.FESlytLogoSmall", true)?,
                    load_texture(context, &mut packages, "HPMenu.Icons.FESlytLogoTiny", true)?,
                ],
            ],
            quidditch_vs: load_texture(context, &mut packages, "HPMenu.Icons.FEVSTexture", true)?,
            quidditch_vs_small: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FESmallVSTexture",
                true,
            )?,
            story_background: (1..=6)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("HPMenu.Icons.HPStoryTextureBackground{index}"),
                        false,
                    )
                })
                .collect::<Result<_>>()?,
            report_background: (1..=6)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("HPMenu.Icons.FEReportBackTexture{index}"),
                        false,
                    )
                })
                .collect::<Result<_>>()?,
            report_badges: [
                load_texture(
                    context,
                    &mut packages,
                    "HPMenu.Icons.BeanBadgeTexture",
                    true,
                )?,
                load_texture(
                    context,
                    &mut packages,
                    "HPMenu.Icons.CardBadgeTexture",
                    true,
                )?,
                load_texture(
                    context,
                    &mut packages,
                    "HPMenu.Icons.PointBadgeTexture",
                    true,
                )?,
            ],
            report_sand: [
                load_texture(
                    context,
                    &mut packages,
                    "HPMenu.Icons.BookReportBlueSand",
                    true,
                )?,
                load_texture(
                    context,
                    &mut packages,
                    "HPMenu.Icons.BookReportYellowSand",
                    true,
                )?,
                load_texture(
                    context,
                    &mut packages,
                    "HPMenu.Icons.BookReportGreenSand",
                    true,
                )?,
                load_texture(
                    context,
                    &mut packages,
                    "HPMenu.Icons.BookReportRedSand",
                    true,
                )?,
            ],
            report_buttons: [
                [
                    load_texture(context, &mut packages, "HPMenu.Icons.BlueUpTexture", true)?,
                    load_texture(context, &mut packages, "HPMenu.Icons.BlueOverTexture", true)?,
                ],
                [
                    load_texture(context, &mut packages, "HPMenu.Icons.GreenUpTexture", true)?,
                    load_texture(
                        context,
                        &mut packages,
                        "HPMenu.Icons.GreenOverTexture",
                        true,
                    )?,
                ],
                [
                    load_texture(context, &mut packages, "HPMenu.Icons.PurpleUpTexture", true)?,
                    load_texture(
                        context,
                        &mut packages,
                        "HPMenu.Icons.PurpleOverTexture",
                        true,
                    )?,
                ],
            ],
            folio_background: (1..=6)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("HPMenu.Icons.FEFolioBackTexture{index}"),
                        false,
                    )
                })
                .collect::<Result<_>>()?,
            folio_harry_background: (1..=6)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("HPMenu.Icons.FEFolioHarryTexture{index}"),
                        false,
                    )
                })
                .collect::<Result<_>>()?,
            folio_cards: WIZARD_CARDS
                .iter()
                .map(|(id, name, _)| {
                    Ok((
                        *id,
                        CardTextures {
                            big: load_texture(
                                context,
                                &mut packages,
                                &format!("HPMenu.Icons.WizCard{name}BigTexture"),
                                true,
                            )?,
                            small: load_texture(
                                context,
                                &mut packages,
                                &format!("HPMenu.Icons.WizCard{name}SmallTexture"),
                                true,
                            )?,
                        },
                    ))
                })
                .collect::<Result<_>>()?,
            folio_missing_big: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.WizCardMissingBigTexture",
                true,
            )?,
            folio_missing_small: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.WizCardMissingSmallTexture",
                true,
            )?,
            folio_right: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FERightArrowUpIcon",
                true,
            )?,
            folio_right_hover: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FERightArrowOverIcon",
                true,
            )?,
            hud_health_full: load_texture(
                context,
                &mut packages,
                "HPBase.Icons.HarryBarFull",
                true,
            )?,
            hud_health_empty: load_texture(
                context,
                &mut packages,
                "HPBase.Icons.HarryBarEmpty",
                true,
            )?,
            hud_counters: [
                load_texture(context, &mut packages, "HPMenu.Icons.FireSeedIcon", true)?,
                load_texture(context, &mut packages, "HPMenu.Icons.StarIcon", true)?,
                load_texture(context, &mut packages, "HPMenu.Icons.pointsIcon", true)?,
                load_texture(context, &mut packages, "HPMenu.Icons.beancounter", true)?,
            ],
            hud_bean_piles: [
                load_texture(context, &mut packages, "HPMenu.Icons.beans1", true)?,
                load_texture(context, &mut packages, "HPMenu.Icons.beans2", true)?,
                load_texture(context, &mut packages, "HPMenu.Icons.beans3", true)?,
                load_texture(context, &mut packages, "HPMenu.Icons.beans4", true)?,
            ],
            slider_track: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEOverSliderTexture",
                true,
            )?,
            slider_knob: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FESliderKnobTexture",
                true,
            )?,
            checkbox_off: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEOptionTickUncheckedTex",
                true,
            )?,
            checkbox_on: load_texture(
                context,
                &mut packages,
                "HPMenu.Icons.FEOptionTickCheckedTex",
                true,
            )?,
            logo: (1..=2)
                .map(|index| {
                    load_texture(
                        context,
                        &mut packages,
                        &format!("MenuArt.Logo{index}"),
                        true,
                    )
                })
                .collect::<Result<_>>()?,
        };
        let mut story_art = HashMap::new();
        for (graphic, _) in NEW_GAME_STORY {
            if !story_art.contains_key(graphic) {
                let art: [TextureHandle; 4] = [1, 2, 3, 4]
                    .map(|piece| {
                        load_texture(
                            context,
                            &mut packages,
                            &format!("StoryBookTest.Default.{graphic}00{piece}"),
                            true,
                        )
                    })
                    .into_iter()
                    .collect::<Result<Vec<_>>>()?
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("new-game story art needs four pieces"))?;
                story_art.insert(graphic, art);
            }
        }
        let story_pages = NEW_GAME_STORY
            .iter()
            .map(|(graphic, dialog)| {
                let art = story_art
                    .get(graphic)
                    .context("new-game story art was not loaded")?
                    .clone();
                let text = packages.localize("HPDialog", "all", dialog);
                if text.is_empty() {
                    bail!("HPDialog.int is missing [all] {dialog}");
                }
                let sound = load_audio_clip(&mut packages, &format!("AllDialog.{dialog}"))?;
                Ok(StoryPage {
                    art,
                    text,
                    duration: wav_duration(sound.data()).unwrap_or(Duration::from_secs(6)),
                    sound,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let localized = |key: &str| -> Result<String> {
            let value = packages.localize("HPMenu", "text", key);
            if value.is_empty() {
                bail!("HPMenu.int is missing [text] {key}");
            }
            Ok(value)
        };
        let labels = Labels {
            start: localized("main_menu_03")?,
            options: localized("main_menu_04")?,
            quidditch: localized("main_menu_05")?,
            exit: localized("main_menu_06")?,
            select_game: localized("select_game_01")?,
            new_game: localized("select_game_02")?,
            load_game: localized("select_game_03")?,
            replace_game: localized("select_game_04")?,
            confirm_replace: localized("select_game_05")?,
            confirm_exit: localized("main_menu_08")?,
            yes: localized("main_menu_09")?,
            no: localized("main_menu_10")?,
            quit_game: localized("report_buttons_04")?,
            resume_game: localized("report_buttons_03")?,
            folio: localized("report_buttons_01")?,
            confirm_quit_game: localized("report_buttons_05")?,
        };
        let pickup = |key: &str| -> Result<String> {
            let value = packages.localize("Pickup", "all", key);
            if value.is_empty() {
                bail!("Pickup.int is missing [all] {key}");
            }
            Ok(value)
        };
        let option_labels = OptionLabels {
            title: localized("options_01")?,
            video: pickup("videoText")?,
            controls: localized("options_16")?,
            resolution: localized("options_02")?,
            color_depth: localized("options_03")?,
            texture_detail: localized("options_05")?,
            object_detail: localized("options_12")?,
            brightness: localized("options_04")?,
            mouse_speed: localized("options_17")?,
            detail: [
                localized("options_06")?,
                localized("options_07")?,
                localized("options_08")?,
                localized("options_10")?,
                localized("options_11")?,
            ],
            audio: localized("options_13")?,
            music_volume: localized("options_14")?,
            sound_volume: localized("options_15")?,
            keys: [
                localized("options_21")?,
                localized("options_22")?,
                localized("options_23")?,
                localized("options_24")?,
                localized("options_25")?,
                localized("options_26")?,
                localized("flying_02")?,
                localized("flying_03")?,
            ],
            auto_jump: pickup("AutoJumpText")?,
            invert_broom: localized("flying_04")?,
            broomstick_practice: localized("quidditch_02")?,
            quidditch_league: localized("quidditch_03")?,
            quidditch_instructions_title: localized("quidditch_07")?,
            quidditch_instructions: localized("quidditch_08")?,
            quidditch_round: localized("quidditch_09")?,
            quidditch_round_results: localized("quidditch_10")?,
            quidditch_wins: localized("quidditch_11")?,
            quidditch_losses: localized("quidditch_12")?,
            quidditch_points: localized("quidditch_13")?,
            quidditch_final: localized("quidditch_14")?,
            quidditch_final_results: localized("quidditch_15")?,
            quidditch_champion: localized("quidditch_16")?,
        };
        let card_descriptions = WIZARD_CARDS
            .iter()
            .map(|(id, _, key)| Ok((*id, localized(key)?)))
            .collect::<Result<_>>()?;
        let harry_card_objective =
            packages.localize("Pickup2", "all", "harry_potter_card_objective");
        let resolutions = option_resolutions(options.resolutions, options.resolution);
        let resolution = resolutions
            .iter()
            .position(|candidate| *candidate == options.resolution)
            .unwrap_or_default();
        let options = OptionValues {
            resolution,
            resolutions,
            brightness: ((options.brightness - 0.2) / 0.8).clamp(0.0, 1.0),
            mouse_speed: 0.5,
            music_volume: options.music_volume,
            sound_volume: options.sound_volume,
            texture_detail: 0,
            object_detail: 0,
            auto_jump: false,
            invert_broom: false,
        };
        let save_slots = std::array::from_fn(|slot| {
            fs::metadata(save_dir.join(format!("save{slot}.usa")))
                .is_ok_and(|metadata| metadata.is_file())
        });
        let quidditch_unlocked = packages
            .config_values("HP", "HPMenu.FEQuidMatchPage", "unlocked")
            .first()
            .and_then(|value| value.parse::<u8>().ok())
            .unwrap_or_default()
            .min(2);
        let startup = is_startup_map(map);
        Ok(Self {
            open: startup,
            startup,
            page: Page::Main,
            options_return: Page::Main,
            confirm_exit: false,
            confirm_quit_game: false,
            confirm_replace: false,
            selected_slot: None,
            action: None,
            save_slots,
            labels,
            option_labels,
            options,
            open_combo: None,
            game_root: game_root.to_path_buf(),
            settings_dir: save_dir.to_path_buf(),
            quidditch_unlocked,
            quidditch: QuidditchLeague::default(),
            player: PlayerUiState::default(),
            player_seen: false,
            hud_until: [None; 4],
            folio_page: 0,
            selected_card: None,
            story_pages,
            story_page: 0,
            story_slot: None,
            story_sound_at: None,
            story_deadline: None,
            card_descriptions,
            harry_card_objective,
            textures,
        })
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn take_action(&mut self) -> Option<Action> {
        self.action.take()
    }

    pub(super) fn open_pause(&mut self) {
        if !self.startup {
            self.open = true;
            self.page = Page::Report;
            self.options_return = Page::Report;
        }
    }

    pub(super) fn escape(&mut self) -> bool {
        self.confirm_exit = false;
        self.confirm_quit_game = false;
        if !self.startup {
            self.open = false;
            return true;
        }
        self.story_deadline = None;
        self.story_sound_at = None;
        self.story_slot = None;
        self.page = Page::Main;
        self.open_combo = None;
        false
    }

    pub(super) fn pauses_game(&self) -> bool {
        self.open && !self.startup
    }

    pub(super) fn set_player_state(&mut self, player: PlayerUiState) {
        if self.player_seen {
            for (index, changed) in changed_hud_counters(self.player, player)
                .into_iter()
                .enumerate()
            {
                if changed {
                    self.hud_until[index] = Some(Instant::now() + Duration::from_secs(5));
                }
            }
        }
        self.player = player;
        self.player_seen = true;
    }

    pub(super) fn unlock_quidditch(&mut self, level: u8) -> Result<()> {
        let level = level.min(2);
        if level <= self.quidditch_unlocked {
            return Ok(());
        }
        PackageStore::scan_game_root_with_settings_dir(&self.game_root, &self.settings_dir)?
            .save_config(
                "HP",
                &[ConfigEntry {
                    section: "HPMenu.FEQuidMatchPage".to_owned(),
                    key: "unlocked".to_owned(),
                    values: vec![level.to_string()],
                }],
            )?;
        self.quidditch_unlocked = level;
        Ok(())
    }

    pub(super) fn preserve_session_from(&mut self, previous: &Self) {
        self.quidditch = previous.quidditch.clone();
    }

    pub(super) fn finish_quidditch_match(&mut self, team0_score: i32, opponent_score: i32) {
        self.quidditch.finish(team0_score, opponent_score);
        self.open = true;
        self.page = Page::Quidditch;
    }

    pub(super) fn ui(&mut self, context: &egui::Context) {
        if !self.open {
            if !self.startup {
                self.hud(context);
            }
            return;
        }
        let screen = context.content_rect();
        let scale = (screen.width() / REFERENCE_SIZE.x)
            .min(screen.height() / REFERENCE_SIZE.y)
            .max(0.01);
        let canvas = Rect::from_center_size(screen.center(), REFERENCE_SIZE * scale);
        let painter = context.layer_painter(LayerId::new(Order::Background, Id::new("game ui")));
        painter.rect_filled(screen, 0.0, Color32::BLACK);
        let painter = painter.with_clip_rect(canvas);
        let background = match self.page {
            Page::Slots => &self.textures.save_background,
            Page::Options => &self.textures.options_background,
            Page::Quidditch => &self.textures.quidditch_background,
            Page::Report => &self.textures.report_background,
            Page::Folio if self.folio_page == 6 => &self.textures.folio_harry_background,
            Page::Folio => &self.textures.folio_background,
            Page::StoryBook => &self.textures.story_background,
            Page::Main => &self.textures.main_background,
        };
        for (index, texture) in background.iter().enumerate() {
            let x = (index % 3) as f32 * 256.0;
            let y = (index / 3) as f32 * 256.0;
            draw_texture(&painter, canvas.min, scale, texture, Pos2::new(x, y));
        }
        if self.page == Page::Main {
            for (index, texture) in self.textures.logo.iter().enumerate() {
                draw_texture(
                    &painter,
                    canvas.min,
                    scale,
                    texture,
                    Pos2::new(74.0 + index as f32 * 256.0, 243.0),
                );
            }
        }

        egui::Area::new(Id::new("game ui controls"))
            .fixed_pos(canvas.min)
            .order(Order::Middle)
            .show(context, |ui| {
                ui.set_min_size(canvas.size());
                match self.page {
                    Page::Main => self.main_page(ui, scale),
                    Page::Slots => self.slot_page(ui, scale),
                    Page::Options => self.options_page(ui, scale),
                    Page::Quidditch => self.quidditch_page(ui, scale),
                    Page::Report => self.report_page(ui, scale),
                    Page::Folio => self.folio_page(ui, scale),
                    Page::StoryBook => self.storybook_page(ui, scale),
                }
                if self.confirm_exit {
                    self.exit_confirmation(ui, scale);
                }
                if self.confirm_replace {
                    self.replace_confirmation(ui, scale);
                }
                if self.confirm_quit_game {
                    self.quit_game_confirmation(ui, scale);
                }
            });
    }

    fn hud(&self, context: &egui::Context) {
        let screen = context.content_rect();
        let scale = (screen.width() / REFERENCE_SIZE.x)
            .min(screen.height() / REFERENCE_SIZE.y)
            .max(0.01);
        let canvas = Rect::from_center_size(screen.center(), REFERENCE_SIZE * scale);
        let painter = context.layer_painter(LayerId::new(Order::Middle, Id::new("game hud")));
        draw_texture(
            &painter,
            canvas.min,
            scale,
            &self.textures.hud_health_full,
            Pos2::ZERO,
        );
        let empty_height = self.textures.hud_health_empty.size_vec2().y
            * (1.0 - self.player.health.clamp(0.0, 1.0));
        if empty_height > 0.0 {
            let texture_height = self.textures.hud_health_empty.size_vec2().y;
            painter.image(
                self.textures.hud_health_empty.id(),
                Rect::from_min_size(
                    canvas.min,
                    Vec2::new(self.textures.hud_health_empty.size_vec2().x, empty_height) * scale,
                ),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, empty_height / texture_height)),
                Color32::WHITE,
            );
        }

        let now = Instant::now();
        let values = [
            self.player.fire_seeds,
            self.player.stars,
            self.player.house_points_harry,
            self.player.beans,
        ];
        for (index, (x, value)) in [160.0, 320.0, 480.0, 480.0]
            .into_iter()
            .zip(values)
            .enumerate()
        {
            if !self.hud_until[index].is_some_and(|until| until > now) {
                continue;
            }
            let texture = &self.textures.hud_counters[index];
            draw_texture(&painter, canvas.min, scale, texture, Pos2::new(x, 4.0));
            painter.text(
                canvas.min + Vec2::new(x, 4.0) * scale + texture.size_vec2() * scale * 0.5,
                Align2::CENTER_CENTER,
                value.to_string(),
                FontId::proportional(14.0 * scale),
                Color32::BLACK,
            );
            if index == 3 {
                let mut remaining = value.min(15) - 3;
                for pile in &self.textures.hud_bean_piles {
                    if remaining <= 0 {
                        break;
                    }
                    draw_texture(&painter, canvas.min, scale, pile, Pos2::new(x, 4.0));
                    remaining -= 3;
                }
            }
        }
    }

    fn main_page(&mut self, ui: &mut egui::Ui, scale: f32) {
        let choices = [
            (self.labels.start.clone(), Page::Slots),
            (self.labels.options.clone(), Page::Options),
            (self.labels.quidditch.clone(), Page::Quidditch),
        ];
        for (index, (label, page)) in choices.into_iter().enumerate() {
            if menu_button(ui, scale, 265.0, 360.0 + index as f32 * 22.0, &label) {
                if page == Page::Options {
                    self.options_return = Page::Main;
                }
                self.page = page;
            }
        }
        if menu_button(ui, scale, 265.0, 426.0, &self.labels.exit) {
            self.confirm_exit = true;
        }
    }

    fn start_new_game_story(&mut self, slot: u32) {
        self.story_page = 0;
        self.story_slot = Some(slot);
        self.page = Page::StoryBook;
        let page = &self.story_pages[0];
        (self.story_sound_at, self.story_deadline) = story_timing(Instant::now(), page.duration);
    }

    fn advance_story(&mut self) {
        self.story_page += 1;
        if let Some(page) = self.story_pages.get(self.story_page) {
            (self.story_sound_at, self.story_deadline) =
                story_timing(Instant::now(), page.duration);
        } else if let Some(slot) = self.story_slot.take() {
            self.story_sound_at = None;
            self.story_deadline = None;
            self.action = Some(Action::NewGame(slot));
        }
    }

    fn storybook_page(&mut self, ui: &mut egui::Ui, scale: f32) {
        let now = Instant::now();
        if self.story_deadline.is_some_and(|deadline| now >= deadline) {
            self.advance_story();
        } else if self.story_sound_at.is_some_and(|sound_at| now >= sound_at) {
            self.story_sound_at = None;
            self.action = Some(Action::PlayUiSound(
                self.story_pages[self.story_page].sound.clone(),
            ));
        }
        let page = &self.story_pages[self.story_page.min(self.story_pages.len() - 1)];
        for (index, texture) in page.art.iter().enumerate() {
            draw_texture(
                ui.painter(),
                ui.min_rect().min,
                scale,
                texture,
                Pos2::new(
                    92.0 + (index % 2) as f32 * 256.0,
                    42.0 + (index / 2) as f32 * 256.0,
                ),
            );
        }
        let galley = ui.painter().layout(
            page.text.clone(),
            FontId::proportional(14.0 * scale),
            Color32::WHITE,
            413.0 * scale,
        );
        ui.painter().galley(
            ui.min_rect().min + Vec2::new(108.0, 381.0) * scale,
            galley,
            Color32::WHITE,
        );
    }

    fn slot_page(&mut self, ui: &mut egui::Ui, scale: f32) {
        page_title(ui, scale, 30.0, &self.labels.select_game, Color32::MAGENTA);
        for slot in 0..6 {
            let row = slot / 3;
            let column = slot % 3;
            let kind = if self.save_slots[slot] {
                &self.labels.load_game
            } else {
                &self.labels.new_game
            };
            let x = 78.0 + column as f32 * 174.0;
            let y = 90.0 + row as f32 * 174.0;
            ui.painter().text(
                ui.min_rect().min + Vec2::new(x, y - 24.0) * scale,
                Align2::LEFT_TOP,
                (slot + 1).to_string(),
                FontId::proportional(16.0 * scale),
                Color32::WHITE,
            );
            if textured_button(
                ui,
                scale,
                x,
                y,
                &self.textures.empty_slot,
                &self.textures.empty_slot,
                kind,
            ) {
                if self.save_slots[slot] {
                    self.selected_slot = Some(slot);
                } else {
                    self.start_new_game_story(slot as u32);
                }
            }
        }
        if let Some(slot) = self.selected_slot {
            if menu_button(ui, scale, 170.0, 408.0, &self.labels.load_game) {
                self.action = Some(Action::LoadSave(slot as u32));
            }
            if menu_button(ui, scale, 300.0, 408.0, &self.labels.replace_game) {
                self.confirm_replace = true;
            }
        }
        if textured_button(
            ui,
            scale,
            565.0,
            431.0,
            &self.textures.back,
            &self.textures.back_hover,
            "",
        ) {
            self.page = Page::Main;
            self.selected_slot = None;
        }
    }

    fn options_page(&mut self, ui: &mut egui::Ui, scale: f32) {
        const PURPLE: Color32 = Color32::from_rgb(96, 0, 96);
        const BLUE: Color32 = Color32::from_rgb(20, 60, 210);
        page_title(ui, scale, 27.0, &self.option_labels.title, PURPLE);
        option_text(ui, scale, 212.0, 59.0, &self.option_labels.video, BLUE);
        option_text(ui, scale, 374.0, 59.0, &self.option_labels.controls, BLUE);

        let left_rows = [87.0, 125.0, 165.0, 201.0];
        let left_labels = [
            &self.option_labels.resolution,
            &self.option_labels.color_depth,
            &self.option_labels.texture_detail,
            &self.option_labels.object_detail,
        ];
        for (y, label) in left_rows.into_iter().zip(left_labels) {
            option_label(ui, scale, 45.0, y, label, PURPLE);
        }

        let resolution = self.options.resolutions[self.options.resolution];
        if option_button(
            ui,
            scale,
            159.0,
            left_rows[0],
            &self.textures.option_bar,
            &format!("{}x{}", resolution.0, resolution.1),
        ) {
            self.open_combo = (self.open_combo != Some(0)).then_some(0);
        }
        if option_button(
            ui,
            scale,
            159.0,
            left_rows[1],
            &self.textures.option_bar,
            "32 Bit",
        ) {
            self.open_combo = (self.open_combo != Some(1)).then_some(1);
        }
        if option_button(
            ui,
            scale,
            159.0,
            left_rows[2],
            &self.textures.option_bar,
            &self.option_labels.detail[self.options.texture_detail + 1],
        ) {
            self.open_combo = (self.open_combo != Some(2)).then_some(2);
        }
        if option_button(
            ui,
            scale,
            159.0,
            left_rows[3],
            &self.textures.option_bar,
            &self.option_labels.detail[self.options.object_detail],
        ) {
            self.open_combo = (self.open_combo != Some(3)).then_some(3);
        }

        option_label(
            ui,
            scale,
            45.0,
            238.0,
            &self.option_labels.brightness,
            PURPLE,
        );
        if option_slider(
            ui,
            scale,
            159.0,
            238.0,
            &self.textures.slider_track,
            &self.textures.slider_knob,
            &mut self.options.brightness,
        ) {
            self.action = Some(Action::SetBrightness(0.2 + self.options.brightness * 0.8));
        }
        option_label(
            ui,
            scale,
            45.0,
            271.0,
            &self.option_labels.mouse_speed,
            PURPLE,
        );
        option_slider(
            ui,
            scale,
            159.0,
            271.0,
            &self.textures.slider_track,
            &self.textures.slider_knob,
            &mut self.options.mouse_speed,
        );
        option_text(
            ui,
            scale,
            159.0,
            298.0,
            &self.option_labels.detail[3],
            PURPLE,
        );
        option_text(
            ui,
            scale,
            293.0,
            298.0,
            &self.option_labels.detail[1],
            PURPLE,
        );
        option_text(ui, scale, 212.0, 326.0, &self.option_labels.audio, BLUE);
        option_label(
            ui,
            scale,
            45.0,
            352.0,
            &self.option_labels.music_volume,
            PURPLE,
        );
        if option_slider(
            ui,
            scale,
            159.0,
            352.0,
            &self.textures.slider_track,
            &self.textures.slider_knob,
            &mut self.options.music_volume,
        ) {
            self.action = Some(Action::SetMusicVolume(
                (self.options.music_volume * 255.0).round() as u8,
            ));
        }
        option_label(
            ui,
            scale,
            45.0,
            389.0,
            &self.option_labels.sound_volume,
            PURPLE,
        );
        if option_slider(
            ui,
            scale,
            159.0,
            389.0,
            &self.textures.slider_track,
            &self.textures.slider_knob,
            &mut self.options.sound_volume,
        ) {
            self.action = Some(Action::SetSoundVolume(
                (self.options.sound_volume * 255.0).round() as u8,
            ));
        }

        let key_values = [
            "W or Up",
            "S or Down",
            "A or Left",
            "D or Right",
            "Space or Right Mouse",
            "Left Mouse or Alt",
            "Z",
            "X",
        ];
        let key_rows = [87.0, 125.0, 165.0, 201.0, 239.0, 279.0, 317.0, 357.0];
        for ((label, value), y) in self.option_labels.keys.iter().zip(key_values).zip(key_rows) {
            let _ = option_button(ui, scale, 329.0, y, &self.textures.option_bar, value);
            option_label(ui, scale, 484.0, y, label, PURPLE);
        }
        if option_checkbox(
            ui,
            scale,
            329.0,
            397.0,
            &self.textures.checkbox_off,
            &self.textures.checkbox_on,
            &self.option_labels.auto_jump,
            self.options.auto_jump,
        ) {
            self.options.auto_jump = !self.options.auto_jump;
        }
        if option_checkbox(
            ui,
            scale,
            329.0,
            417.0,
            &self.textures.checkbox_off,
            &self.textures.checkbox_on,
            &self.option_labels.invert_broom,
            self.options.invert_broom,
        ) {
            self.options.invert_broom = !self.options.invert_broom;
        }
        if textured_button(
            ui,
            scale,
            565.0,
            431.0,
            &self.textures.back,
            &self.textures.back_hover,
            "",
        ) {
            self.page = self.options_return;
            self.open_combo = None;
        }

        if let Some(combo) = self.open_combo {
            let (items, selected) = match combo {
                0 => (
                    self.options
                        .resolutions
                        .iter()
                        .map(|(width, height)| format!("{width}x{height}"))
                        .collect::<Vec<_>>(),
                    self.options.resolution,
                ),
                1 => (vec!["32 Bit".to_owned()], 0),
                2 => (
                    self.option_labels.detail[1..=3].to_vec(),
                    self.options.texture_detail,
                ),
                3 => (
                    self.option_labels.detail.to_vec(),
                    self.options.object_detail,
                ),
                _ => unreachable!(),
            };
            if let Some(selection) = option_combo_list(
                ui,
                scale,
                159.0,
                left_rows[combo] + 18.0,
                &self.textures,
                &items,
                selected,
            ) {
                match combo {
                    0 => {
                        self.options.resolution = selection;
                        let (width, height) = self.options.resolutions[selection];
                        self.action = Some(Action::SetResolution(width, height));
                    }
                    1 => {}
                    2 => self.options.texture_detail = selection,
                    3 => self.options.object_detail = selection,
                    _ => unreachable!(),
                }
                self.open_combo = None;
            }
        }
    }

    fn quidditch_page(&mut self, ui: &mut egui::Ui, scale: f32) {
        match self.quidditch.screen {
            QuidditchScreen::Start => self.quidditch_start(ui, scale),
            QuidditchScreen::Instructions => {
                page_title(
                    ui,
                    scale,
                    85.0,
                    &self.option_labels.quidditch_instructions_title,
                    Color32::WHITE,
                );
                let galley = ui.painter().layout(
                    self.option_labels.quidditch_instructions.clone(),
                    FontId::proportional(16.0 * scale),
                    Color32::WHITE,
                    400.0 * scale,
                );
                ui.painter().galley(
                    ui.min_rect().min + Vec2::new(120.0, 130.0) * scale,
                    galley,
                    Color32::WHITE,
                );
            }
            QuidditchScreen::Matchup => {
                let title = if self.quidditch.finals {
                    self.option_labels.quidditch_final.clone()
                } else {
                    self.option_labels.quidditch_round.replacen(
                        '#',
                        &(self.quidditch.current_game + 1).to_string(),
                        1,
                    )
                };
                page_title(ui, scale, 85.0, &title, Color32::WHITE);
                self.draw_quidditch_matchup(ui, scale);
            }
            QuidditchScreen::Results => {
                let title = format!(
                    "{} {}",
                    self.option_labels.quidditch_round_results,
                    self.quidditch.current_game + 1
                );
                page_title(ui, scale, 85.0, &title, Color32::WHITE);
                self.draw_quidditch_results(ui, scale);
            }
            QuidditchScreen::FinalResults => {
                page_title(
                    ui,
                    scale,
                    85.0,
                    &self.option_labels.quidditch_final_results,
                    Color32::WHITE,
                );
                let fixture = self.quidditch.fixture();
                let score = self.quidditch.scores[self.quidditch.current_game];
                let winner = if score.home > score.visitor {
                    fixture.home
                } else {
                    fixture.visitor
                };
                draw_centered_texture(
                    ui.painter(),
                    ui.min_rect().min,
                    scale,
                    &self.textures.quidditch_team_logos[winner][0],
                    320.0,
                    175.0,
                );
                page_title(
                    ui,
                    scale,
                    240.0,
                    &self.option_labels.quidditch_champion,
                    Color32::WHITE,
                );
                self.draw_quidditch_standings(ui, scale);
            }
        }

        let screen = self.quidditch.screen;
        if screen != QuidditchScreen::Results && screen != QuidditchScreen::FinalResults {
            if textured_button(
                ui,
                scale,
                4.0,
                436.0,
                &self.textures.quidditch_back,
                &self.textures.quidditch_back_hover,
                "",
            ) {
                self.quidditch.screen = match screen {
                    QuidditchScreen::Start => {
                        self.page = Page::Main;
                        QuidditchScreen::Start
                    }
                    QuidditchScreen::Instructions => QuidditchScreen::Start,
                    QuidditchScreen::Matchup if self.quidditch.current_game == 0 => {
                        QuidditchScreen::Start
                    }
                    QuidditchScreen::Matchup => QuidditchScreen::Results,
                    _ => screen,
                };
            }
        }
        if screen != QuidditchScreen::Start {
            let forward_clicked = textured_button(
                ui,
                scale,
                572.0,
                436.0,
                &self.textures.folio_right,
                &self.textures.folio_right_hover,
                "",
            );
            if forward_clicked {
                match screen {
                    QuidditchScreen::Instructions | QuidditchScreen::Results => {
                        self.quidditch.screen = QuidditchScreen::Matchup;
                    }
                    QuidditchScreen::Matchup => {
                        let level = self.quidditch.fixture().level;
                        if level.is_empty() {
                            self.finish_quidditch_match(0, 0);
                        } else {
                            self.action = Some(Action::LoadLevel(level.to_owned()));
                        }
                    }
                    QuidditchScreen::FinalResults => {
                        self.quidditch.screen = QuidditchScreen::Start;
                    }
                    QuidditchScreen::Start => {}
                }
            }
        }
    }

    fn quidditch_start(&mut self, ui: &mut egui::Ui, scale: f32) {
        let practice = if self.quidditch_unlocked >= 1 {
            &self.textures.broomstick_practice
        } else {
            &self.textures.broomstick_practice_locked
        };
        let practice_size = practice.size_vec2();
        if textured_button(
            ui,
            scale,
            192.0 - practice_size.x * 0.5,
            160.0,
            practice,
            practice,
            "",
        ) && self.quidditch_unlocked >= 1
        {
            self.action = Some(Action::LoadLevel("Lev_Tut2.unr".to_owned()));
        }
        let league = if self.quidditch_unlocked >= 2 {
            &self.textures.quidditch_league
        } else {
            &self.textures.quidditch_league_locked
        };
        let league_size = league.size_vec2();
        if textured_button(
            ui,
            scale,
            448.0 - league_size.x * 0.5,
            160.0,
            league,
            league,
            "",
        ) && self.quidditch_unlocked >= 2
        {
            self.quidditch.restart();
        }
        for (x, label) in [
            (192.0, &self.option_labels.broomstick_practice),
            (448.0, &self.option_labels.quidditch_league),
        ] {
            ui.painter().text(
                ui.min_rect().min + Vec2::new(x, 290.0) * scale,
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(18.0 * scale),
                Color32::WHITE,
            );
        }
    }

    fn draw_quidditch_matchup(&self, ui: &egui::Ui, scale: f32) {
        let fixture = self.quidditch.fixture();
        for (team, x) in [(fixture.home, 192.0), (fixture.visitor, 448.0)] {
            draw_centered_texture(
                ui.painter(),
                ui.min_rect().min,
                scale,
                &self.textures.quidditch_team_logos[team][0],
                x,
                185.0,
            );
        }
        draw_centered_texture(
            ui.painter(),
            ui.min_rect().min,
            scale,
            &self.textures.quidditch_vs,
            320.0,
            185.0,
        );
        if !self.quidditch.finals {
            for (team, x) in [(fixture.other_home, 192.0), (fixture.other_visitor, 448.0)] {
                draw_centered_texture(
                    ui.painter(),
                    ui.min_rect().min,
                    scale,
                    &self.textures.quidditch_team_logos[team][0],
                    x,
                    335.0,
                );
            }
            draw_centered_texture(
                ui.painter(),
                ui.min_rect().min,
                scale,
                &self.textures.quidditch_vs,
                320.0,
                335.0,
            );
        }
    }

    fn draw_quidditch_results(&mut self, ui: &egui::Ui, scale: f32) {
        let game = self.quidditch.current_game - 1;
        let fixture = QUIDDITCH_FIXTURES[game];
        let score = self.quidditch.scores[game];
        for (team, x, value) in [
            (fixture.home, 192.0, score.home),
            (fixture.visitor, 448.0, score.visitor),
            (fixture.other_home, 192.0, score.other_home),
            (fixture.other_visitor, 448.0, score.other_visitor),
        ] {
            let y = if team == fixture.home || team == fixture.visitor {
                140.0
            } else {
                200.0
            };
            draw_centered_texture(
                ui.painter(),
                ui.min_rect().min,
                scale,
                &self.textures.quidditch_team_logos[team][1],
                x,
                y,
            );
            ui.painter().text(
                ui.min_rect().min + Vec2::new(if x < 320.0 { 246.0 } else { 394.0 }, y) * scale,
                Align2::CENTER_CENTER,
                value,
                FontId::proportional(16.0 * scale),
                Color32::WHITE,
            );
        }
        for y in [140.0, 200.0] {
            draw_centered_texture(
                ui.painter(),
                ui.min_rect().min,
                scale,
                &self.textures.quidditch_vs_small,
                320.0,
                y,
            );
        }
        self.draw_quidditch_standings(ui, scale);
    }

    fn draw_quidditch_standings(&mut self, ui: &egui::Ui, scale: f32) {
        let standings = self.quidditch.sort_teams();
        let origin = ui.min_rect().min;
        ui.painter().line_segment(
            [
                origin + Vec2::new(250.0, 248.0) * scale,
                origin + Vec2::new(490.0, 248.0) * scale,
            ],
            egui::Stroke::new(scale, Color32::WHITE),
        );
        for (x, label) in [
            (370.0, &self.option_labels.quidditch_wins),
            (410.0, &self.option_labels.quidditch_losses),
            (450.0, &self.option_labels.quidditch_points),
        ] {
            ui.painter().text(
                origin + Vec2::new(x, 245.0) * scale,
                Align2::LEFT_TOP,
                label,
                FontId::proportional(12.0 * scale),
                Color32::WHITE,
            );
        }
        for (row, team) in standings.into_iter().enumerate() {
            draw_centered_texture(
                ui.painter(),
                origin,
                scale,
                &self.textures.quidditch_team_logos[team][2],
                192.0,
                285.0 + row as f32 * 35.0,
            );
            for (x, value) in [
                (370.0, self.quidditch.teams[team].wins),
                (410.0, self.quidditch.teams[team].losses),
                (450.0, self.quidditch.teams[team].points),
            ] {
                ui.painter().text(
                    origin + Vec2::new(x, 270.0 + row as f32 * 35.0) * scale,
                    Align2::LEFT_TOP,
                    value,
                    FontId::proportional(12.0 * scale),
                    Color32::WHITE,
                );
            }
        }
    }

    fn report_page(&mut self, ui: &mut egui::Ui, scale: f32) {
        let origin = ui.min_rect().min;
        let points = [
            self.player.house_points_ravenclaw,
            self.player.house_points_hufflepuff,
            self.player.house_points_slytherin,
            self.player.house_points_gryffindor,
        ];
        for (index, points) in points.into_iter().enumerate() {
            let fraction = if self.player.max_points_per_house > 0 {
                (points as f32 / self.player.max_points_per_house as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            draw_report_sand(
                ui.painter(),
                origin,
                scale,
                &self.textures.report_sand[index],
                [87.0, 151.0, 215.0, 280.0][index],
                125.0,
                fraction,
            );
            ui.painter().text(
                origin + Vec2::new([206.0, 270.0, 334.0, 399.0][index], 227.0) * scale,
                Align2::LEFT_TOP,
                points.to_string(),
                FontId::proportional(10.0 * scale),
                Color32::BLACK,
            );
        }
        for (texture, x, text) in [
            (
                &self.textures.report_badges[0],
                31.0,
                self.player.beans.to_string(),
            ),
            (
                &self.textures.report_badges[1],
                246.0,
                format!("{}/25", self.player.cards),
            ),
            (
                &self.textures.report_badges[2],
                466.0,
                self.player.house_points_harry.to_string(),
            ),
        ] {
            draw_texture(ui.painter(), origin, scale, texture, Pos2::new(x, 62.0));
            ui.painter().text(
                origin + Vec2::new(x + 65.0, 141.0) * scale,
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(18.0 * scale),
                Color32::BLACK,
            );
        }
        let buttons = [
            (107.0, 0, self.labels.quit_game.clone()),
            (286.0, 1, self.labels.options.clone()),
            (450.0, 2, self.labels.folio.clone()),
        ];
        for (x, kind, label) in buttons {
            if textured_button(
                ui,
                scale,
                x,
                354.0,
                &self.textures.report_buttons[kind][0],
                &self.textures.report_buttons[kind][1],
                "",
            ) {
                match kind {
                    0 => self.confirm_quit_game = true,
                    1 => {
                        self.options_return = Page::Report;
                        self.page = Page::Options;
                    }
                    2 => {
                        self.folio_page = 0;
                        self.selected_card = None;
                        self.page = Page::Folio;
                    }
                    _ => unreachable!(),
                }
            }
            ui.painter().text(
                origin + Vec2::new(x + 32.0, 426.0) * scale,
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(16.0 * scale),
                Color32::from_rgb(215, 0, 215),
            );
        }
        let card_badge = scaled_rect(origin, scale, 246.0, 62.0, 128.0, 128.0);
        if ui
            .interact(card_badge, Id::new("report card badge"), Sense::click())
            .clicked()
        {
            self.folio_page = 0;
            self.selected_card = None;
            self.page = Page::Folio;
        }
        let resume = scaled_rect(origin, scale, 485.0, 0.0, 100.0, 120.0);
        let response = ui.interact(resume, Id::new("resume game"), Sense::click());
        ui.painter().text(
            resume.center(),
            Align2::CENTER_CENTER,
            &self.labels.resume_game,
            FontId::proportional(16.0 * scale),
            if response.hovered() {
                Color32::WHITE
            } else {
                Color32::from_rgb(215, 0, 215)
            },
        );
        if response.clicked() {
            self.open = false;
            self.action = Some(Action::Resume);
        }
    }

    fn folio_page(&mut self, ui: &mut egui::Ui, scale: f32) {
        let origin = ui.min_rect().min;
        page_title(
            ui,
            scale,
            14.0,
            &self.labels.folio,
            Color32::from_rgb(96, 0, 96),
        );
        let harry_page = self.folio_page == 6;
        let selected = if harry_page {
            self.player.wizard_cards[24]
        } else {
            self.selected_card
        };
        let big = selected
            .and_then(|id| self.textures.folio_cards.get(&id))
            .map_or(&self.textures.folio_missing_big, |card| &card.big);
        draw_texture(ui.painter(), origin, scale, big, Pos2::new(182.0, 31.0));

        if !harry_page {
            for (index, (x, y)) in [(49.0, 130.0), (49.0, 268.0), (449.0, 131.0), (451.0, 268.0)]
                .into_iter()
                .enumerate()
            {
                let card = self.player.wizard_cards[self.folio_page * 4 + index];
                let texture = card
                    .and_then(|id| self.textures.folio_cards.get(&id))
                    .map_or(&self.textures.folio_missing_small, |card| &card.small);
                if textured_button(ui, scale, x, y, texture, texture, "") {
                    self.selected_card = card;
                }
            }
        }

        let description = selected
            .and_then(|id| self.card_descriptions.get(&id))
            .cloned()
            .unwrap_or_else(|| {
                harry_page
                    .then(|| self.harry_card_objective.clone())
                    .unwrap_or_default()
            });
        if !description.is_empty() {
            let galley = ui.painter().layout(
                description,
                FontId::proportional(14.0 * scale),
                Color32::from_rgb(96, 0, 96),
                210.0 * scale,
            );
            ui.painter().galley(
                origin + Vec2::new(211.0, 315.0) * scale,
                galley,
                Color32::from_rgb(96, 0, 96),
            );
        }

        if self.folio_page > 0
            && textured_button(
                ui,
                scale,
                80.0,
                400.0,
                &self.textures.quidditch_back,
                &self.textures.quidditch_back_hover,
                "",
            )
        {
            self.folio_page -= 1;
        }
        if self.folio_page < 6
            && textured_button(
                ui,
                scale,
                485.0,
                400.0,
                &self.textures.folio_right,
                &self.textures.folio_right_hover,
                "",
            )
        {
            self.folio_page += 1;
        }
        ui.painter().text(
            origin + Vec2::new(305.0, 440.0) * scale,
            Align2::CENTER_CENTER,
            format!("{} / 7", self.folio_page + 1),
            FontId::proportional(14.0 * scale),
            Color32::from_rgb(215, 0, 215),
        );
        if textured_button(
            ui,
            scale,
            565.0,
            431.0,
            &self.textures.back,
            &self.textures.back_hover,
            "",
        ) {
            self.page = Page::Report;
        }
    }

    fn exit_confirmation(&mut self, ui: &mut egui::Ui, scale: f32) {
        confirmation_panel(ui, scale, &self.labels.confirm_exit);
        if menu_button(ui, scale, 205.0, 245.0, &self.labels.yes) {
            self.action = Some(Action::Exit);
        }
        if menu_button(ui, scale, 335.0, 245.0, &self.labels.no) {
            self.confirm_exit = false;
        }
    }

    fn replace_confirmation(&mut self, ui: &mut egui::Ui, scale: f32) {
        confirmation_panel(ui, scale, &self.labels.confirm_replace);
        if menu_button(ui, scale, 205.0, 245.0, &self.labels.yes) {
            if let Some(slot) = self.selected_slot {
                self.start_new_game_story(slot as u32);
            }
            self.confirm_replace = false;
        }
        if menu_button(ui, scale, 335.0, 245.0, &self.labels.no) {
            self.confirm_replace = false;
        }
    }

    fn quit_game_confirmation(&mut self, ui: &mut egui::Ui, scale: f32) {
        confirmation_panel(ui, scale, &self.labels.confirm_quit_game);
        if menu_button(ui, scale, 205.0, 245.0, &self.labels.yes) {
            self.action = Some(Action::LoadLevel("startup.unr".to_owned()));
            self.confirm_quit_game = false;
        }
        if menu_button(ui, scale, 335.0, 245.0, &self.labels.no) {
            self.confirm_quit_game = false;
        }
    }
}

fn confirmation_panel(ui: &egui::Ui, scale: f32, text: &str) {
    let origin = ui.min_rect().min;
    let rect = scaled_rect(origin, scale, 140.0, 170.0, 360.0, 130.0);
    ui.painter()
        .rect_filled(rect, 6.0 * scale, Color32::from_black_alpha(225));
    ui.painter().text(
        origin + Vec2::new(320.0, 205.0) * scale,
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(18.0 * scale),
        Color32::WHITE,
    );
}

fn load_texture(
    context: &egui::Context,
    packages: &mut PackageStore,
    name: &str,
    masked: bool,
) -> Result<TextureHandle> {
    let ResolvedObject {
        package,
        export_index,
    } = packages
        .find_localized_object(name, "Texture")?
        .with_context(|| format!("shipped UI texture {name} is missing"))?;
    let texture = Texture::decode(&package, export_index)?;
    let ObjectReference::Export(palette_index) = texture.palette else {
        bail!("shipped UI texture {name} has a non-local palette");
    };
    let palette = Palette::decode(&package, palette_index)?;
    let mip = texture
        .mips
        .first()
        .context("shipped UI texture has no mip")?;
    let rgba = texture.rgba(0, &palette, masked)?;
    let image =
        egui::ColorImage::from_rgba_unmultiplied([mip.width as usize, mip.height as usize], &rgba);
    Ok(context.load_texture(name, image, egui::TextureOptions::NEAREST))
}

fn load_audio_clip(packages: &mut PackageStore, name: &str) -> Result<AudioClip> {
    let ResolvedObject {
        package,
        export_index,
    } = packages
        .find_localized_object(name, "Sound")?
        .with_context(|| format!("shipped story sound {name} is missing"))?;
    Ok(AudioClip::decode(&package, export_index)?)
}

fn wav_duration(bytes: &[u8]) -> Option<Duration> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut position = 12;
    let mut byte_rate = None;
    let mut data_size = None;
    while position + 8 <= bytes.len() {
        let size =
            u32::from_le_bytes(bytes.get(position + 4..position + 8)?.try_into().ok()?) as usize;
        let value = position + 8;
        let end = value.checked_add(size)?;
        if end > bytes.len() {
            return None;
        }
        match bytes.get(position..position + 4)? {
            b"fmt " if size >= 12 => {
                byte_rate = Some(u32::from_le_bytes(
                    bytes.get(value + 8..value + 12)?.try_into().ok()?,
                ));
            }
            b"data" => data_size = Some(size as u64),
            _ => {}
        }
        position = end + (size & 1);
    }
    let byte_rate = u64::from(byte_rate?.max(1));
    Some(Duration::from_secs_f64(
        data_size? as f64 / byte_rate as f64,
    ))
}

fn story_timing(now: Instant, narration: Duration) -> (Option<Instant>, Option<Instant>) {
    (
        Some(now + STORY_MIN_TIME_ON_PAGE - STORY_SOUND_ADVANCE),
        Some(now + STORY_MIN_TIME_ON_PAGE + narration + STORY_TRAILING_TIME),
    )
}

fn draw_texture(
    painter: &egui::Painter,
    origin: Pos2,
    scale: f32,
    texture: &TextureHandle,
    position: Pos2,
) {
    let size = texture.size_vec2() * scale;
    let position = origin + position.to_vec2() * scale;
    painter.image(
        texture.id(),
        Rect::from_min_size(position, size),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn draw_centered_texture(
    painter: &egui::Painter,
    origin: Pos2,
    scale: f32,
    texture: &TextureHandle,
    x: f32,
    y: f32,
) {
    let size = texture.size_vec2();
    draw_texture(
        painter,
        origin,
        scale,
        texture,
        Pos2::new(x - size.x * 0.5, y - size.y * 0.5),
    );
}

fn draw_report_sand(
    painter: &egui::Painter,
    origin: Pos2,
    scale: f32,
    texture: &TextureHandle,
    x: f32,
    y: f32,
    fraction: f32,
) {
    let height = 256.0 * fraction;
    if height <= 0.0 {
        return;
    }
    painter.image(
        texture.id(),
        Rect::from_min_size(
            origin + Vec2::new(x, y + 256.0 - height) * scale,
            Vec2::new(256.0, height) * scale,
        ),
        Rect::from_min_max(Pos2::new(0.0, 1.0 - fraction), Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn page_title(ui: &egui::Ui, scale: f32, y: f32, text: &str, color: Color32) {
    ui.painter().text(
        ui.min_rect().min + Vec2::new(320.0, y) * scale,
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(18.0 * scale),
        color,
    );
}

fn menu_button(ui: &mut egui::Ui, scale: f32, x: f32, y: f32, text: &str) -> bool {
    let rect = scaled_rect(ui.min_rect().min, scale, x, y, 140.0, 25.0);
    let response = ui.interact(
        rect,
        Id::new((text, x.to_bits(), y.to_bits())),
        Sense::click(),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(16.0 * scale),
        if response.hovered() {
            Color32::RED
        } else {
            Color32::WHITE
        },
    );
    response.clicked()
}

fn option_text(ui: &egui::Ui, scale: f32, x: f32, y: f32, text: &str, color: Color32) {
    ui.painter().text(
        ui.min_rect().min + Vec2::new(x, y) * scale,
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(10.0 * scale),
        color,
    );
}

fn option_label(ui: &egui::Ui, scale: f32, x: f32, y: f32, text: &str, color: Color32) {
    ui.painter().text(
        ui.min_rect().min + Vec2::new(x, y + 8.0) * scale,
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(10.0 * scale),
        color,
    );
}

fn option_button(
    ui: &mut egui::Ui,
    scale: f32,
    x: f32,
    y: f32,
    texture: &TextureHandle,
    text: &str,
) -> bool {
    let rect = scaled_rect(ui.min_rect().min, scale, x, y, 134.0, 18.0);
    let response = ui.interact(
        rect,
        Id::new(("option", x.to_bits(), y.to_bits())),
        Sense::click(),
    );
    ui.painter().image(
        texture.id(),
        rect,
        texture_uv(texture, Vec2::new(134.0, 18.0)),
        Color32::WHITE,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(10.0 * scale),
        Color32::BLACK,
    );
    response.clicked()
}

fn option_combo_list(
    ui: &mut egui::Ui,
    scale: f32,
    x: f32,
    y: f32,
    textures: &UiTextures,
    items: &[String],
    selected: usize,
) -> Option<usize> {
    let origin = ui.min_rect().min;
    let bar = scaled_rect(origin, scale, x, y - 18.0, 134.0, 18.0);
    ui.painter().image(
        textures.option_bar_open.id(),
        bar,
        texture_uv(&textures.option_bar_open, Vec2::new(134.0, 18.0)),
        Color32::WHITE,
    );
    ui.painter().text(
        bar.center(),
        Align2::CENTER_CENTER,
        &items[selected],
        FontId::proportional(10.0 * scale),
        Color32::BLACK,
    );

    let height = combo_list_height(items.len());
    let background = if items.len() > 3 {
        &textures.combo_list_large
    } else {
        &textures.combo_list_small
    };
    let popup = scaled_rect(origin, scale, x, y, 147.0, height);
    ui.painter().image(
        background.id(),
        popup,
        texture_uv(background, Vec2::new(147.0, height)),
        Color32::WHITE,
    );
    let item_height = (height - 7.0) / items.len() as f32;
    let responses = items
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let rect = scaled_rect(
                origin,
                scale,
                x,
                y + 3.0 + item_height * index as f32,
                147.0,
                item_height,
            );
            ui.interact(
                rect,
                Id::new(("option combo", x.to_bits(), y.to_bits(), index)),
                Sense::click(),
            )
        })
        .collect::<Vec<_>>();
    let hovered = responses.iter().position(egui::Response::hovered);
    for (index, (item, response)) in items.iter().zip(&responses).enumerate() {
        if hovered == Some(index) || (hovered.is_none() && selected == index) {
            ui.painter().image(
                textures.combo_list_selection.id(),
                response.rect,
                texture_uv(
                    &textures.combo_list_selection,
                    textures.combo_list_selection.size_vec2(),
                ),
                Color32::WHITE,
            );
        }
        ui.painter().text(
            response.rect.min + Vec2::new(12.0, item_height * 0.5) * scale,
            Align2::LEFT_CENTER,
            item,
            FontId::proportional(10.0 * scale),
            Color32::BLACK,
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            return Some(index);
        }
    }
    None
}

fn combo_list_height(item_count: usize) -> f32 {
    if item_count > 3 { 89.0 } else { 54.0 }
}

fn option_resolutions(available: Vec<(u32, u32)>, current: (u32, u32)) -> Vec<(u32, u32)> {
    let mut resolutions = available
        .into_iter()
        .filter(|resolution| AUTHORED_RESOLUTIONS.contains(resolution))
        .collect::<Vec<_>>();
    resolutions.push(current);
    resolutions.sort_unstable();
    resolutions.dedup();
    resolutions
}

fn changed_hud_counters(previous: PlayerUiState, current: PlayerUiState) -> [bool; 4] {
    let values = |player: PlayerUiState| {
        [
            player.fire_seeds,
            player.stars,
            player.house_points_harry,
            player.beans,
        ]
    };
    let previous = values(previous);
    let current = values(current);
    std::array::from_fn(|index| previous[index] != current[index])
}

fn option_slider(
    ui: &mut egui::Ui,
    scale: f32,
    x: f32,
    y: f32,
    track: &TextureHandle,
    knob: &TextureHandle,
    value: &mut f32,
) -> bool {
    let origin = ui.min_rect().min;
    let rect = scaled_rect(origin, scale, x, y, 134.0, 25.0);
    let response = ui.interact(
        rect,
        Id::new(("slider", x.to_bits(), y.to_bits())),
        Sense::click_and_drag(),
    );
    let changed = (response.clicked() || response.dragged())
        && response.interact_pointer_pos().is_some_and(|pointer| {
            let next = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let changed = (*value - next).abs() > f32::EPSILON;
            *value = next;
            changed
        });
    let track_rect = Rect::from_min_size(
        origin + Vec2::new(x, y + 8.0) * scale,
        Vec2::new(134.0, 9.0) * scale,
    );
    if response.hovered() {
        ui.painter().image(
            track.id(),
            track_rect,
            texture_uv(track, Vec2::new(134.0, 9.0)),
            Color32::WHITE,
        );
    }
    let knob_width = 9.0 * scale;
    let knob_position = Pos2::new(
        rect.left() + (rect.width() - knob_width) * *value,
        rect.top(),
    );
    ui.painter().image(
        knob.id(),
        Rect::from_min_size(knob_position, Vec2::new(9.0, 25.0) * scale),
        texture_uv(knob, Vec2::new(9.0, 25.0)),
        Color32::WHITE,
    );
    changed
}

fn option_checkbox(
    ui: &mut egui::Ui,
    scale: f32,
    x: f32,
    y: f32,
    off: &TextureHandle,
    on: &TextureHandle,
    text: &str,
    checked: bool,
) -> bool {
    let texture = if checked { on } else { off };
    let rect = scaled_rect(ui.min_rect().min, scale, x, y, 160.0, 18.0);
    let response = ui.interact(
        rect,
        Id::new(("check", x.to_bits(), y.to_bits())),
        Sense::click(),
    );
    ui.painter().image(
        texture.id(),
        Rect::from_min_size(rect.min, Vec2::new(12.0, 12.0) * scale),
        texture_uv(texture, Vec2::new(12.0, 12.0)),
        Color32::WHITE,
    );
    ui.painter().text(
        rect.min + Vec2::new(17.0, 8.0) * scale,
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(10.0 * scale),
        Color32::from_rgb(96, 0, 96),
    );
    response.clicked()
}

fn texture_uv(texture: &TextureHandle, size: Vec2) -> Rect {
    let texture_size = texture.size_vec2();
    Rect::from_min_max(
        Pos2::ZERO,
        Pos2::new(
            size.x.min(texture_size.x) / texture_size.x,
            size.y.min(texture_size.y) / texture_size.y,
        ),
    )
}

fn textured_button(
    ui: &mut egui::Ui,
    scale: f32,
    x: f32,
    y: f32,
    texture: &TextureHandle,
    hover_texture: &TextureHandle,
    text: &str,
) -> bool {
    let rect = scaled_rect(
        ui.min_rect().min,
        scale,
        x,
        y,
        texture.size_vec2().x,
        texture.size_vec2().y,
    );
    let response = ui.interact(
        rect,
        Id::new(("texture button", x.to_bits(), y.to_bits())),
        Sense::click(),
    );
    let texture = if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        hover_texture
    } else {
        texture
    };
    ui.painter().image(
        texture.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    if !text.is_empty() {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(16.0 * scale),
            if response.hovered() {
                Color32::WHITE
            } else {
                Color32::from_rgb(250, 4, 30)
            },
        );
    }
    response.clicked()
}

fn scaled_rect(origin: Pos2, scale: f32, x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::from_min_size(
        origin + Vec2::new(x, y) * scale,
        Vec2::new(width * scale, height * scale),
    )
}

fn is_startup_map(path: &Path) -> bool {
    path.file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("startup"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontend_only_opens_for_the_authored_startup_map() {
        assert!(is_startup_map(Path::new("game/Maps/STARTUP.unr")));
        assert!(!is_startup_map(Path::new("game/Maps/Entry.unr")));
        assert!(!is_startup_map(Path::new("game/Maps/Lev_Tut1.unr")));
    }

    #[test]
    fn combo_lists_use_the_two_authored_popup_sizes() {
        assert_eq!(combo_list_height(3), 54.0);
        assert_eq!(combo_list_height(4), 89.0);
    }

    #[test]
    fn options_keep_authored_resolutions_plus_the_active_window() {
        assert_eq!(
            option_resolutions(
                vec![(3840, 2160), (800, 600), (640, 480), (1920, 1080)],
                (2560, 1600),
            ),
            vec![(640, 480), (800, 600), (2560, 1600)]
        );
    }

    #[test]
    fn folio_table_has_one_texture_pair_for_each_authored_card() {
        assert_eq!(WIZARD_CARDS.len(), 25);
        assert_eq!(
            WIZARD_CARDS
                .iter()
                .map(|(id, _, _)| id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            25
        );
    }

    #[test]
    fn hud_only_reveals_the_counter_whose_authored_value_changed() {
        let previous = PlayerUiState {
            beans: 4,
            stars: 2,
            fire_seeds: 1,
            house_points_harry: 10,
            ..PlayerUiState::default()
        };
        let current = PlayerUiState {
            beans: 5,
            ..previous
        };
        assert_eq!(
            changed_hud_counters(previous, current),
            [false, false, false, true]
        );
    }

    #[test]
    fn quidditch_league_uses_the_compiled_six_match_schedule_and_final() {
        assert_eq!(
            QUIDDITCH_FIXTURES.map(|game| (
                game.home,
                game.visitor,
                game.other_home,
                game.other_visitor
            )),
            [
                (0, 3, 1, 2),
                (0, 1, 2, 3),
                (0, 2, 1, 3),
                (3, 0, 2, 1),
                (1, 0, 3, 2),
                (2, 0, 3, 1),
            ]
        );

        let mut league = QuidditchLeague::default();
        league.restart();
        league.random = 1;
        for _ in 0..QUIDDITCH_FIXTURES.len() {
            league.finish(200, 100);
        }
        assert!(league.finals);
        assert_eq!(league.current_game, 6);
        assert_eq!(league.final_teams[0], 0);
        assert!(!league.fixture().level.is_empty());
        league.finish(200, 100);
        assert_eq!(league.screen, QuidditchScreen::FinalResults);
    }

    #[test]
    fn new_game_story_uses_all_compiled_pages_and_wav_timing() {
        assert_eq!(NEW_GAME_STORY.len(), 14);
        assert_eq!(NEW_GAME_STORY[0], ("3_1_", "StoryBook1"));
        assert_eq!(NEW_GAME_STORY[13], ("3_7_", "StoryBook49"));

        let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        wav.extend(16_u32.to_le_bytes());
        wav.extend([1, 0, 1, 0]);
        wav.extend(8_000_u32.to_le_bytes());
        wav.extend(8_000_u32.to_le_bytes());
        wav.extend([1, 0, 8, 0]);
        wav.extend(b"data");
        wav.extend(16_000_u32.to_le_bytes());
        wav.resize(wav.len() + 16_000, 0);
        let narration = wav_duration(&wav).unwrap();
        assert_eq!(narration, Duration::from_secs(2));

        let now = Instant::now();
        let (sound_at, deadline) = story_timing(now, narration);
        assert_eq!(
            sound_at.unwrap().duration_since(now),
            Duration::from_millis(1900)
        );
        assert_eq!(
            deadline.unwrap().duration_since(now),
            Duration::from_secs(5)
        );
    }
}
