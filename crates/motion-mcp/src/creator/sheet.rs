//! Contact sheets (plan §9): rendered frames tiled into one PNG whose long
//! side is at most [`MAX_SIDE`] px, so a multimodal model can look at a whole
//! video (or k looks of one story) in a single image.
//!
//! The tiling is `ffmpeg` (`scale`, `hstack`, `vstack`, `pad`: no optional
//! filters). Labels are drawn here as small bitmap strips (a built-in 5x7
//! font) and stacked above their tile or row, so they never cover the picture
//! and do not depend on ffmpeg's `drawtext` (which many builds lack).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::proc;

/// Longest side of a sheet or a returned image, in pixels.
pub const MAX_SIDE: u32 = 1024;
/// Tiles per row before a sheet gets another row.
pub const MAX_PER_ROW: usize = 6;
/// Pixel scale of a tile label (glyphs are 5x7 before scaling).
pub const CELL_LABEL_SCALE: u32 = 2;
/// Pixel scale of a row label.
pub const ROW_LABEL_SCALE: u32 = 3;
/// ffmpeg deadline.
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(90);

const STRIP_BG: [u8; 3] = [0x1c, 0x1c, 0x1c];
const STRIP_FG: [u8; 3] = [0xf2, 0xf2, 0xf2];

/// One picture of a sheet, with an optional small label above it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub image: PathBuf,
    pub label: Option<String>,
}

/// One row of tiles, with an optional label strip above the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: Option<String>,
    pub cells: Vec<Cell>,
}

/// Sizes of a sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Tiles in the widest row.
    pub cols: usize,
    pub rows: usize,
    pub tile_w: u32,
    pub tile_h: u32,
    /// Height of the label strip above every tile (0 = no tile labels).
    pub cell_label_h: u32,
    /// Height of the label strip above every row (0 = no row labels).
    pub row_label_h: u32,
    pub width: u32,
    pub height: u32,
}

/// Height of a label strip drawn at `scale`.
pub fn strip_height(scale: u32) -> u32 {
    9 * scale + 2
}

/// How `n` tiles split into rows: at most [`MAX_PER_ROW`] per row, balanced
/// (7 tiles are 4 + 3, not 6 + 1). Returns the number of tiles in each row.
pub fn row_split(n: usize) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let rows = n.div_ceil(MAX_PER_ROW);
    let per = n.div_ceil(rows);
    let mut left = n;
    let mut out = Vec::new();
    while left > 0 {
        let take = per.min(left);
        out.push(take);
        left -= take;
    }
    out
}

/// The sheet's sizes for `cols` x `rows` tiles of a `frame_w` x `frame_h`
/// picture: the largest even tile width with both sheet sides within
/// [`MAX_SIDE`] (never above the picture's own width). `None` when nothing
/// sensible fits.
pub fn layout(
    cols: usize,
    rows: usize,
    frame_w: u32,
    frame_h: u32,
    cell_labels: bool,
    row_labels: bool,
) -> Option<Layout> {
    if cols == 0 || rows == 0 || frame_w == 0 || frame_h == 0 {
        return None;
    }
    let clh = if cell_labels {
        strip_height(CELL_LABEL_SCALE)
    } else {
        0
    };
    let rlh = if row_labels {
        strip_height(ROW_LABEL_SCALE)
    } else {
        0
    };
    let tile_h_of =
        |w: u32| ((w as u64 * frame_h as u64 + frame_w as u64 / 2) / frame_w as u64) as u32;
    let by_width = MAX_SIDE / cols as u32;
    let row_budget = (MAX_SIDE / rows as u32).checked_sub(clh + rlh)?;
    let by_height = (row_budget as u64 * frame_w as u64 / frame_h as u64) as u32;
    let mut w = by_width.min(by_height).min(frame_w) & !1;
    while w >= 16 {
        let h = tile_h_of(w);
        if rows as u32 * (h + clh + rlh) <= MAX_SIDE && cols as u32 * w <= MAX_SIDE {
            return Some(Layout {
                cols,
                rows,
                tile_w: w,
                tile_h: h,
                cell_label_h: clh,
                row_label_h: rlh,
                width: cols as u32 * w,
                height: rows as u32 * (h + clh + rlh),
            });
        }
        w -= 2;
    }
    None
}

/// The size of a PNG, from its header.
pub fn png_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut head = [0u8; 24];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head[..8] != b"\x89PNG\r\n\x1a\n" || &head[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let h = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    Some((w, h))
}

