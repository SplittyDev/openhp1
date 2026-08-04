use anyhow::{Result, bail};

use super::super::Graphics;

mod fly;
mod help;
mod here;
mod load;
mod play;
mod report;
mod reset;
mod respawn;

pub(super) struct Command {
    name: &'static str,
    usage: &'static str,
    summary: &'static str,
    execute: fn(&mut Graphics, &str) -> Result<String>,
}

impl Command {
    const fn new(
        name: &'static str,
        usage: &'static str,
        summary: &'static str,
        execute: fn(&mut Graphics, &str) -> Result<String>,
    ) -> Self {
        Self {
            name,
            usage,
            summary,
            execute,
        }
    }
}

const COMMANDS: &[Command] = &[
    fly::COMMAND,
    help::COMMAND,
    here::COMMAND,
    load::COMMAND,
    play::COMMAND,
    report::COMMAND,
    reset::COMMAND,
    respawn::COMMAND,
];

pub(in crate::app) fn execute(graphics: &mut Graphics, input: &str) -> Result<String> {
    let (name, arguments) = parse_invocation(input);
    let Some(command) = find(name) else {
        bail!("unknown command `{name}`; type `help` for available commands");
    };
    (command.execute)(graphics, arguments)
}

fn parse_invocation(input: &str) -> (&str, &str) {
    let input = input.trim();
    input
        .find(char::is_whitespace)
        .map_or((input, ""), |split| {
            (&input[..split], input[split..].trim())
        })
}

fn find(name: &str) -> Option<&'static Command> {
    COMMANDS
        .iter()
        .find(|command| command.name.eq_ignore_ascii_case(name))
}

fn help(arguments: &str) -> Result<String> {
    if arguments.is_empty() {
        return Ok(COMMANDS
            .iter()
            .map(|command| format!("{:<16} {}", command.usage, command.summary))
            .collect::<Vec<_>>()
            .join("\n"));
    }
    let (name, extra) = parse_invocation(arguments);
    if !extra.is_empty() {
        bail!("unexpected extra input `{extra}`; usage: help [command]");
    }
    let Some(command) = find(name) else {
        bail!("unknown command `{name}`");
    };
    Ok(format!("{:<16} {}", command.usage, command.summary))
}

fn unavailable(name: &str) -> Result<String> {
    bail!("`{name}` is not implemented yet")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_drives_help_and_parses_the_argument_tail() {
        assert_eq!(
            parse_invocation("  report  \"moving pillar\"  "),
            ("report", "\"moving pillar\"")
        );
        assert!(find("LOAD").is_some());
        let text = help("").unwrap();
        assert_eq!(text.lines().count(), COMMANDS.len());
        for command in COMMANDS {
            assert!(text.contains(command.usage));
            assert!(text.contains(command.summary));
        }
        assert_eq!(
            help("FLY").unwrap(),
            "fly              Enter no-clip fly camera mode."
        );
        assert_eq!(
            help("fly now").unwrap_err().to_string(),
            "unexpected extra input `now`; usage: help [command]"
        );
    }
}
