//! Paste / drop / attach helpers and vault media path resolution for embeds.

use std::borrow::Cow;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use bluelink_core::{is_media_extension, Vault};
use chrono::Local;
use image::ImageEncoder;

/// Longest edge for rasters we store or decode for preview.
/// egui_extras `ImageCrateLoader` ignores SizeHint and decodes full-res on the UI thread.
pub const MAX_IMAGE_EDGE: u32 = 2048;

/// Raster formats we show inline (via our capped texture path, not raw file://).
pub fn is_previewable_image(ext: &str) -> bool {
    matches!(
        ext.trim_start_matches('.').to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp"
    )
}

/// If either edge exceeds [`MAX_IMAGE_EDGE`], thumbnail and re-encode as PNG.
/// Otherwise returns the original bytes unchanged.
pub fn cap_image_bytes(bytes: &[u8]) -> Cow<'_, [u8]> {
    let Ok(img) = image::load_from_memory(bytes) else {
        return Cow::Borrowed(bytes);
    };
    if img.width().max(img.height()) <= MAX_IMAGE_EDGE {
        return Cow::Borrowed(bytes);
    }
    let img = img.thumbnail(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE);
    let mut out = Vec::new();
    if img
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .is_err()
    {
        return Cow::Borrowed(bytes);
    }
    Cow::Owned(out)
}

/// Decode path to an RGBA [`egui::ColorImage`], thumbnailing if needed.
/// Call off the UI thread — full decode can take seconds on large files.
pub fn load_capped_color_image(path: &Path) -> Result<egui::ColorImage, String> {
    let img = image::open(path).map_err(|e| e.to_string())?;
    let img = if img.width().max(img.height()) > MAX_IMAGE_EDGE {
        img.thumbnail(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        size,
        rgba.as_raw(),
    ))
}

/// Resolve `![[target]]` / image dest to an absolute path under the vault.
///
/// Paint-path safe: no `canonicalize`, no `list_files` (which reads+SHA-256s every
/// vault file). Those made document scroll hitch whenever an embed was open.
pub fn resolve_media(vault: &Vault, target: &str) -> Option<PathBuf> {
    let target = target.trim().trim_start_matches('/');
    if target.is_empty() {
        return None;
    }

    let root = vault.root();

    // Validated relative path — `..` / absolute already rejected by path rules.
    if bluelink_core::validate_vault_rel_path(target).is_ok() {
        let direct = root.join(target);
        if direct.is_file() {
            return Some(direct);
        }
    }

    find_media_by_basename(root, target)
}

/// Basename search via directory walk only (no file reads / hashes).
fn find_media_by_basename(root: &Path, target: &str) -> Option<PathBuf> {
    let want = Path::new(target)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(target)
        .to_ascii_lowercase();

    let mut best: Option<(bool, usize, PathBuf)> = None;
    let _ = walk_media_files(root, "", &mut |rel, abs| {
        let Some(name) = abs.file_name().and_then(|f| f.to_str()) else {
            return;
        };
        if !name.eq_ignore_ascii_case(&want) {
            return;
        }
        // Prefer attachments/, then shorter rel path (same as previous sort key).
        let key = (!rel.starts_with("attachments/"), rel.len());
        let replace = match &best {
            None => true,
            Some((bad_att, len, _)) => (key.0, key.1) < (*bad_att, *len),
        };
        if replace {
            best = Some((key.0, key.1, abs.to_path_buf()));
        }
    });
    best.map(|(_, _, p)| p)
}

fn walk_media_files(
    dir: &Path,
    rel_prefix: &str,
    visit: &mut dyn FnMut(&str, &Path),
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let rel = if rel_prefix.is_empty() {
            name.to_string()
        } else {
            format!("{rel_prefix}/{name}")
        };
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            walk_media_files(&path, &rel, visit)?;
        } else if ft.is_file() {
            let ext = Path::new(name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            if is_media_extension(ext) {
                visit(&rel, &path);
            }
        }
    }
    Ok(())
}

