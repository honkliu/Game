//! Font enumeration and face selection (§9.1).
//!
//! We read `C:\Windows\Fonts` for the faces named in the architecture's table
//! and resolve a CSS `font-family` list to one of them. No font-config
//! matching, no variable-font axes: weight and style pick from the small fixed
//! set of faces we load.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ahash::AHashMap;

use crate::face::{FaceId, FontFace};

/// Windows font directory. Evaluated once; a machine without it yields an empty
/// font set and every measure returns zero rather than panicking.
pub fn font_dir() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    Path::new(&root).join("Fonts")
}

/// One entry in the face table: a file, the families it serves, and its
/// weight/style.
struct FaceSpec {
    file: &'static str,
    families: &'static [&'static str],
    weight: u16,
    italic: bool,
}

/// The faces we ship against. `font-family` names are matched
/// case-insensitively, and the CSS generic families map through
/// [`generic_fallback`].
const FACES: &[FaceSpec] = &[
    FaceSpec { file: "segoeui.ttf", families: &["Segoe UI", "ui-sans-serif", "system-ui"], weight: 400, italic: false },
    FaceSpec { file: "segoeuib.ttf", families: &["Segoe UI", "ui-sans-serif", "system-ui"], weight: 700, italic: false },
    FaceSpec { file: "segoeuii.ttf", families: &["Segoe UI", "ui-sans-serif", "system-ui"], weight: 400, italic: true },
    FaceSpec { file: "segoeuiz.ttf", families: &["Segoe UI", "ui-sans-serif", "system-ui"], weight: 700, italic: true },
    FaceSpec { file: "arial.ttf", families: &["Arial", "Helvetica", "sans-serif"], weight: 400, italic: false },
    FaceSpec { file: "arialbd.ttf", families: &["Arial", "Helvetica", "sans-serif"], weight: 700, italic: false },
    FaceSpec { file: "ariali.ttf", families: &["Arial", "Helvetica", "sans-serif"], weight: 400, italic: true },
    FaceSpec { file: "arialbi.ttf", families: &["Arial", "Helvetica", "sans-serif"], weight: 700, italic: true },
    FaceSpec { file: "times.ttf", families: &["Times New Roman", "Times", "serif"], weight: 400, italic: false },
    FaceSpec { file: "timesbd.ttf", families: &["Times New Roman", "Times", "serif"], weight: 700, italic: false },
    FaceSpec { file: "timesi.ttf", families: &["Times New Roman", "Times", "serif"], weight: 400, italic: true },
    FaceSpec { file: "timesbi.ttf", families: &["Times New Roman", "Times", "serif"], weight: 700, italic: true },
    FaceSpec { file: "georgia.ttf", families: &["Georgia"], weight: 400, italic: false },
    FaceSpec { file: "georgiab.ttf", families: &["Georgia"], weight: 700, italic: false },
    FaceSpec { file: "consola.ttf", families: &["Consolas", "Courier New", "monospace"], weight: 400, italic: false },
    FaceSpec { file: "consolab.ttf", families: &["Consolas", "Courier New", "monospace"], weight: 700, italic: false },
    FaceSpec { file: "cour.ttf", families: &["Courier New", "monospace"], weight: 400, italic: false },
    FaceSpec { file: "courbd.ttf", families: &["Courier New", "monospace"], weight: 700, italic: false },
];

/// CSS generic families mapped to a concrete one (§9.1: `cursive`/`fantasy`
/// become sans-serif).
pub fn generic_fallback(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "serif" => "Times New Roman",
        "sans-serif" | "ui-sans-serif" | "system-ui" | "cursive" | "fantasy" => "Segoe UI",
        "monospace" | "ui-monospace" => "Consolas",
        _ => return None,
    })
}

/// The loaded face table, keyed by CSS family (lowercased) and weight/italic.
pub struct FaceTable {
    /// family(lowercase) -> (weight, italic) -> FaceId
    by_family: AHashMap<String, AHashMap<(u16, bool), FaceId>>,
    /// Faces available for glyph fallback, in preference order.
    fallback: Vec<FaceId>,
    /// Which file each face came from, for the report.
    pub files: HashMap<FaceId, PathBuf>,
}

impl FaceTable {
    /// Load every face in [`FACES`] that exists on this machine.
    ///
    /// A missing file is not an error: a machine without `segoeuii.ttf` simply
    /// has no italic Segoe UI, and selection falls back to the regular weight.
    pub fn load() -> (FaceTable, Vec<FontFace>) {
        let dir = font_dir();
        let mut table = FaceTable {
            by_family: AHashMap::new(),
            fallback: Vec::new(),
            files: HashMap::new(),
        };
        let mut faces = Vec::new();
        for spec in FACES {
            let path = dir.join(spec.file);
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let Some(face) = FontFace::from_bytes(bytes, spec.file) else { continue };
            let id = FaceId(faces.len() as u32);
            faces.push(face);
            table.files.insert(id, path);
            for fam in spec.families {
                table
                    .by_family
                    .entry(fam.to_ascii_lowercase())
                    .or_default()
                    .insert((spec.weight, spec.italic), id);
            }
            if spec.weight == 400 || table.fallback.is_empty() {
                table.fallback.push(id);
            }
        }
        (table, faces)
    }

    pub fn is_empty(&self) -> bool {
        self.fallback.is_empty()
    }

    /// Resolve a CSS family list plus weight/style to a face.
    ///
    /// Weight is mapped by threshold (400 normal, 700 bold) as §9.1 specifies.
    pub fn select(&self, families: &[String], weight: u16, italic: bool) -> Option<FaceId> {
        let bold = weight >= 600;
        for fam in families {
            let key = fam.to_ascii_lowercase();
            let candidates = self.by_family.get(&key).or_else(|| {
                generic_fallback(fam).and_then(|g| self.by_family.get(&g.to_ascii_lowercase()))
            });
            let Some(map) = candidates else { continue };
            // Exact (weight, italic), then same style at the other weight, then
            // the regular face. No synthetic bold or oblique (§15).
            if let Some(&id) = map.get(&(if bold { 700 } else { 400 }, italic)) {
                return Some(id);
            }
            if let Some(&id) = map.get(&(if bold { 700 } else { 400 }, false)) {
                return Some(id);
            }
            if let Some(&id) = map.get(&(400, false)) {
                return Some(id);
            }
        }
        self.fallback.first().copied()
    }

    /// Faces to try, in order, when a glyph is missing from the selected face.
    pub fn fallback_order(&self) -> &[FaceId] {
        &self.fallback
    }
}