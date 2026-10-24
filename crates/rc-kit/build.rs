//! Embeds every SVG of `assets/icons` as a sorted table `(name, light, dark)`, where `name` is
//! `dir/file` without the extension and `dark` is the `_dark.svg` variant when there is one.

use std::fmt::Write;
use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut names = Vec::new();
    for dir in std::fs::read_dir(&root).expect("assets/icons").flatten() {
        if !dir.path().is_dir() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", dir.path().display());
        for file in std::fs::read_dir(dir.path()).unwrap().flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".svg").filter(|s| !s.ends_with("_dark")) {
                names.push(format!("{}/{stem}", dir.file_name().to_string_lossy()));
            }
        }
    }
    names.sort();
    let mut out = String::from("pub(crate) static ICONS: &[(&str, &[u8], Option<&[u8]>)] = &[\n");
    for n in &names {
        let light = root.join(format!("{n}.svg"));
        let dark = root.join(format!("{n}_dark.svg"));
        let dark = if dark.exists() { format!("Some(include_bytes!({:?}))", dark.display().to_string()) } else { "None".into() };
        writeln!(out, "    ({n:?}, include_bytes!({:?}), {dark}),", light.display().to_string()).unwrap();
    }
    out.push_str("];\n");
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons_table.rs"), out).unwrap();
}
