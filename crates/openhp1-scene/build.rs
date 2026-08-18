use std::{env, fs, path::PathBuf};

fn main() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../window_masks");
    println!("cargo:rerun-if-changed={}", directory.display());

    let mut masks = fs::read_dir(&directory)
        .expect("reading window_masks")
        .map(|entry| entry.expect("reading window mask entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "png"))
        .collect::<Vec<_>>();
    masks.sort();

    let entries = masks
        .iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("window mask filename must be UTF-8");
            format!("({name:?}, include_bytes!({path:?}) as &[u8]),")
        })
        .collect::<String>();
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    fs::write(output.join("window_masks.rs"), format!("&[{entries}]"))
        .expect("writing embedded window mask index");
}