/// A P6 PPM of a label strip: `text` in the built-in font at `scale`, light on
/// dark, `width` px wide, cut where it would not fit.
pub fn label_ppm(text: &str, width: u32, scale: u32) -> Vec<u8> {
    let height = strip_height(scale);
    let mut px = vec![0u8; (width * height * 3) as usize];
    for chunk in px.chunks_exact_mut(3) {
        chunk.copy_from_slice(&STRIP_BG);
    }
    let pad = 2 * scale;
    let top = scale + 1;
    let mut x0 = pad;
    for ch in text.chars() {
        if x0 + 5 * scale + pad > width {
            break;
        }
        for (row, bits) in glyph(ch).iter().enumerate() {
            for col in 0..5u32 {
                if bits & (0b10000 >> col) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let x = x0 + col * scale + dx;
                        let y = top + row as u32 * scale + dy;
                        let at = ((y * width + x) * 3) as usize;
                        px[at..at + 3].copy_from_slice(&STRIP_FG);
                    }
                }
            }
        }
        x0 += 6 * scale;
    }
    let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
    out.extend_from_slice(&px);
    out
}

/// 5x7 glyph rows (bit 4 = leftmost column). Letters are drawn as capitals;
/// anything unknown is a question mark.
fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        ' ' => [0; 7],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        ',' => [0, 0, 0, 0, 0b01100, 0b00100, 0b01000],
        '-' => [0, 0, 0, 0b11111, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0b11111],
        ':' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0],
        '(' => [
            0b00010, 0b00100, 0b01000, 0b01000, 0b01000, 0b00100, 0b00010,
        ],
        ')' => [
            0b01000, 0b00100, 0b00010, 0b00010, 0b00010, 0b00100, 0b01000,
        ],
        '/' => [
            0b00001, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b10000,
        ],
        '+' => [0, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0],
        '#' => [
            0b01010, 0b01010, 0b11111, 0b01010, 0b11111, 0b01010, 0b01010,
        ],
        _ => [0b01110, 0b10001, 0b00001, 0b00110, 0b00100, 0, 0b00100],
    }
}

/// `[a][b]…<filter>=inputs=N[out]`, or a plain pass-through for one input
/// (hstack and vstack need at least two).
fn stack(filter: &str, inputs: &[String], out: &str) -> String {
    match inputs {
        [one] => format!("[{one}]null[{out}]"),
        many => {
            let ins: String = many.iter().map(|i| format!("[{i}]")).collect();
            format!("{ins}{filter}=inputs={}[{out}]", many.len())
        }
    }
}

/// The ffmpeg `-filter_complex` graph for `rows` laid out as `l`. Inputs are
/// the frames row by row, then the tile label strips, then the row label
/// strips (the same order [`build`] passes them).
fn graph(rows: &[Row], l: &Layout) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut frame_idx = 0usize;
    let n_frames: usize = rows.iter().map(|r| r.cells.len()).sum();
    let mut cell_label_idx = n_frames;
    let n_cell_labels: usize = rows
        .iter()
        .flat_map(|r| &r.cells)
        .filter(|c| c.label.is_some())
        .count();
    let mut row_label_idx = n_frames + n_cell_labels;
    let bg = format!(
        "0x{:02x}{:02x}{:02x}",
        STRIP_BG[0], STRIP_BG[1], STRIP_BG[2]
    );
    let mut row_names: Vec<String> = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let mut tiles: Vec<String> = Vec::new();
        for (c, cell) in row.cells.iter().enumerate() {
            let f = format!("f{r}_{c}");
            parts.push(format!(
                "[{frame_idx}:v]scale={}:{}:flags=area,format=rgb24[{f}]",
                l.tile_w, l.tile_h
            ));
            frame_idx += 1;
            if cell.label.is_some() {
                let lab = format!("l{r}_{c}");
                parts.push(format!("[{cell_label_idx}:v]format=rgb24[{lab}]"));
                cell_label_idx += 1;
                let t = format!("t{r}_{c}");
                parts.push(stack("vstack", &[lab, f], &t));
                tiles.push(t);
            } else if l.cell_label_h > 0 {
                // A sheet with some labelled tiles keeps every tile the same
                // height: pad the unlabelled ones on top.
                let t = format!("t{r}_{c}");
                parts.push(format!(
                    "[{f}]pad={}:{}:0:{}:color={bg}[{t}]",
                    l.tile_w,
                    l.tile_h + l.cell_label_h,
                    l.cell_label_h
                ));
                tiles.push(t);
            } else {
                tiles.push(f);
            }
        }
        let tile_h = l.tile_h + l.cell_label_h;
        let joined = format!("row{r}");
        parts.push(stack("hstack", &tiles, &joined));
        let padded = if row.cells.len() < l.cols {
            let p = format!("rowp{r}");
            parts.push(format!(
                "[{joined}]pad={}:{tile_h}:0:0:color={bg}[{p}]",
                l.width
            ));
            p
        } else {
            joined
        };
        if row.label.is_some() {
            let lab = format!("rl{r}");
            parts.push(format!("[{row_label_idx}:v]format=rgb24[{lab}]"));
            row_label_idx += 1;
            let full = format!("rowl{r}");
            parts.push(stack("vstack", &[lab, padded], &full));
            row_names.push(full);
        } else {
            row_names.push(padded);
        }
    }
    parts.push(stack("vstack", &row_names, "out"));
    parts.join(";")
}

