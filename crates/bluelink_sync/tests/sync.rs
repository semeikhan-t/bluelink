use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bluelink_core::Vault;
use bluelink_sync::{
    host_accept, host_session, join, run_sync, Loopback, PairingError, Role, SyncError,
    SyncOptions, TrustStore,
};

fn open_vault() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path()).unwrap();
    (dir, vault)
}

#[tokio::test]
async fn pairing_ok() {
    let (_d1, v1) = open_vault();
    let (_d2, v2) = open_vault();
    let root1 = v1.root().to_path_buf();
    let root2 = v2.root().to_path_buf();
    let id1 = v1.device().device_id.clone();
    let id2 = v2.device().device_id.clone();

    let (mut t_host, mut t_join) = Loopback::pair();
    let session = host_session().unwrap();
    let code = session.code.clone();
    let expires = session.expires_at;
    let mut attempts = session.attempts_left;

    let root1_host = root1.clone();
    let host = tokio::spawn(async move {
        let vault = Vault::open(&root1_host).unwrap();
        host_accept(&mut t_host, &vault, &code, expires, &mut attempts).await
    });

    let join_vault = Vault::open(&root2).unwrap();
    let join_res = join(&mut t_join, &join_vault, &session.code).await.unwrap();
    let host_res = host.await.unwrap().unwrap();

    assert_eq!(join_res.device_id, id1);
    assert_eq!(host_res.device_id, id2);

    let trust1 = TrustStore::load(&root1).unwrap();
    let trust2 = TrustStore::load(&root2).unwrap();
    assert!(trust1.get(&id2).is_some());
    assert!(trust2.get(&id1).is_some());
}

#[tokio::test]
async fn pairing_wrong_code_x3() {
    let (_d1, v1) = open_vault();
    let (_d2, v2) = open_vault();
    let root1 = v1.root().to_path_buf();
    let root2 = v2.root().to_path_buf();

    let session = host_session().unwrap();
    let real_code = session.code.clone();
    let expires = session.expires_at;
    let attempts = Arc::new(std::sync::Mutex::new(session.attempts_left));

    for i in 0..3 {
        let (mut t_host, mut t_join) = Loopback::pair();
        let code_host = real_code.clone();
        let attempts_h = Arc::clone(&attempts);
        let root_h = root1.clone();

        let host = tokio::spawn(async move {
            let vault = Vault::open(&root_h).unwrap();
            let mut left = *attempts_h.lock().unwrap();
            let r = host_accept(&mut t_host, &vault, &code_host, expires, &mut left).await;
            *attempts_h.lock().unwrap() = left;
            r
        });

        let join_vault = Vault::open(&root2).unwrap();
        let wrong = "000000";
        assert_ne!(wrong, real_code);
        let join_err = join(&mut t_join, &join_vault, wrong).await.unwrap_err();
        let host_err = host.await.unwrap();

        assert!(
            matches!(
                join_err,
                PairingError::WrongCode
                    | PairingError::Expired
                    | PairingError::Transport(_)
                    | PairingError::Crypto(_)
            ),
            "join attempt {i}: {join_err:?}"
        );
        assert!(host_err.is_err(), "host attempt {i}: {host_err:?}");
    }

    assert_eq!(*attempts.lock().unwrap(), 0);

    let trust1 = TrustStore::load(&root1).unwrap();
    let trust2 = TrustStore::load(&root2).unwrap();
    assert!(trust1.list().is_empty());
    assert!(trust2.list().is_empty());
}

async fn pair_vaults(root_host: &std::path::Path, root_join: &std::path::Path) {
    let (mut t_host, mut t_join) = Loopback::pair();
    let session = host_session().unwrap();
    let code = session.code.clone();
    let mut attempts = session.attempts_left;
    let expires = session.expires_at;
    let root_h = root_host.to_path_buf();

    let host = tokio::spawn(async move {
        let vault = Vault::open(&root_h).unwrap();
        host_accept(&mut t_host, &vault, &code, expires, &mut attempts).await
    });

    let join_vault = Vault::open(root_join).unwrap();
    join(&mut t_join, &join_vault, &session.code)
        .await
        .unwrap();
    host.await.unwrap().unwrap();
}

#[tokio::test]
async fn sync_new_modify_delete() {
    let (_d_pc, v_pc0) = open_vault();
    let (_d_phone, v_phone0) = open_vault();
    let root_pc = v_pc0.root().to_path_buf();
    let root_phone = v_phone0.root().to_path_buf();

    pair_vaults(&root_pc, &root_phone).await;

    let mut v_pc = Vault::open(&root_pc).unwrap();
    let mut v_phone = Vault::open(&root_phone).unwrap();

    v_pc.write("only_pc.md", "from pc").unwrap();
    v_phone.write("only_phone.md", "from phone").unwrap();
    v_pc.write("shared.md", "v1").unwrap();
    v_phone.write("shared.md", "v1").unwrap();

    sync_once(&root_pc, &root_phone).await.unwrap();

    v_pc = Vault::open(&root_pc).unwrap();
    v_phone = Vault::open(&root_phone).unwrap();
    assert_eq!(v_phone.read("only_pc.md").unwrap(), "from pc");
    assert_eq!(v_pc.read("only_phone.md").unwrap(), "from phone");

    v_phone.write("shared.md", "v2 phone").unwrap();
    v_pc.delete("only_phone.md").unwrap();

    sync_once(&root_pc, &root_phone).await.unwrap();

    v_pc = Vault::open(&root_pc).unwrap();
    v_phone = Vault::open(&root_phone).unwrap();
    assert_eq!(v_pc.read("shared.md").unwrap(), "v2 phone");
    assert!(v_phone.read("only_phone.md").is_err());
}

