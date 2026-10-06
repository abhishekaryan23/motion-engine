//! Bring your own images (plan §5a): the model passes references, never
//! pixels. A reference is a path relative to a configured asset root; the
//! server rejects absolute paths, `..`, symlinks out of the root, non-images,
//! files over 40 MB or 8000 px. Roles are given or guessed (Apple Vision on
//! macOS: a person → person, a wide opaque scene → place, else object); opaque
//! people photos are cut out with `motion-engine matte`; results are cached by
//! file hash under `<jobs>/_cache/images/`.
//!
//! This module holds the building blocks (sandbox, folder convention, probe,
//! cutout, manifest); [`crate::policy`] decides which beat and role each image
//! serves. Nothing here prints to stdout (the MCP transport owns it): every
//! subprocess runs with captured output.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::job::{IMAGE_CACHE_DIR, MANIFEST_JSON};
use crate::lite::ImageRole;
use crate::pictures::edit_distance;
use crate::profile::ServerConfig;

/// Largest accepted image file.
pub const MAX_IMAGE_BYTES: u64 = 40 * 1024 * 1024;
/// Largest accepted image side.
pub const MAX_IMAGE_SIDE: u32 = 8000;
/// Smallest useful image height (a fix-it below this).
pub const MIN_IMAGE_HEIGHT: u32 = 720;
/// Accepted file extensions (checked together with the file's magic bytes).
pub const IMAGE_EXTENSIONS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];
/// Role guess: a detected person covering at least this share of the image.
pub const PERSON_MIN_AREA: f64 = 0.08;
/// Role guess: an opaque image at least this wide (width / height) is a place.
pub const PLACE_MIN_ASPECT: f64 = 1.2;

/// One user image, ready to deliver to the compiler.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageEntry {
    /// Manifest id the compiler asks for, e.g. `beat_2.hero_object`.
    pub id: String,
    /// The file to copy into the job (the original, or its cached cutout).
    pub source: PathBuf,
    /// File name inside `<job>/images/`.
    pub file_name: String,
    /// sha256 of the original file (job id input).
    pub sha256: String,
    pub role: ImageRole,
    pub role_guessed: bool,
    pub width: u32,
    pub height: u32,
    pub alpha: bool,
    /// True when `source` is a cutout made by `matte`.
    pub cutout: bool,
    /// Engine-measured labels (`describe`), e.g. `["beach", "person"]`.
    pub labels: Vec<String>,
    /// Every request id this image serves (empty = just `id`). An object
    /// image serves the object roles a grammar may ask for
    /// (`hero_object`, `supporting_object`, `evidence_image`).
    pub serves: Vec<String>,
    /// A passed-through manifest entry (plan §5a way 3): its other fields
    /// (anchors, face bounds …) are kept in the job manifest.
    pub meta: Option<serde_json::Value>,
}

/// The images of one request.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PreparedImages {
    pub entries: Vec<ImageEntry>,
    /// Check-mode lines, one per image: role, size, person, cutout, problems.
    pub lines: Vec<String>,
}

impl PreparedImages {
    /// The job-id input: ids, roles and content hashes (no paths).
    pub fn fingerprint(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id, "role": e.role, "sha256": e.sha256, "cutout": e.cutout,
                    })
                })
                .collect(),
        )
    }
}

