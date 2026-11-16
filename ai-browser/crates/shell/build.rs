use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ui = manifest.join("ui");
    let pane_src = manifest.join("..").join("..").join("ui");
    let pane_out = ui.join("sidebar");

    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-env-changed=AI_BROWSER_SKIP_UI_BUILD");
    for p in ["src", "sidebar.html", "package.json", "vite.config.ts", "tsconfig.json"] {
        println!("cargo:rerun-if-changed={}", pane_src.join(p).display());
    }

    build_pane(&pane_src, &pane_out);
    if !pane_out.join("sidebar.html").is_file() {
        panic!("the AI pane is not built: run `npm install && npm run build` in ai-browser/ui");
    }

    // Embed every shell asset so browser.exe is a single self-contained file.
    let mut files = Vec::new();
    collect(&ui, &ui, &mut files);
    files.sort();
    let mut table = String::from("pub static ASSETS: &[(&str, &[u8])] = &[\n");
    for rel in &files {
        let abs = ui.join(rel).canonicalize().unwrap();
        let abs = abs.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/");
        table.push_str(&format!("    ({rel:?}, include_bytes!({abs:?})),\n"));
    }
    table.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("assets.rs");
    std::fs::write(out, table).unwrap();
}

/// Runs the Vite build for the Preact pane. Skippable for offline/CI builds that
/// ship prebuilt output.
fn build_pane(src: &Path, out: &Path) {
    if std::env::var_os("AI_BROWSER_SKIP_UI_BUILD").is_some() || !src.join("package.json").is_file() {
        return;
    }
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let run = |args: &[&str]| Command::new(npm).args(args).current_dir(src).status();
    if !src.join("node_modules").is_dir()
        && !matches!(run(&["install", "--no-audit", "--no-fund"]), Ok(s) if s.success())
    {
        println!("cargo:warning=npm install failed; using existing pane output if present");
        return;
    }
    match run(&["run", "build"]) {
        Ok(s) if s.success() => {}
        _ if out.join("sidebar.html").is_file() => {
            println!("cargo:warning=npm run build failed; using the previously built pane");
        }
        _ => panic!("npm run build failed for the AI pane"),
    }
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}