#[tokio::test]
async fn sync_conflict() {
    let (_d_pc, v_pc0) = open_vault();
    let (_d_phone, v_phone0) = open_vault();
    let root_pc = v_pc0.root().to_path_buf();
    let root_phone = v_phone0.root().to_path_buf();

    pair_vaults(&root_pc, &root_phone).await;

    let mut v_pc = Vault::open(&root_pc).unwrap();
    let mut v_phone = Vault::open(&root_phone).unwrap();
    v_pc.write("note.md", "base").unwrap();
    v_phone.write("note.md", "base").unwrap();
    sync_once(&root_pc, &root_phone).await.unwrap();

    v_pc = Vault::open(&root_pc).unwrap();
    v_phone = Vault::open(&root_phone).unwrap();
    v_pc.write("note.md", "pc edit").unwrap();
    v_phone.write("note.md", "phone edit").unwrap();

    sync_once(&root_pc, &root_phone).await.unwrap();

    v_phone = Vault::open(&root_phone).unwrap();
    assert_eq!(v_phone.read("note.md").unwrap(), "phone edit");
    let notes = v_phone.list().unwrap();
    let conflict = notes
        .iter()
        .find(|n| n.rel_path.contains(".conflict-"))
        .expect("conflict file");
    let conflict_body = v_phone.read(&conflict.rel_path).unwrap();
    assert_eq!(conflict_body, "pc edit");
}

#[tokio::test]
async fn sync_abort_before_commit() {
    let (_d_pc, v_pc0) = open_vault();
    let (_d_phone, v_phone0) = open_vault();
    let root_pc = v_pc0.root().to_path_buf();
    let root_phone = v_phone0.root().to_path_buf();

    pair_vaults(&root_pc, &root_phone).await;

    let mut v_phone = Vault::open(&root_phone).unwrap();
    v_phone.write("new.md", "data").unwrap();

    let cancel = Arc::new(AtomicBool::new(true));
    let (mut t_pc, mut t_phone) = Loopback::pair();
    let root_pc2 = root_pc.clone();
    let cancel_pc = Arc::clone(&cancel);

    let pc = tokio::spawn(async move {
        let mut vault = Vault::open(&root_pc2).unwrap();
        let unsaved = HashSet::new();
        let opts = SyncOptions {
            role: Role::Responder,
            unsaved: &unsaved,
            cancel: Some(cancel_pc.as_ref()),
        };
        run_sync(&mut t_pc, &mut vault, opts).await
    });

    let mut vault = Vault::open(&root_phone).unwrap();
    let unsaved = HashSet::new();
    let opts = SyncOptions {
        role: Role::Initiator,
        unsaved: &unsaved,
        cancel: Some(cancel.as_ref()),
    };
    let phone_err = run_sync(&mut t_phone, &mut vault, opts).await;
    let _ = pc.await;

    assert!(matches!(phone_err, Err(SyncError::Cancelled)), "{phone_err:?}");

    let v_pc = Vault::open(&root_pc).unwrap();
    assert!(v_pc.read("new.md").is_err());

    cancel.store(false, Ordering::Relaxed);
    sync_once(&root_pc, &root_phone).await.unwrap();
    let v_pc = Vault::open(&root_pc).unwrap();
    assert_eq!(v_pc.read("new.md").unwrap(), "data");
}

#[tokio::test]
async fn sync_untrusted() {
    let (_d_pc, v_pc0) = open_vault();
    let (_d_phone, v_phone0) = open_vault();
    let root_pc = v_pc0.root().to_path_buf();
    let root_phone = v_phone0.root().to_path_buf();

    pair_vaults(&root_pc, &root_phone).await;

    let v_phone = Vault::open(&root_phone).unwrap();
    let phone_id = v_phone.device().device_id.clone();
    let mut trust = TrustStore::load(&root_pc).unwrap();
    trust.revoke(&phone_id).unwrap();

    let mut v_phone = Vault::open(&root_phone).unwrap();
    v_phone.write("secret.md", "nope").unwrap();

    let err = sync_once(&root_pc, &root_phone).await;
    assert!(
        matches!(
            err,
            Err(SyncError::Untrusted)
                | Err(SyncError::Crypto(_))
                | Err(SyncError::Transport(_))
                | Err(SyncError::Protocol(_))
        ),
        "{err:?}"
    );

    let v_pc = Vault::open(&root_pc).unwrap();
    assert!(v_pc.read("secret.md").is_err());
}

async fn sync_once(
    root_pc: &std::path::Path,
    root_phone: &std::path::Path,
) -> Result<(), SyncError> {
    let (mut t_pc, mut t_phone) = Loopback::pair();
    let root_pc = root_pc.to_path_buf();
    let root_phone = root_phone.to_path_buf();

    let pc = tokio::spawn(async move {
        let mut vault = Vault::open(&root_pc).unwrap();
        let unsaved = HashSet::new();
        let opts = SyncOptions::responder(&unsaved);
        run_sync(&mut t_pc, &mut vault, opts).await
    });

    let mut vault = Vault::open(&root_phone).unwrap();
    let unsaved = HashSet::new();
    let opts = SyncOptions::initiator(&unsaved);
    let phone_res = run_sync(&mut t_phone, &mut vault, opts).await;
    let pc_res = pc.await.unwrap();

    phone_res?;
    pc_res?;
    Ok(())
}