/// Copy the images into `<job_dir>/images/` and write
/// `<job_dir>/assets.manifest.json` (paths relative to the job dir). Returns
/// the manifest path, for `reel --asset-manifest`.
pub fn write_manifest(images: &PreparedImages, job_dir: &Path) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind};
    let manifest = manifest_json(images).map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
    let dir = job_dir.join("images");
    std::fs::create_dir_all(&dir)?;
    let mut copied = BTreeSet::new();
    for e in &images.entries {
        if copied.insert(e.file_name.clone()) {
            std::fs::copy(&e.source, dir.join(&e.file_name))?;
        }
    }
    let path = job_dir.join(MANIFEST_JSON);
    let text = serde_json::to_string_pretty(&manifest)
        .map_err(|e| Error::new(ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(&path, text + "\n")?;
    Ok(path)
}

/// The job manifest for `images` (an engine `AssetManifest`: id, path
/// `images/<file>`, size, alpha, serves; passed-through entries keep their
/// other fields), checked with `AssetManifest::validate`. Analysis is left
/// out, so the engine measures each picture when it loads the manifest.
pub fn manifest_json(images: &PreparedImages) -> Result<serde_json::Value, String> {
    let mut assets = Vec::new();
    for e in &images.entries {
        if !safe_file_name(&e.file_name) {
            return Err(format!("unsafe image file name '{}'", e.file_name));
        }
        let mut entry = match &e.meta {
            Some(serde_json::Value::Object(m)) => m.clone(),
            _ => serde_json::Map::new(),
        };
        let serves = if e.serves.is_empty() {
            vec![e.id.clone()]
        } else {
            e.serves.clone()
        };
        entry.insert("id".into(), e.id.clone().into());
        entry.insert("path".into(), format!("images/{}", e.file_name).into());
        entry.insert("width".into(), e.width.into());
        entry.insert("height".into(), e.height.into());
        entry.insert("alpha".into(), e.alpha.into());
        entry.insert("serves".into(), serde_json::json!(serves));
        assets.push(serde_json::Value::Object(entry));
    }
    let manifest = serde_json::json!({
        "version": motion_core::assets::ASSET_MANIFEST_VERSION,
        "assets": assets,
    });
    let parsed: motion_core::assets::AssetManifest =
        serde_json::from_value(manifest.clone()).map_err(|e| e.to_string())?;
    parsed.validate().map_err(|errs| errs.join("; "))?;
    Ok(manifest)
}

fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

// ---------------------------------------------------------------------------
// Image headers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    /// The extension a copy of this image gets.
    pub fn ext(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
        }
    }
}

/// Format, size and alpha read from the file's own header bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageHeader {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    /// The format carries an alpha channel (it may still be fully opaque).
    pub alpha: bool,
}

fn be16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(*b.get(i)?) << 8 | u32::from(*b.get(i + 1)?))
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    Some(be16(b, i)? << 16 | be16(b, i + 2)?)
}

fn le16(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(*b.get(i)?) | u32::from(*b.get(i + 1)?) << 8)
}

fn le24(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from(*b.get(i)?) | u32::from(*b.get(i + 1)?) << 8 | u32::from(*b.get(i + 2)?) << 16)
}

/// Parse a PNG, JPEG or WebP header. `None` = not one of those images.
pub fn image_header(b: &[u8]) -> Option<ImageHeader> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        if b.get(12..16)? != b"IHDR" {
            return None;
        }
        let (width, height) = (be32(b, 16)?, be32(b, 20)?);
        let color = *b.get(25)?;
        let mut alpha = matches!(color, 4 | 6);
        // A palette or grey/RGB image with a tRNS chunk has transparency.
        let mut i = 8usize;
        while !alpha && i + 8 <= b.len() {
            let len = be32(b, i)? as usize;
            let kind = b.get(i + 4..i + 8)?;
            if kind == b"tRNS" {
                alpha = true;
            }
            if kind == b"IDAT" || kind == b"IEND" {
                break;
            }
            i = i.checked_add(12 + len)?;
        }
        return Some(ImageHeader {
            format: ImageFormat::Png,
            width,
            height,
            alpha,
        });
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        let mut i = 2usize;
        while i + 4 <= b.len() {
            if b[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = b[i + 1];
            if marker == 0xFF {
                i += 1;
                continue;
            }
            if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
                i += 2;
                continue;
            }
            let len = be16(b, i + 2)? as usize;
            let sof = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
            if sof {
                return Some(ImageHeader {
                    format: ImageFormat::Jpeg,
                    height: be16(b, i + 5)?,
                    width: be16(b, i + 7)?,
                    alpha: false,
                });
            }
            if marker == 0xDA || marker == 0xD9 {
                return None;
            }
            i += 2 + len;
        }
        return None;
    }
    if b.len() >= 30 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        let header = |width, height, alpha| ImageHeader {
            format: ImageFormat::Webp,
            width,
            height,
            alpha,
        };
        return match &b[12..16] {
            b"VP8 " => {
                if b.get(23..26)? != [0x9d, 0x01, 0x2a] {
                    return None;
                }
                let w = le16(b, 26)? & 0x3fff;
                let h = le16(b, 28)? & 0x3fff;
                Some(header(w, h, false))
            }
            b"VP8L" => {
                if *b.get(20)? != 0x2f {
                    return None;
                }
                let bits = u32::from(b[21])
                    | u32::from(b[22]) << 8
                    | u32::from(b[23]) << 16
                    | u32::from(b[24]) << 24;
                let w = (bits & 0x3fff) + 1;
                let h = ((bits >> 14) & 0x3fff) + 1;
                Some(header(w, h, (bits >> 28) & 1 == 1))
            }
            b"VP8X" => {
                let flags = *b.get(20)?;
                Some(header(
                    le24(b, 24)? + 1,
                    le24(b, 27)? + 1,
                    flags & 0x10 != 0,
                ))
            }
            _ => None,
        };
    }
    None
}

