use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing to transfer; update base to this hash after commit.
    Noop { rel_path: String, hash: Option<String> },
    /// Fetch remote content (or delete local if remote is None).
    Pull { rel_path: String, remote_hash: Option<String> },
    /// Send local content (or delete remote if local is None).
    Push { rel_path: String, local_hash: Option<String> },
    /// Both changed differently — keep local, save remote as conflict file.
    Conflict { rel_path: String },
}

/// Build sync actions from union of local/remote manifests and baseline.
///
/// `unsaved` paths are treated as locally modified even if hash matches disk
/// (caller should pass current editor buffer hash as local, or force Conflict/Push).
/// Here we only force: if unsaved and remote differs from local → Conflict / don't Pull.
pub fn build_plan(
    local: &HashMap<String, String>,
    remote: &HashMap<String, String>,
    base: &HashMap<String, String>,
    unsaved: &HashSet<String>,
) -> Vec<Action> {
    let mut paths: HashSet<&str> = HashSet::new();
    paths.extend(local.keys().map(|s| s.as_str()));
    paths.extend(remote.keys().map(|s| s.as_str()));
    paths.extend(base.keys().map(|s| s.as_str()));

    let mut paths: Vec<&str> = paths.into_iter().collect();
    paths.sort_unstable();

    let mut actions = Vec::with_capacity(paths.len());
    for path in paths {
        let l = local.get(path).cloned();
        let mut r = remote.get(path).cloned();
        let b = base.get(path).cloned();

        // Unsaved local edits: never pull-overwrite; treat as local change.
        if unsaved.contains(path) {
            if l == r {
                actions.push(Action::Noop {
                    rel_path: path.to_string(),
                    hash: l,
                });
            } else if r.is_some() && l != r {
                // Deletion-vs-edit: if remote deleted (r=None) but local unsaved, keep local → Push
                // If both differ → Conflict
                if r.is_none() {
                    actions.push(Action::Push {
                        rel_path: path.to_string(),
                        local_hash: l,
                    });
                } else {
                    actions.push(Action::Conflict {
                        rel_path: path.to_string(),
                    });
                }
            } else if r.is_none() {
                actions.push(Action::Push {
                    rel_path: path.to_string(),
                    local_hash: l,
                });
            }
            continue;
        }

        let action = decide(path, l, r.take(), b);
        actions.push(action);
    }
    actions
}

