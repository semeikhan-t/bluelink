use chrono::Local;

/// Build conflict sibling path: `Name.conflict-<device>-<YYYY-MM-DD HH-MM>.<ext>`
///
/// Keeps the original extension (`.md` or media). Stem is everything before the
/// last `.` in the file name.
pub fn conflict_path(rel_path: &str, peer_name: &str) -> String {
    let safe_name = sanitize_device_name(peer_name);
    let stamp = Local::now().format("%Y-%m-%d %H-%M").to_string();

    let (dir, stem, ext) = split_rel(rel_path);
    let file = format!("{stem}.conflict-{safe_name}-{stamp}.{ext}");
    if dir.is_empty() {
        file
    } else {
        format!("{dir}/{file}")
    }
}

fn split_rel(rel_path: &str) -> (&str, &str, &str) {
    let (dir, name) = match rel_path.rfind('/') {
        Some(i) => (&rel_path[..i], &rel_path[i + 1..]),
        None => ("", rel_path),
    };
    // Stem must be non-empty; treat leading-dot-only names as extensionless.
    match name.rfind('.') {
        Some(i) if i > 0 => (dir, &name[..i], &name[i + 1..]),
        _ => (dir, name, "bin"),
    }
}

fn sanitize_device_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        return "peer".into();
    }
    // path segments must not start with '.'
    if out.starts_with('.') {
        format!("d{out}")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bluelink_core::{validate_rel_path, validate_vault_rel_path};

    #[test]
    fn conflict_path_shape_and_valid() {
        let p = conflict_path("inbox/todo.md", "Pixel 8");
        assert!(p.starts_with("inbox/todo.conflict-Pixel_8-"));
        assert!(p.ends_with(".md"));
        // stamp uses a space between date and time per ТЗ
        assert!(p.contains(' '));
        assert!(validate_rel_path(&p).is_ok());
    }

    #[test]
    fn conflict_path_media_keeps_extension() {
        let p = conflict_path("attachments/shot.png", "Phone");
        assert!(p.starts_with("attachments/shot.conflict-Phone-"));
        assert!(p.ends_with(".png"));
        assert!(validate_vault_rel_path(&p).is_ok());
    }
}
