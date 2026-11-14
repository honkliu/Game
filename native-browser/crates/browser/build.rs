fn main() {
    println!("cargo:rerun-if-changed=ui");

    // Copy `ui/` next to the produced executable so `browser.exe` finds its
    // shell assets without an installer. `app::ui_dir` falls back to the source
    // tree, but a copied binary has no source tree, so this copy is what makes
    // the shipped layout work.
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest.join("ui");
    let target_dir = match std::env::var("CARGO_TARGET_DIR") {
        Ok(t) => std::path::PathBuf::from(t),
        Err(_) => manifest
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("target"))
            .unwrap_or_else(|| manifest.join("target")),
    };
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    // Binary lives at target/<profile>/[deps/]browser.exe; copy to target/<profile>/ui.
    let out_dir = target_dir.join(&profile).join("ui");
    if src.is_dir() {
        if let Err(e) = copy_dir(&src, &out_dir) {
            println!("cargo:warning=could not copy ui assets: {e}");
        }
    }
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            // Ignore the error of a file being replaced while the browser runs.
            let _ = std::fs::copy(&from, &to);
        }
    }
    Ok(())
}