// ---------------------------------------------------------------------------
// Sandbox
// ---------------------------------------------------------------------------

/// Why a reference was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefErrorKind {
    /// Absolute, `..`, a backslash or a symlink leading out of the root:
    /// always refused (security), whatever the mode.
    Outside,
    /// Nothing at that path inside any asset root.
    NotFound,
    /// Exists but is not usable (not an image, too large …).
    Invalid,
}

/// A refused reference, as fix-it material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefError {
    pub kind: RefErrorKind,
    pub problem: String,
    /// Close existing names, best first.
    pub options: Vec<String>,
    pub otherwise: Option<String>,
}

impl RefError {
    fn new(kind: RefErrorKind, problem: impl Into<String>) -> Self {
        RefError {
            kind,
            problem: problem.into(),
            options: Vec::new(),
            otherwise: None,
        }
    }

    fn otherwise(mut self, text: impl Into<String>) -> Self {
        self.otherwise = Some(text.into());
        self
    }

    /// The fix-it for `field` of beat `beat`.
    pub fn to_fix(&self, beat: Option<usize>, field: &str) -> crate::reply::Fix {
        let mut f = crate::reply::Fix::new(beat, field, self.problem.clone())
            .options(self.options.iter().cloned());
        f.otherwise = self.otherwise.clone();
        f
    }
}

/// The configured asset roots, with a repository-relative label for messages.
#[derive(Debug, Clone, Copy)]
pub struct Sandbox<'a> {
    pub roots: &'a [PathBuf],
    pub repo: &'a Path,
}

/// A sandbox-checked user image.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedFile {
    /// The reference as given (trimmed, `file:` removed).
    pub reference: String,
    /// Canonical path (inside `root`).
    pub path: PathBuf,
    /// Canonical asset root it was found in.
    pub root: PathBuf,
    pub header: ImageHeader,
    pub bytes: u64,
    pub sha256: String,
}

impl CheckedFile {
    /// The file name (for messages).
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.reference.clone())
    }
}

impl<'a> Sandbox<'a> {
    pub fn new(config: &'a ServerConfig) -> Self {
        Sandbox {
            roots: &config.asset_roots,
            repo: &config.repo,
        }
    }

    /// `assets/inbox/` (the first root, relative to the repository).
    pub fn label(&self) -> String {
        let Some(root) = self.roots.first() else {
            return "the asset folder".to_string();
        };
        let rel = root.strip_prefix(self.repo).unwrap_or(root);
        format!("{}/", rel.display())
    }

