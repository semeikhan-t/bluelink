use chrono::Local;

/// Build conflict sibling path: `Name.conflict-<device>-<YYYY-MM-DD HH-MM>.md`
pub fn conflict_path(rel_path: &str, peer_name: &str) -> String {
    let safe_name = sanitize_device_name(peer_name);
    let stamp = Local::now().format("%Y-%m-%d %H-%M").to_string();

    let (dir, stem) = split_md(rel_path);
    let file = format!("{stem}.conflict-{safe_name}-{stamp}.md");
    if dir.is_empty() {
        file
    } else {
        format!("{dir}/{file}")
    }
}

fn split_md(rel_path: &str) -> (&str, &str) {
    let without_ext = rel_path.strip_suffix(".md").unwrap_or(rel_path);
    match without_ext.rfind('/') {
        Some(i) => (&without_ext[..i], &without_ext[i + 1..]),
        None => ("", without_ext),
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
    use bluelink_core::validate_rel_path;

    #[test]
    fn conflict_path_shape_and_valid() {
        let p = conflict_path("inbox/todo.md", "Pixel 8");
        assert!(p.starts_with("inbox/todo.conflict-Pixel_8-"));
        assert!(p.ends_with(".md"));
        // stamp uses a space between date and time per ТЗ
        assert!(p.contains(' '));
        assert!(validate_rel_path(&p).is_ok());
    }
}