/// `file://` URL for opening a local path (not for egui image decode).
pub fn file_url(path: &Path) -> String {
    // Skip canonicalize on the paint path — vault resolve already validated the file.
    format!("file://{}", path.display())
}

pub fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

/// Obsidian-style stem for clipboard screenshots.
pub fn pasted_image_stem() -> String {
    format!("Pasted image {}", Local::now().format("%Y%m%d%H%M%S"))
}

/// Encode clipboard RGBA pixels as PNG bytes (thumbnail if over [`MAX_IMAGE_EDGE`]).
pub fn encode_clipboard_png(width: usize, height: usize, rgba: &[u8]) -> Option<Vec<u8>> {
    let expected = width.checked_mul(height)?.checked_mul(4)?;
    if rgba.len() < expected || width == 0 || height == 0 {
        return None;
    }
    let raw = image::RgbaImage::from_raw(width as u32, height as u32, rgba[..expected].to_vec())?;
    let mut img = image::DynamicImage::ImageRgba8(raw);
    if img.width().max(img.height()) > MAX_IMAGE_EDGE {
        img = img.thumbnail(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE);
    }
    let rgba8 = img.to_rgba8();
    let mut out = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(Cursor::new(&mut out));
    encoder
        .write_image(
            rgba8.as_raw(),
            rgba8.width(),
            rgba8.height(),
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(out)
}

/// Read image bytes from the OS clipboard when present.
pub fn clipboard_image_png() -> Option<Vec<u8>> {
    let mut cb = arboard::Clipboard::new().ok()?;
    let img = cb.get_image().ok()?;
    encode_clipboard_png(img.width, img.height, img.bytes.as_ref())
}

/// Save bytes via vault and return the vault-relative path for `![[…]]`.
/// Raster images are capped to [`MAX_IMAGE_EDGE`] (re-encoded as PNG when resized).
pub fn save_bytes(
    vault: &Vault,
    preferred_stem: &str,
    ext: &str,
    bytes: &[u8],
    active_note: &str,
) -> Result<String, String> {
    let (ext, data): (&str, Cow<'_, [u8]>) = if is_previewable_image(ext) {
        match cap_image_bytes(bytes) {
            Cow::Owned(v) => ("png", Cow::Owned(v)),
            borrowed => (ext, borrowed),
        }
    } else {
        (ext, Cow::Borrowed(bytes))
    };
    vault
        .save_attachment(preferred_stem, ext, data.as_ref(), active_note)
        .map_err(|e| e.to_string())
}

/// Ingest a filesystem path (drop / attach picker). Returns embed rel path.
pub fn ingest_path(vault: &Vault, path: &Path, active_note: &str) -> Result<String, String> {
    let ext = extension_of(path).ok_or_else(|| "missing extension".to_string())?;
    if !is_media_extension(&ext) {
        return Err(format!("unsupported type .{ext}"));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("attachment");
    save_bytes(vault, stem, &ext, &bytes, active_note)
}

/// Insert `![[rel]]` at the active line (or end of buffer).
pub fn insert_embed(buffer: &mut String, active_line: Option<usize>, rel: &str) {
    let embed = format!("![[{rel}]]");
    let ranges = line_byte_ranges(buffer);
    if let Some(i) = active_line {
        if let Some(&(start, end)) = ranges.get(i) {
            let line_empty = start == end;
            if line_empty {
                buffer.replace_range(start..end, &embed);
            } else {
                buffer.insert_str(end, &format!("\n{embed}"));
            }
            return;
        }
    }
    if !buffer.is_empty() && !buffer.ends_with('\n') {
        buffer.push('\n');
    }
    buffer.push_str(&embed);
    buffer.push('\n');
}

/// Set/replace Obsidian size suffix on `![[target…]]`. Prefers `line_idx`, then whole buffer.
pub fn rewrite_embed_size(
    buffer: &mut String,
    line_idx: Option<usize>,
    target: &str,
    width: u32,
    height: Option<u32>,
) -> bool {
    if width == 0 {
        return false;
    }
    let size = match height {
        Some(h) if h > 0 => format!("{width}x{h}"),
        _ => width.to_string(),
    };
    let ranges = line_byte_ranges(buffer);
    if let Some(i) = line_idx {
        if let Some(&(start, end)) = ranges.get(i) {
            if rewrite_embed_in_range(buffer, start, end, target, &size) {
                return true;
            }
        }
    }
    rewrite_embed_in_range(buffer, 0, buffer.len(), target, &size)
}

fn rewrite_embed_in_range(
    buffer: &mut String,
    start: usize,
    end: usize,
    target: &str,
    size: &str,
) -> bool {
    let marker = format!("![[{target}");
    let hay = &buffer[start..end];
    let mut from = 0usize;
    while let Some(rel) = hay[from..].find(&marker) {
        let open = start + from + rel;
        let after_target = open + marker.len();
        let next = buffer.as_bytes().get(after_target).copied();
        // Exact target boundary: `![[target]]` or `![[target|…]]`.
        if next != Some(b'|') && next != Some(b']') {
            from += rel + marker.len();
            continue;
        }
        let Some(close_rel) = buffer[after_target..].find("]]") else {
            return false;
        };
        let close = after_target + close_rel;
        buffer.replace_range(after_target..close, &format!("|{size}"));
        return true;
    }
    false
}

fn line_byte_ranges(s: &str) -> Vec<(usize, usize)> {
    if s.is_empty() {
        return vec![(0, 0)];
    }
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            out.push((start, i));
            start = i + 1;
        }
    }
    out.push((start, s.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluelink_core::Vault;
    use tempfile::tempdir;

    #[test]
    fn insert_embed_on_empty_active_line() {
        let mut buf = "a\n\nc".to_string();
        insert_embed(&mut buf, Some(1), "attachments/x.png");
        assert_eq!(buf, "a\n![[attachments/x.png]]\nc");
    }

    #[test]
    fn insert_embed_after_nonempty_line() {
        let mut buf = "hello".to_string();
        insert_embed(&mut buf, Some(0), "attachments/x.png");
        assert_eq!(buf, "hello\n![[attachments/x.png]]");
    }

    #[test]
    fn resolve_by_path_and_basename() {
        let dir = tempdir().unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let rel = vault
            .save_attachment("shot", "png", b"PNGDATA", "note.md")
            .unwrap();
        assert!(resolve_media(&vault, &rel).is_some());
        assert!(resolve_media(&vault, "shot.png").is_some());
        assert!(resolve_media(&vault, "missing.png").is_none());
    }

    #[test]
    fn encode_tiny_png() {
        let rgba = [255u8, 0, 0, 255];
        let png = encode_clipboard_png(1, 1, &rgba).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn cap_image_bytes_thumbnails_oversized() {
        let big = image::RgbImage::from_pixel(
            MAX_IMAGE_EDGE + 10,
            8,
            image::Rgb([1, 2, 3]),
        );
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(big)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let capped = cap_image_bytes(&png);
        assert!(matches!(capped, Cow::Owned(_)));
        let img = image::load_from_memory(&capped).unwrap();
        assert!(img.width().max(img.height()) <= MAX_IMAGE_EDGE);
    }

    #[test]
    fn rewrite_adds_and_replaces_size() {
        let mut buf = "before ![[a.png]] after".to_string();
        assert!(rewrite_embed_size(&mut buf, Some(0), "a.png", 400, None));
        assert_eq!(buf, "before ![[a.png|400]] after");

        assert!(rewrite_embed_size(&mut buf, Some(0), "a.png", 400, Some(200)));
        assert_eq!(buf, "before ![[a.png|400x200]] after");

        assert!(rewrite_embed_size(&mut buf, Some(0), "a.png", 320, None));
        assert_eq!(buf, "before ![[a.png|320]] after");
    }

    #[test]
    fn rewrite_prefers_line_and_exact_target() {
        let mut buf = "![[a.png.bak]]\n![[a.png|90]]\n".to_string();
        assert!(rewrite_embed_size(&mut buf, Some(1), "a.png", 200, None));
        assert_eq!(buf, "![[a.png.bak]]\n![[a.png|200]]\n");
    }
}