    /// `./x`, or a path written with the root itself (`assets/inbox/x`) → `x`.
    fn strip_root_prefix<'r>(&self, r: &'r str) -> &'r str {
        let mut r = r;
        while let Some(rest) = r.strip_prefix("./") {
            r = rest;
        }
        for root in self.roots {
            let rel = root.strip_prefix(self.repo).unwrap_or(root);
            if rel.is_absolute() || rel.as_os_str().is_empty() {
                continue;
            }
            let label = rel.to_string_lossy().replace('\\', "/");
            if let Some(rest) = r
                .strip_prefix(label.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
            {
                return rest;
            }
        }
        r
    }

    /// Syntactic checks shared by files and folders.
    fn check_syntax(&self, reference: &str) -> Result<String, RefError> {
        let r = reference.trim();
        let r = r.strip_prefix("file:").unwrap_or(r).trim();
        let r = self.strip_root_prefix(r);
        let inside = format!("use a path inside {}", self.label());
        if r.is_empty() {
            return Err(RefError::new(RefErrorKind::Invalid, "empty file reference")
                .otherwise(format!("{inside}, e.g. my_trip/beach.jpg")));
        }
        if r.contains('\\') || r.contains('\0') {
            return Err(RefError::new(
                RefErrorKind::Outside,
                format!("'{r}' uses a backslash or control character"),
            )
            .otherwise(format!("{inside} with / between folders")));
        }
        let p = Path::new(r);
        let windows_drive =
            r.len() >= 2 && r.as_bytes()[1] == b':' && r.as_bytes()[0].is_ascii_alphabetic();
        if p.is_absolute() || r.starts_with('~') || windows_drive {
            return Err(
                RefError::new(RefErrorKind::Outside, format!("'{r}' is an absolute path"))
                    .otherwise(inside),
            );
        }
        if p.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(RefError::new(
                RefErrorKind::Outside,
                format!("'{r}' is outside the asset folder"),
            )
            .otherwise(inside));
        }
        Ok(r.to_string())
    }

    /// Canonicalize `candidate` and check it stays inside `root`.
    fn inside(&self, r: &str, candidate: &Path, root: &Path) -> Result<PathBuf, RefError> {
        let root = root
            .canonicalize()
            .map_err(|_| RefError::new(RefErrorKind::NotFound, format!("'{r}' not found")))?;
        let canon = candidate
            .canonicalize()
            .map_err(|_| RefError::new(RefErrorKind::NotFound, format!("'{r}' not found")))?;
        if !canon.starts_with(&root) {
            return Err(RefError::new(
                RefErrorKind::Outside,
                format!("'{r}' is outside the asset folder (a link leads out)"),
            )
            .otherwise(format!("use a path inside {}", self.label())));
        }
        Ok(canon)
    }

    /// The first root holding `r` (symlinks included), if any.
    fn locate(&self, r: &str) -> Option<&'a PathBuf> {
        self.roots
            .iter()
            .find(|root| std::fs::symlink_metadata(root.join(r)).is_ok())
    }

    /// Resolve a folder reference (`assets: "my_trip"`).
    pub fn resolve_folder(&self, reference: &str) -> Result<(PathBuf, PathBuf), RefError> {
        let r = self.check_syntax(reference)?;
        let r = r.trim_end_matches('/').to_string();
        let Some(root) = self.locate(&r) else {
            return Err(RefError::new(
                RefErrorKind::NotFound,
                format!("'{r}' not found in {}", self.label()),
            )
            .otherwise(format!(
                "put the images in {}<folder>/ and pass the folder name",
                self.label()
            )));
        };
        let canon = self.inside(&r, &root.join(&r), root)?;
        if !canon.is_dir() {
            return Err(RefError::new(
                RefErrorKind::Invalid,
                format!("'{r}' is a file, not a folder"),
            )
            .otherwise("pass a folder name, or reference the file in a beat as file:<path>"));
        }
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        Ok((canon, root))
    }

    /// Resolve and check one image reference (relative to an asset root).
    pub fn resolve(&self, reference: &str) -> Result<CheckedFile, RefError> {
        let r = self.check_syntax(reference)?;
        let Some(root) = self.locate(&r) else {
            let mut e = RefError::new(
                RefErrorKind::NotFound,
                format!("'{r}' not found in {}", self.label()),
            )
            .otherwise("check the name, or use a library picture");
            e.options = self.near_names(&r);
            return Err(e);
        };
        let path = self.inside(&r, &root.join(&r), root)?;
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        check_image_file(&r, &path, root)
    }

    /// Up to 3 existing image paths close to `r` (same folder, by spelling).
    fn near_names(&self, r: &str) -> Vec<String> {
        let (dir, name) = match r.rsplit_once('/') {
            Some((d, n)) => (d.to_string(), n.to_string()),
            None => (String::new(), r.to_string()),
        };
        let mut scored: Vec<(usize, String)> = Vec::new();
        for root in self.roots {
            let base = if dir.is_empty() {
                root.clone()
            } else {
                root.join(&dir)
            };
            let Ok(canon) = base.canonicalize() else {
                continue;
            };
            let Ok(root_c) = root.canonicalize() else {
                continue;
            };
            if !canon.starts_with(&root_c) {
                continue;
            }
            let Ok(rd) = std::fs::read_dir(&canon) else {
                continue;
            };
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if !has_image_ext(&n) {
                    continue;
                }
                let d = edit_distance(&n.to_lowercase(), &name.to_lowercase());
                if d <= 3 {
                    let full = if dir.is_empty() {
                        n
                    } else {
                        format!("{dir}/{n}")
                    };
                    scored.push((d, full));
                }
            }
        }
        scored.sort();
        scored.dedup();
        scored.into_iter().take(3).map(|(_, n)| n).collect()
    }
}

