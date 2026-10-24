use std::fs;
use std::io::Write;

use bluelink_core::{Vault, VaultError};
use tempfile::tempdir;

#[test]
fn open_creates_bluelink_meta() {
    let dir = tempdir().unwrap();
    let vault = Vault::open(dir.path()).unwrap();
    assert!(dir.path().join(".bluelink/device.json").is_file());
    assert!(!vault.device().device_id.is_empty());
    assert!(!vault.device().private_key.is_empty());
}

#[test]
fn open_corrupt_meta_errors() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".bluelink")).unwrap();
    fs::write(dir.path().join(".bluelink/device.json"), "{not json").unwrap();
    assert!(matches!(
        Vault::open(dir.path()),
        Err(VaultError::CorruptMeta)
    ));
}

#[test]
fn rejects_dangerous_paths_on_write() {
    let dir = tempdir().unwrap();
    let mut vault = Vault::open(dir.path()).unwrap();
    for p in [
        "../x.md",
        "/etc/x.md",
        ".bluelink/trusted.json",
        r"foo\bar.md",
        "note",
        "a/../b.md",
    ] {
        let err = vault.write(p, "x").unwrap_err();
        assert!(
            matches!(err, VaultError::InvalidPath),
            "{p:?} => {err:?}"
        );
    }
}

#[test]
fn create_empty_folder() {
    let dir = tempdir().unwrap();
    let vault = Vault::open(dir.path()).unwrap();
    vault.create_folder("inbox").unwrap();
    vault.create_folder("projects/work").unwrap();
    let folders = vault.list_folders().unwrap();
    assert!(folders.iter().any(|f| f == "inbox"));
    assert!(folders.iter().any(|f| f == "projects"));
    assert!(folders.iter().any(|f| f == "projects/work"));
    assert!(vault.create_folder("../x").is_err());
}

#[test]
fn crud_roundtrip() {
    let dir = tempdir().unwrap();
    let mut vault = Vault::open(dir.path()).unwrap();

    let meta = vault.write("inbox/todo.md", "# Todo\n").unwrap();
    assert_eq!(meta.rel_path, "inbox/todo.md");
    assert_eq!(meta.size, "# Todo\n".len() as u64);
    assert!(!meta.sha256.is_empty());

    assert_eq!(vault.read("inbox/todo.md").unwrap(), "# Todo\n");

    let list = vault.list().unwrap();
    assert!(list.iter().any(|n| n.rel_path == "inbox/todo.md"));
    assert!(vault.list_folders().unwrap().iter().any(|f| f == "inbox"));

    let renamed = vault.rename("inbox/todo.md", "done.md").unwrap();
    assert_eq!(renamed.rel_path, "done.md");
    assert!(vault.read("inbox/todo.md").is_err());
    assert_eq!(vault.read("done.md").unwrap(), "# Todo\n");

    vault.delete("done.md").unwrap();
    assert!(vault.read("done.md").is_err());
    assert!(vault.list().unwrap().is_empty());
}

#[test]
fn atomic_write_leaves_complete_file() {
    let dir = tempdir().unwrap();
    let mut vault = Vault::open(dir.path()).unwrap();
    let body = "hello world\n".repeat(1000);
    vault.write("big.md", &body).unwrap();
    let on_disk = fs::read_to_string(dir.path().join("big.md")).unwrap();
    assert_eq!(on_disk, body);
    // No leftover tmp files in vault root.
    for entry in fs::read_dir(dir.path()).unwrap() {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        assert!(
            !name.contains(".tmp-"),
            "leftover temp file: {name}"
        );
    }
}

#[test]
fn wikilinks_resolve_and_backlinks() {
    let dir = tempdir().unwrap();
    let mut vault = Vault::open(dir.path()).unwrap();
    vault
        .write("Folder/Note.md", "target note")
        .unwrap();
    vault
        .write("index.md", "see [[Folder/Note|алиас]] and [[note]]\n")
        .unwrap();
    vault.write("note.md", "root note").unwrap();

    assert_eq!(
        vault.index().resolve("Folder/Note|алиас").as_deref(),
        Some("Folder/Note.md")
    );
    assert_eq!(vault.index().resolve("note").as_deref(), Some("note.md"));
    assert_eq!(
        vault.index().backlinks("Folder/Note.md"),
        vec!["index.md".to_string()]
    );
}

#[test]
fn non_utf8_read_is_not_utf8() {
    let dir = tempdir().unwrap();
    let mut vault = Vault::open(dir.path()).unwrap();
    vault.write("bin.md", "placeholder").unwrap();

    let mut f = fs::File::create(dir.path().join("bin.md")).unwrap();
    f.write_all(&[0xff, 0xfe, 0x00, b'x']).unwrap();
    drop(f);

    // Re-open so index rebuild sees the file without panicking.
    let vault = Vault::open(dir.path()).unwrap();
    let err = vault.read("bin.md").unwrap_err();
    assert!(matches!(err, VaultError::NotUtf8));
}

#[test]
fn too_large_rejected() {
    let dir = tempdir().unwrap();
    let mut vault = Vault::open(dir.path()).unwrap();
    let big = "x".repeat((1024 * 1024) + 1);
    let err = vault.write("huge.md", &big).unwrap_err();
    assert!(matches!(err, VaultError::TooLarge));
}
