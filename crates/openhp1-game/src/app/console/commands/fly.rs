use anyhow::{Result, bail};

use super::{Command, Graphics};

pub(super) const COMMAND: Command =
    Command::new("fly", "fly", "Enter no-clip fly camera mode.", execute);

fn execute(graphics: &mut Graphics, arguments: &str) -> Result<String> {
    validate_arguments(arguments)?;
    graphics.fly_camera_active = true;
    Ok("Fly camera enabled.".to_owned())
}

fn validate_arguments(arguments: &str) -> Result<()> {
    if !arguments.is_empty() {
        bail!("usage: fly");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_arguments() {
        assert!(validate_arguments("").is_ok());
        assert_eq!(
            validate_arguments("now").unwrap_err().to_string(),
            "usage: fly"
        );
    }
}