fn has_image_ext(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The checks every image file passes: a file, an image extension, at most
/// [`MAX_IMAGE_BYTES`], image magic bytes, sides at most [`MAX_IMAGE_SIDE`].
fn check_image_file(r: &str, path: &Path, root: PathBuf) -> Result<CheckedFile, RefError> {
    let meta = std::fs::metadata(path)
        .map_err(|_| RefError::new(RefErrorKind::NotFound, format!("'{r}' not found")))?;
    if !meta.is_file() {
        return Err(RefError::new(
            RefErrorKind::Invalid,
            format!("'{r}' is a folder, not an image"),
        )
        .otherwise("reference an image file, or pass the folder as assets"));
    }
    if !has_image_ext(r) {
        return Err(RefError::new(
            RefErrorKind::Invalid,
            format!("'{r}' is not a jpg, png or webp image"),
        )
        .otherwise("convert it to jpg or png, or use a library picture"));
    }
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(RefError::new(
            RefErrorKind::Invalid,
            format!(
                "'{r}' is {} MB, over the {} MB limit",
                meta.len().div_ceil(1024 * 1024),
                MAX_IMAGE_BYTES / (1024 * 1024)
            ),
        )
        .otherwise("use a smaller file"));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| RefError::new(RefErrorKind::Invalid, format!("'{r}' cannot be read: {e}")))?;
    let Some(header) = image_header(&bytes) else {
        return Err(RefError::new(
            RefErrorKind::Invalid,
            format!("'{r}' is not a readable jpg, png or webp image"),
        )
        .otherwise("export it again as jpg or png"));
    };
    if header.width == 0 || header.height == 0 {
        return Err(RefError::new(
            RefErrorKind::Invalid,
            format!("'{r}' has no pixels"),
        ));
    }
    let side = header.width.max(header.height);
    if side > MAX_IMAGE_SIDE {
        return Err(RefError::new(
            RefErrorKind::Invalid,
            format!(
                "'{r}' is {}×{} px, over the {MAX_IMAGE_SIDE} px limit",
                header.width, header.height
            ),
        )
        .otherwise(format!("resize it to at most {MAX_IMAGE_SIDE} px")));
    }
    let digest = Sha256::digest(&bytes);
    Ok(CheckedFile {
        reference: r.to_string(),
        path: path.to_path_buf(),
        root,
        header,
        bytes: meta.len(),
        sha256: digest.iter().map(|b| format!("{b:02x}")).collect(),
    })
}

