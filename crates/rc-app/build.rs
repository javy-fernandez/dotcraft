fn main() -> Result<(), Box<dyn std::error::Error>> {
    let icon = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR")?).join("../../assets/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // llvm-rc (cargo xwin) resolves paths from the preprocessed copy in OUT_DIR, not the
        // project root, so the resource script names the icon by its absolute path.
        let icon = icon.canonicalize()?.to_string_lossy().replace('\\', "/");
        let rc = std::path::Path::new(&std::env::var("OUT_DIR")?).join("dotcraft.rc");
        std::fs::write(&rc, format!("1 ICON \"{icon}\"\n"))?;
        embed_resource::compile(&rc, embed_resource::NONE).manifest_required()?;
    }
    Ok(())
}
