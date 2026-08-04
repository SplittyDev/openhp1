use anyhow::Result;
use egui::{Color32, Key, RichText};

pub(super) mod commands;

pub(super) struct DeveloperConsole {
    open: bool,
    focus_input: bool,
    input: String,
    draft: String,
    lines: Vec<ConsoleLine>,
    history: Vec<String>,
    history_index: Option<usize>,
    submitted: Vec<String>,
}

struct ConsoleLine {
    text: String,
    kind: LineKind,
}

enum LineKind {
    Input,
    Output,
    Error,
}

impl DeveloperConsole {
    pub(super) fn new() -> Self {
        Self {
            open: false,
            focus_input: false,
            input: String::new(),
            draft: String::new(),
            lines: vec![ConsoleLine {
                text: "OpenHP1 developer console. Type `help` for commands.".to_owned(),
                kind: LineKind::Output,
            }],
            history: Vec::new(),
            history_index: None,
            submitted: Vec::new(),
        }
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn toggle(&mut self) {
        self.open = !self.open;
        self.focus_input = self.open;
    }

    pub(super) fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.open {
            return;
        }
        egui::Panel::bottom("developer console")
            .default_size(280.0)
            .min_size(120.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.take_available_height();
                ui.visuals_mut().override_text_color = Some(Color32::LIGHT_GRAY);
                let output_height = (ui.available_height() - 32.0).max(40.0);
                egui::ScrollArea::vertical()
                    .max_height(output_height)
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for line in &self.lines {
                            let color = match line.kind {
                                LineKind::Input => Color32::LIGHT_BLUE,
                                LineKind::Output => Color32::LIGHT_GRAY,
                                LineKind::Error => Color32::LIGHT_RED,
                            };
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&line.text).monospace().color(color),
                                )
                                .wrap(),
                            );
                        }
                    });
                ui.separator();
                let response = ui
                    .horizontal(|ui| {
                        ui.monospace(">");
                        ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::singleline(&mut self.input)
                                .font(egui::TextStyle::Monospace),
                        )
                    })
                    .inner;
                if self.focus_input {
                    response.request_focus();
                    self.focus_input = false;
                }
                let (older, newer, submit) = ui.input(|input| {
                    (
                        input.key_pressed(Key::ArrowUp),
                        input.key_pressed(Key::ArrowDown),
                        input.key_pressed(Key::Enter),
                    )
                });
                if submit && (response.has_focus() || response.lost_focus()) {
                    self.submit();
                } else if response.has_focus() {
                    if older {
                        self.older_history();
                    } else if newer {
                        self.newer_history();
                    }
                }
            });
    }

    pub(super) fn take_submitted(&mut self) -> Vec<String> {
        std::mem::take(&mut self.submitted)
    }

    pub(super) fn record_result(&mut self, result: Result<String>) {
        let (text, kind) = match result {
            Ok(text) => (text, LineKind::Output),
            Err(error) => (format!("error: {error:#}"), LineKind::Error),
        };
        if !text.is_empty() {
            self.lines.push(ConsoleLine { text, kind });
        }
    }

    fn submit(&mut self) {
        let input = self.input.trim().to_owned();
        self.input.clear();
        self.history_index = None;
        self.draft.clear();
        self.focus_input = true;
        if input.is_empty() {
            return;
        }
        self.lines.push(ConsoleLine {
            text: format!("> {input}"),
            kind: LineKind::Input,
        });
        self.history.push(input.clone());
        self.submitted.push(input);
    }

    fn older_history(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let index = match self.history_index {
            Some(index) => index.saturating_sub(1),
            None => {
                self.draft.clone_from(&self.input);
                self.history.len() - 1
            }
        };
        self.history_index = Some(index);
        self.input.clone_from(&self.history[index]);
    }

    fn newer_history(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        if index + 1 < self.history.len() {
            self.history_index = Some(index + 1);
            self.input.clone_from(&self.history[index + 1]);
        } else {
            self.history_index = None;
            self.input.clone_from(&self.draft);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_commands_and_restores_the_unsubmitted_draft() {
        let mut console = DeveloperConsole::new();
        console.input = "help".to_owned();
        console.submit();
        console.input = "load Lev_Tut1b".to_owned();
        console.submit();
        assert_eq!(console.take_submitted(), ["help", "load Lev_Tut1b"]);

        console.input = "rep".to_owned();
        console.older_history();
        assert_eq!(console.input, "load Lev_Tut1b");
        console.older_history();
        assert_eq!(console.input, "help");
        console.newer_history();
        assert_eq!(console.input, "load Lev_Tut1b");
        console.newer_history();
        assert_eq!(console.input, "rep");
    }
}