// ---------------------------------------------------------------------------
// Folder convention
// ---------------------------------------------------------------------------

/// What a file name in an images folder means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convention {
    /// `beatN.ext` / `beatN_<role>.ext`.
    Beat(usize, Option<ImageRole>),
    /// `background.ext`: the environment of every beat without its own image.
    Background,
}

/// Parse a role word (`person`, `people`, `object`, `thing`, `place`, `scene` …).
pub fn role_word(word: &str) -> Option<ImageRole> {
    match word.trim().to_ascii_lowercase().as_str() {
        "person" | "people" | "human" | "portrait" | "face" | "figure" => Some(ImageRole::Person),
        "object" | "thing" | "item" | "product" | "prop" => Some(ImageRole::Object),
        "place" | "scene" | "location" | "landscape" | "background" | "environment" | "bg" => {
            Some(ImageRole::Place)
        }
        _ => None,
    }
}

/// The meaning of `file_name` under the folder convention.
pub fn convention(file_name: &str) -> Option<Convention> {
    if !has_image_ext(file_name) {
        return None;
    }
    let stem = Path::new(file_name)
        .file_stem()?
        .to_str()?
        .to_ascii_lowercase();
    if matches!(stem.as_str(), "background" | "bg") {
        return Some(Convention::Background);
    }
    let rest = stem.strip_prefix("beat")?;
    let rest = rest.trim_start_matches(['_', '-', ' ']);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let n: usize = digits.parse().ok()?;
    let tail = rest[digits.len()..].trim_start_matches(['_', '-', ' ']);
    if tail.is_empty() {
        return Some(Convention::Beat(n, None));
    }
    role_word(tail).map(|r| Convention::Beat(n, Some(r)))
}

/// An images folder read by the convention.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FolderScan {
    /// Beat (1-based) → (file name, role given by the name).
    pub beats: BTreeMap<usize, (String, Option<ImageRole>)>,
    pub background: Option<String>,
    /// `manifest.json` exists (it then replaces the convention).
    pub manifest: bool,
    /// Image files the convention does not use (one line each).
    pub ignored: Vec<String>,
}

/// Read `dir` (non-recursive, by name order).
pub fn scan_folder(dir: &Path) -> std::io::Result<FolderScan> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut scan = FolderScan::default();
    for name in names {
        if name == "manifest.json" {
            scan.manifest = true;
            continue;
        }
        match convention(&name) {
            Some(Convention::Beat(n, role)) if n >= 1 => {
                if let Some((first, _)) = scan.beats.get(&n) {
                    scan.ignored.push(format!(
                        "'{name}' not used (beat {n} already has '{first}')"
                    ));
                } else {
                    scan.beats.insert(n, (name, role));
                }
            }
            Some(Convention::Background) => {
                if let Some(first) = &scan.background {
                    scan.ignored
                        .push(format!("'{name}' not used (the background is '{first}')"));
                } else {
                    scan.background = Some(name);
                }
            }
            _ if has_image_ext(&name) => scan.ignored.push(format!(
                "'{name}' not used — name it beatN.{} or background.{}",
                ext_of(&name),
                ext_of(&name)
            )),
            _ => {}
        }
    }
    Ok(scan)
}

fn ext_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg")
        .to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// Probe (Apple Vision) and role guess
// ---------------------------------------------------------------------------

/// What the Vision helper measured (`tools/vision_probe.swift`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub opaque: bool,
    pub humans: u32,
    pub human_area_max: f64,
    #[serde(default)]
    pub labels: Vec<String>,
}

/// Role guess (plan §5a): a person box ≥ 8 % of the image → person; else an
/// opaque image with aspect ≥ 1.2 → place; else object.
pub fn guess_role(p: &Probe) -> ImageRole {
    if p.humans > 0 && p.human_area_max >= PERSON_MIN_AREA {
        ImageRole::Person
    } else if p.opaque
        && p.height > 0
        && f64::from(p.width) / f64::from(p.height) >= PLACE_MIN_ASPECT
    {
        ImageRole::Place
    } else {
        ImageRole::Object
    }
}

