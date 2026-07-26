use std::{env, error::Error, path::PathBuf};

use openhp1_runtime::{ScriptRuntime, Value};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let root = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: dispatch_event <game-root> <class-package> <class-export> <event>")?;
    let package = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: dispatch_event <game-root> <class-package> <class-export> <event>")?;
    let export = arguments
        .next()
        .ok_or("usage: dispatch_event <game-root> <class-package> <class-export> <event>")?
        .to_string_lossy()
        .parse::<usize>()?;
    let event = arguments
        .next()
        .ok_or("usage: dispatch_event <game-root> <class-package> <class-export> <event>")?;
    let values = arguments
        .next()
        .map(|value| value.to_string_lossy().parse().map(Value::Float))
        .transpose()?
        .into_iter()
        .collect::<Vec<_>>();

    let mut runtime = ScriptRuntime::new(root)?;
    for action in runtime.dispatch_event_with_arguments(
        0,
        package,
        export,
        &event.to_string_lossy(),
        &values,
    )? {
        println!("{action:?}");
    }
    Ok(())
}