fn decide(
    path: &str,
    l: Option<String>,
    r: Option<String>,
    b: Option<String>,
) -> Action {
    // L == R → noop, B := L
    if l == r {
        return Action::Noop {
            rel_path: path.to_string(),
            hash: l,
        };
    }

    // Deletion loses to modification:
    // if one side deleted (None) and the other changed from base → keep the modified side.
    match (&l, &r, &b) {
        // L == B, R != B → take remote (R=None → delete local)
        (l, r, b) if l == b && r != b => Action::Pull {
            rel_path: path.to_string(),
            remote_hash: r.clone(),
        },
        // R == B, L != B → send local (L=None → delete remote)
        (l, r, b) if r == b && l != b => Action::Push {
            rel_path: path.to_string(),
            local_hash: l.clone(),
        },
        // L != B, R != B, L != R → conflict
        // Special case: one side deleted, other modified → deletion loses (restore/keep file)
        (None, Some(rh), Some(bh)) if rh != bh => {
            // local deleted, remote modified → pull remote (deletion loses)
            Action::Pull {
                rel_path: path.to_string(),
                remote_hash: Some(rh.clone()),
            }
        }
        (Some(lh), None, Some(bh)) if lh != bh => {
            // remote deleted, local modified → push local
            Action::Push {
                rel_path: path.to_string(),
                local_hash: Some(lh.clone()),
            }
        }
        (Some(_), Some(_), Some(_)) if l != b && r != b && l != r => Action::Conflict {
            rel_path: path.to_string(),
        },
        // B missing, only one side present → copy missing
        (Some(lh), None, None) => Action::Push {
            rel_path: path.to_string(),
            local_hash: Some(lh.clone()),
        },
        (None, Some(rh), None) => Action::Pull {
            rel_path: path.to_string(),
            remote_hash: Some(rh.clone()),
        },
        // B missing, both present and differ → conflict
        (Some(_), Some(_), None) if l != r => Action::Conflict {
            rel_path: path.to_string(),
        },
        // B missing, both present and equal — already handled by L==R
        _ => {
            // Fallback: if L != R treat as conflict when both exist, else prefer present side
            match (l, r) {
                (Some(_), Some(_)) => Action::Conflict {
                    rel_path: path.to_string(),
                },
                (Some(lh), None) => Action::Push {
                    rel_path: path.to_string(),
                    local_hash: Some(lh),
                },
                (None, Some(rh)) => Action::Pull {
                    rel_path: path.to_string(),
                    remote_hash: Some(rh),
                },
                (None, None) => Action::Noop {
                    rel_path: path.to_string(),
                    hash: None,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn identical_is_noop() {
        let local = map(&[("a.md", "h1")]);
        let remote = map(&[("a.md", "h1")]);
        let base = map(&[("a.md", "h0")]);
        let plan = build_plan(&local, &remote, &base, &HashSet::new());
        assert_eq!(
            plan,
            vec![Action::Noop {
                rel_path: "a.md".into(),
                hash: Some("h1".into())
            }]
        );
    }

    #[test]
    fn remote_changed_pull() {
        let local = map(&[("a.md", "h0")]);
        let remote = map(&[("a.md", "h1")]);
        let base = map(&[("a.md", "h0")]);
        let plan = build_plan(&local, &remote, &base, &HashSet::new());
        assert_eq!(
            plan,
            vec![Action::Pull {
                rel_path: "a.md".into(),
                remote_hash: Some("h1".into())
            }]
        );
    }

    #[test]
    fn local_changed_push() {
        let local = map(&[("a.md", "h1")]);
        let remote = map(&[("a.md", "h0")]);
        let base = map(&[("a.md", "h0")]);
        let plan = build_plan(&local, &remote, &base, &HashSet::new());
        assert_eq!(
            plan,
            vec![Action::Push {
                rel_path: "a.md".into(),
                local_hash: Some("h1".into())
            }]
        );
    }

    #[test]
    fn both_changed_conflict() {
        let local = map(&[("a.md", "h1")]);
        let remote = map(&[("a.md", "h2")]);
        let base = map(&[("a.md", "h0")]);
        let plan = build_plan(&local, &remote, &base, &HashSet::new());
        assert_eq!(
            plan,
            vec![Action::Conflict {
                rel_path: "a.md".into()
            }]
        );
    }

    #[test]
    fn deletion_loses_to_edit() {
        let local = map(&[]);
        let remote = map(&[("a.md", "h1")]);
        let base = map(&[("a.md", "h0")]);
        let plan = build_plan(&local, &remote, &base, &HashSet::new());
        assert_eq!(
            plan,
            vec![Action::Pull {
                rel_path: "a.md".into(),
                remote_hash: Some("h1".into())
            }]
        );
    }

    #[test]
    fn new_file_no_base() {
        let local = map(&[("new.md", "h1")]);
        let remote = map(&[]);
        let base = map(&[]);
        let plan = build_plan(&local, &remote, &base, &HashSet::new());
        assert_eq!(
            plan,
            vec![Action::Push {
                rel_path: "new.md".into(),
                local_hash: Some("h1".into())
            }]
        );
    }

    #[test]
    fn remote_delete_when_unchanged_locally() {
        let local = map(&[("a.md", "h0")]);
        let remote = map(&[]);
        let base = map(&[("a.md", "h0")]);
        let plan = build_plan(&local, &remote, &base, &HashSet::new());
        assert_eq!(
            plan,
            vec![Action::Pull {
                rel_path: "a.md".into(),
                remote_hash: None
            }]
        );
    }
}