/// `<jobs>/_cache/images/<sha>/`.
pub fn cache_dir(config: &ServerConfig, sha: &str) -> PathBuf {
    config.jobs.join(IMAGE_CACHE_DIR).join(sha)
}

/// A temp-file suffix unique within this machine (process id + a counter),
/// so concurrent preparations never write the same temp file.
fn tmp_suffix() -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn tools_dir() -> Option<PathBuf> {
    std::env::var_os("MOTION_TOOLS_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/motionengine/tools"))
        })
}

/// The compiled Vision helper, (re)built when missing or older than its
/// source (the `matte` helper's pattern). Errors on hosts without macOS.
pub fn vision_helper(config: &ServerConfig) -> Result<PathBuf, String> {
    if !cfg!(target_os = "macos") {
        return Err("image roles need macOS (Apple Vision)".into());
    }
    let root = tools_dir().ok_or("no HOME for the tools cache")?;
    let bin = root.join("vision_probe");
    let src = [
        config
            .repo
            .join("crates/motion-mcp/tools/vision_probe.swift"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/vision_probe.swift"),
    ]
    .into_iter()
    .find(|p| p.is_file());
    let stale = match (std::fs::metadata(&bin), src.as_ref().map(std::fs::metadata)) {
        (Ok(b), Some(Ok(s))) => match (b.modified(), s.modified()) {
            (Ok(bm), Ok(sm)) => sm > bm,
            _ => false,
        },
        (Err(_), _) => true,
        _ => false,
    };
    if stale {
        let src = src.ok_or("the Vision helper source is missing")?;
        std::fs::create_dir_all(&root).map_err(|e| format!("tools cache: {e}"))?;
        let tmp = root.join(format!("vision_probe.tmp{}", tmp_suffix()));
        let out = Command::new("swiftc")
            .arg("-O")
            .arg(&src)
            .arg("-o")
            .arg(&tmp)
            .output()
            .map_err(|e| format!("swiftc not available: {e}"))?;
        if !out.status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err("swiftc failed to build the Vision helper".into());
        }
        std::fs::rename(&tmp, &bin).map_err(|e| format!("tools cache: {e}"))?;
    }
    Ok(bin)
}

/// Probe `file` with Apple Vision (cached by sha256 in `probe.json`).
pub fn probe(file: &CheckedFile, config: &ServerConfig) -> Result<Probe, String> {
    let dir = cache_dir(config, &file.sha256);
    let cached = dir.join("probe.json");
    if let Ok(text) = std::fs::read_to_string(&cached) {
        if let Ok(p) = serde_json::from_str::<Probe>(&text) {
            return Ok(p);
        }
    }
    let bin = vision_helper(config)?;
    let out = Command::new(&bin)
        .arg(&file.path)
        .output()
        .map_err(|e| format!("Vision helper: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Vision helper: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let p: Probe =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("Vision helper: {e}"))?;
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(text) = serde_json::to_string(&p) {
            let _ = std::fs::write(&cached, text);
        }
    }
    Ok(p)
}

/// A PNG copy of a WebP image (the engine's manifests take png/jpg), cached.
pub fn png_copy(file: &CheckedFile, config: &ServerConfig) -> Result<PathBuf, String> {
    let out = cache_dir(config, &file.sha256).join("image.png");
    if out.is_file() {
        return Ok(out);
    }
    let bin = vision_helper(config).map_err(|_| "WebP images need macOS here".to_string())?;
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("image cache: {e}"))?;
    }
    let res = Command::new(&bin)
        .arg(&file.path)
        .arg("--png")
        .arg(&out)
        .output()
        .map_err(|e| format!("Vision helper: {e}"))?;
    if !res.status.success() || !out.is_file() {
        return Err("could not convert the WebP image".into());
    }
    Ok(out)
}

