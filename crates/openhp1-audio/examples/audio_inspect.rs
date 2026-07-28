use std::{env, error::Error, path::PathBuf};

use openhp1_audio::AudioClip;
use openhp1_package::Package;

fn main() -> Result<(), Box<dyn Error>> {
    let path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: cargo run -p openhp1-audio --example audio_inspect -- <package>")?;
    let package = Package::open(&path)?;
    for (index, export) in package.summary().exports.iter().enumerate() {
        let class = package.summary().class_name(export).unwrap_or("<unknown>");
        if !class.eq_ignore_ascii_case("Sound") && !class.eq_ignore_ascii_case("Music") {
            continue;
        }
        let clip = AudioClip::decode(&package, index)?;
        println!(
            "{}: {} bytes of {}",
            package.summary().name(export.object_name),
            clip.data().len(),
            clip.format()
        );
    }
    Ok(())
}