/// Build the sheet for `rows` into `out` (PNG). Returns its size. Every
/// image must exist; all are scaled to the first one's shape.
pub fn build(rows: &[Row], out: &Path) -> Result<(u32, u32), String> {
    let first = rows
        .iter()
        .flat_map(|r| &r.cells)
        .next()
        .ok_or("no frames for the sheet")?;
    let (fw, fh) = png_size(&first.image)
        .ok_or_else(|| format!("{} is not a readable PNG", first.image.display()))?;
    let cols = rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    let cell_labels = rows
        .iter()
        .flat_map(|r| &r.cells)
        .any(|c| c.label.is_some());
    let row_labels = rows.iter().any(|r| r.label.is_some());
    let l = layout(cols, rows.len(), fw, fh, cell_labels, row_labels)
        .ok_or("the frames do not fit a sheet of 1024 px")?;
    let filter = graph(rows, &l);

    // Label strips as PPM files next to the output (removed afterwards).
    let work = out.with_extension("labels");
    std::fs::create_dir_all(&work).map_err(|e| format!("cannot write the sheet: {e}"))?;
    let result: Result<(), String> = (|| {
        let mut inputs: Vec<PathBuf> = rows
            .iter()
            .flat_map(|r| &r.cells)
            .map(|c| c.image.clone())
            .collect();
        for (i, text) in rows
            .iter()
            .flat_map(|r| &r.cells)
            .filter_map(|c| c.label.as_deref())
            .enumerate()
        {
            let p = work.join(format!("cell_{i}.ppm"));
            std::fs::write(&p, label_ppm(text, l.tile_w, CELL_LABEL_SCALE))
                .map_err(|e| format!("cannot write the sheet: {e}"))?;
            inputs.push(p);
        }
        for (i, text) in rows.iter().filter_map(|r| r.label.as_deref()).enumerate() {
            let p = work.join(format!("row_{i}.ppm"));
            std::fs::write(&p, label_ppm(text, l.width, ROW_LABEL_SCALE))
                .map_err(|e| format!("cannot write the sheet: {e}"))?;
            inputs.push(p);
        }
        let tmp = out.with_extension("tmp.png");
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
        for i in &inputs {
            cmd.arg("-i").arg(i);
        }
        cmd.arg("-filter_complex")
            .arg(&filter)
            .args(["-map", "[out]", "-frames:v", "1"])
            .arg(&tmp);
        proc::run("sheet", cmd, FFMPEG_TIMEOUT)?;
        std::fs::rename(&tmp, out).map_err(|e| format!("cannot write the sheet: {e}"))?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    result?;
    let size = png_size(out).ok_or("the sheet was not written")?;
    if size.0.max(size.1) > MAX_SIDE {
        return Err(format!("the sheet came out {}x{} px", size.0, size.1));
    }
    Ok(size)
}

/// `src` as a PNG with its long side at most [`MAX_SIDE`] (copied when it
/// already is).
pub fn fit_image(src: &Path, dst: &Path) -> Result<(u32, u32), String> {
    let (w, h) = png_size(src).ok_or_else(|| format!("{} is not a readable PNG", src.display()))?;
    if w.max(h) <= MAX_SIDE {
        std::fs::copy(src, dst).map_err(|e| format!("cannot write the image: {e}"))?;
        return Ok((w, h));
    }
    let f = MAX_SIDE as f64 / w.max(h) as f64;
    let (nw, nh) = (
        ((w as f64 * f).round() as u32).max(2) & !1,
        ((h as f64 * f).round() as u32).max(2) & !1,
    );
    let tmp = dst.with_extension("tmp.png");
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(src)
        .arg("-vf")
        .arg(format!("scale={nw}:{nh}:flags=area,format=rgb24"))
        .args(["-frames:v", "1"])
        .arg(&tmp);
    proc::run("image", cmd, FFMPEG_TIMEOUT)?;
    std::fs::rename(&tmp, dst).map_err(|e| format!("cannot write the image: {e}"))?;
    Ok((nw, nh))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_balanced_and_capped() {
        assert_eq!(row_split(0), Vec::<usize>::new());
        assert_eq!(row_split(1), vec![1]);
        assert_eq!(row_split(5), vec![5]);
        assert_eq!(row_split(6), vec![6]);
        assert_eq!(row_split(7), vec![4, 3]);
        assert_eq!(row_split(10), vec![5, 5]);
        assert_eq!(row_split(12), vec![6, 6]);
        assert!(row_split(13).iter().all(|n| *n <= MAX_PER_ROW));
    }

    #[test]
    fn layouts_stay_within_1024() {
        for (cols, rows, w, h, cl, rl) in [
            (4, 1, 1080, 1920, true, false),
            (6, 1, 1080, 1920, true, false),
            (6, 2, 1920, 1080, true, false),
            (4, 3, 1080, 1920, false, true),
            (4, 4, 1080, 1350, false, true),
            (1, 1, 1080, 1080, false, false),
            (2, 1, 640, 360, true, false),
        ] {
            let l = layout(cols, rows, w, h, cl, rl).unwrap();
            assert!(l.width <= MAX_SIDE && l.height <= MAX_SIDE, "{l:?}");
            assert_eq!(l.tile_w % 2, 0);
            assert!(l.tile_w <= w);
            // The tile keeps the picture's shape (within a pixel).
            let want = l.tile_w as f64 * h as f64 / w as f64;
            assert!((l.tile_h as f64 - want).abs() <= 1.0, "{l:?}");
        }
        // One vertical row of 4: width-bound at 256.
        let l = layout(4, 1, 1080, 1920, true, false).unwrap();
        assert_eq!((l.tile_w, l.width), (256, 1024));
        assert_eq!(l.cell_label_h, strip_height(CELL_LABEL_SCALE));
        assert!(layout(0, 1, 10, 10, false, false).is_none());
    }

    #[test]
    fn small_frames_are_not_upscaled() {
        let l = layout(2, 1, 320, 180, false, false).unwrap();
        assert_eq!(l.tile_w, 320);
    }

    #[test]
    fn label_strips_are_ppm_with_text() {
        let ppm = label_ppm("Beat 3", 200, 2);
        let header = b"P6\n200 20\n255\n";
        assert!(ppm.starts_with(header));
        assert_eq!(ppm.len(), header.len() + 200 * 20 * 3);
        let px = &ppm[header.len()..];
        // Both the background and the text colour are present.
        assert!(px.chunks(3).any(|c| c == STRIP_BG));
        assert!(px.chunks(3).any(|c| c == STRIP_FG));
        // Text that does not fit is cut, never overruns the strip.
        let long = label_ppm(&"W".repeat(500), 100, 3);
        assert_eq!(long.len(), b"P6\n100 29\n255\n".len() + 100 * 29 * 3);
        // The same text always draws the same pixels.
        assert_eq!(ppm, label_ppm("BEAT 3", 200, 2));
    }

    #[test]
    fn every_label_character_has_a_glyph() {
        for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .,-_:()/+".chars() {
            assert!(glyph(c).iter().any(|r| *r != 0) || c == ' ', "{c}");
        }
        assert_eq!(glyph('a'), glyph('A'));
    }

    #[test]
    fn graph_stacks_labels_above_tiles_and_rows() {
        let cell = |n: &str, label: Option<&str>| Cell {
            image: PathBuf::from(n),
            label: label.map(str::to_string),
        };
        let rows = vec![
            Row {
                label: Some("1 A".into()),
                cells: vec![cell("a", None), cell("b", None)],
            },
            Row {
                label: Some("2 B".into()),
                cells: vec![cell("c", None)],
            },
        ];
        let l = layout(2, 2, 1080, 1920, false, true).unwrap();
        let g = graph(&rows, &l);
        // 3 frames (inputs 0-2) and 2 row labels (inputs 3-4).
        assert!(g.contains("[0:v]scale=") && g.contains("[2:v]scale="));
        assert!(g.contains("[3:v]format=rgb24[rl0]") && g.contains("[4:v]format=rgb24[rl1]"));
        // The short second row is padded to the sheet width; the single tile
        // is passed through (no one-input hstack).
        assert!(g.contains("[f1_0]null[row1]"), "{g}");
        assert!(g.contains("[row1]pad="), "{g}");
        assert!(g.ends_with("[rowl0][rowl1]vstack=inputs=2[out]"), "{g}");

        // Tile labels: labelled tiles stack a strip, a mixed row pads the rest.
        let rows = vec![Row {
            label: None,
            cells: vec![cell("a", Some("BEAT 1")), cell("b", None)],
        }];
        let l = layout(2, 1, 1080, 1920, true, false).unwrap();
        let g = graph(&rows, &l);
        assert!(g.contains("[l0_0][f0_0]vstack=inputs=2[t0_0]"), "{g}");
        assert!(g.contains("[f0_1]pad="), "{g}");
        assert!(
            g.ends_with("[t0_0][t0_1]hstack=inputs=2[row0];[row0]null[out]"),
            "{g}"
        );
    }

    #[test]
    fn png_sizes_come_from_the_header() {
        let dir = std::env::temp_dir().join(format!("sheet-png-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.png");
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&1080u32.to_be_bytes());
        bytes.extend_from_slice(&1920u32.to_be_bytes());
        std::fs::write(&p, &bytes).unwrap();
        assert_eq!(png_size(&p), Some((1080, 1920)));
        std::fs::write(&p, b"not a png at all, really not").unwrap();
        assert_eq!(png_size(&p), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A solid-colour PNG of `w`x`h` made by ffmpeg (None without ffmpeg).
    fn solid_png(dir: &Path, name: &str, w: u32, h: u32) -> Option<PathBuf> {
        let p = dir.join(name);
        let ok = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg(format!("color=c=0x3366cc:s={w}x{h}"))
            .args(["-frames:v", "1"])
            .arg(&p)
            .status()
            .ok()?
            .success();
        ok.then_some(p)
    }

    #[test]
    fn real_sheets_fit_1024_in_every_shape() {
        let dir = std::env::temp_dir().join(format!("sheet-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let Some(tall) = solid_png(&dir, "tall.png", 540, 960) else {
            eprintln!("skipping real_sheets_fit_1024_in_every_shape: no ffmpeg");
            return;
        };
        let wide = solid_png(&dir, "wide.png", 960, 540).unwrap();
        for (name, image, counts, cell_labels, row_labels) in [
            ("a", &tall, vec![4usize], true, false),
            ("b", &tall, row_split(7), true, false),
            ("c", &wide, row_split(12), true, false),
            ("d", &tall, vec![4, 4, 4], false, true),
            ("e", &wide, vec![3, 2], true, true),
            ("f", &tall, vec![1], true, false),
        ] {
            let rows: Vec<Row> = counts
                .iter()
                .enumerate()
                .map(|(r, n)| Row {
                    label: row_labels.then(|| format!("{} look_{r} (auto)", r + 1)),
                    cells: (0..*n)
                        .map(|c| Cell {
                            image: image.clone(),
                            label: cell_labels.then(|| format!("beat {} 1.{c}s", c + 1)),
                        })
                        .collect(),
                })
                .collect();
            let out = dir.join(format!("sheet_{name}.png"));
            let (w, h) = build(&rows, &out).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(w.max(h) <= MAX_SIDE && w.max(h) >= 600, "{name}: {w}x{h}");
            assert_eq!(png_size(&out), Some((w, h)), "{name}");
            // The label folder is gone.
            assert!(!out.with_extension("labels").exists());
        }
        // A long picture shrinks to a long side of 1024.
        let big = solid_png(&dir, "big.png", 1080, 1920).unwrap();
        let fitted = dir.join("fitted.png");
        assert_eq!(fit_image(&big, &fitted).unwrap(), (576, 1024));
        assert_eq!(png_size(&fitted), Some((576, 1024)));
        let small = dir.join("small_copy.png");
        assert_eq!(fit_image(&tall, &small).unwrap(), (540, 960));
        // A missing frame is an error, not a blank tile.
        let rows = vec![Row {
            label: None,
            cells: vec![Cell {
                image: dir.join("nope.png"),
                label: None,
            }],
        }];
        assert!(build(&rows, &dir.join("x.png")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