/// Cut out the subject of `file` with `motion-engine matte --crop` (cached
/// as `cutout.png` by sha256).
pub fn cutout(file: &CheckedFile, config: &ServerConfig) -> Result<PathBuf, String> {
    let out = cache_dir(config, &file.sha256).join("cutout.png");
    if out.is_file() {
        return Ok(out);
    }
    if !config.engine.is_file() {
        return Err("the engine binary is missing".into());
    }
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("image cache: {e}"))?;
    }
    let tmp = out.with_file_name(format!("cutout.tmp{}.png", tmp_suffix()));
    let res = config
        .engine_command()
        .arg("matte")
        .arg(&file.path)
        .arg("-o")
        .arg(&tmp)
        .arg("--crop")
        .output()
        .map_err(|e| format!("matte: {e}"))?;
    if !res.status.success() || !tmp.is_file() {
        let _ = std::fs::remove_file(&tmp);
        let err = String::from_utf8_lossy(&res.stderr);
        return Err(if err.contains("no subject") {
            "no subject found to cut out".to_string()
        } else {
            "the cutout failed".to_string()
        });
    }
    std::fs::rename(&tmp, &out).map_err(|e| format!("image cache: {e}"))?;
    Ok(out)
}

/// The header of an image file on disk.
pub fn read_header(path: &Path) -> Option<ImageHeader> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 256 * 1024];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    image_header(&buf)
}

/// Labels for a plan line: `blue_sky` → `blue sky`.
pub fn label_text(labels: &[String], k: usize) -> String {
    labels
        .iter()
        .take(k)
        .map(|l| l.replace('_', " "))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convention_names() {
        assert_eq!(convention("beat1.jpg"), Some(Convention::Beat(1, None)));
        assert_eq!(
            convention("Beat2_Person.PNG"),
            Some(Convention::Beat(2, Some(ImageRole::Person)))
        );
        assert_eq!(
            convention("beat_12-place.webp"),
            Some(Convention::Beat(12, Some(ImageRole::Place)))
        );
        assert_eq!(convention("background.jpeg"), Some(Convention::Background));
        assert_eq!(convention("beach.jpg"), None);
        assert_eq!(convention("beat3_banana.jpg"), None);
        assert_eq!(convention("beat1.gif"), None);
    }

    #[test]
    fn headers() {
        // Minimal PNG: signature + IHDR (RGBA, 9000 × 10).
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend([0, 0, 0, 13]);
        png.extend(b"IHDR");
        png.extend(9000u32.to_be_bytes());
        png.extend(10u32.to_be_bytes());
        png.extend([8, 6, 0, 0, 0]);
        let h = image_header(&png).unwrap();
        assert_eq!(
            (h.format, h.width, h.height, h.alpha),
            (ImageFormat::Png, 9000, 10, true)
        );
        assert!(image_header(b"hello world, not an image").is_none());
        // VP8X WebP 300 × 200 with alpha.
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend([10, 0, 0, 0, 0x10, 0, 0, 0]);
        webp.extend([43, 1, 0, 199, 0, 0]);
        let h = image_header(&webp).unwrap();
        assert_eq!(
            (h.format, h.width, h.height, h.alpha),
            (ImageFormat::Webp, 300, 200, true)
        );
    }

    #[test]
    fn role_guess() {
        let p = |w, h, opaque: bool, humans, area| Probe {
            width: w,
            height: h,
            has_alpha: !opaque,
            opaque,
            humans,
            human_area_max: area,
            labels: vec![],
        };
        assert_eq!(guess_role(&p(400, 800, true, 1, 0.5)), ImageRole::Person);
        assert_eq!(guess_role(&p(1600, 900, true, 1, 0.02)), ImageRole::Place);
        assert_eq!(guess_role(&p(1600, 900, false, 0, 0.0)), ImageRole::Object);
        assert_eq!(guess_role(&p(800, 800, true, 0, 0.0)), ImageRole::Object);
    }
}
