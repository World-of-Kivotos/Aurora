mod common;

use std::collections::BTreeSet;

use aurora_modpack::diff::{DownloadReason, KeepReason, diff};
use aurora_modpack::model::FilePolicy;
use aurora_modpack::snapshot::AppliedSnapshot;

use common::{SHA_A, SHA_B, SHA_C, disk, manifest, previous, remote, snapshot};

struct DeterministicRng(u64);

impl DeterministicRng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    fn choose(&mut self, upper: u64) -> u64 {
        self.next() % upper
    }
}

fn policy(rng: &mut DeterministicRng) -> FilePolicy {
    match rng.choose(3) {
        0 => FilePolicy::Managed,
        1 => FilePolicy::Seeded,
        _ => FilePolicy::Optional,
    }
}

fn digest(rng: &mut DeterministicRng) -> &'static str {
    match rng.choose(3) {
        0 => SHA_A,
        1 => SHA_B,
        _ => SHA_C,
    }
}

#[test]
fn randomized_diff_preserves_deletion_and_ownership_invariants() {
    let mut rng = DeterministicRng(0x5eed_cafe_d15c_a11e);

    for iteration in 0..200 {
        let mut remote_files = Vec::new();
        let mut old_files = Vec::new();
        let mut disk_files = Vec::new();

        for index in 0..40 {
            let path = format!("mods/generated-{iteration}-{index}.jar");
            let has_remote = rng.choose(2) == 1;
            let has_previous = rng.choose(2) == 1;
            let remote_digest = digest(&mut rng);
            let previous_digest = digest(&mut rng);

            if has_remote {
                remote_files.push(remote(&path, policy(&mut rng), remote_digest));
            }
            if has_previous {
                old_files.push(previous(&path, policy(&mut rng), previous_digest));
            }
            if rng.choose(4) != 0 {
                let disk_digest = match rng.choose(3) {
                    0 if has_remote => remote_digest,
                    1 if has_previous => previous_digest,
                    _ => digest(&mut rng),
                };
                disk_files.push(disk(&path, disk_digest));
            }
        }

        let private_path = format!("mods/player-private-{iteration}.jar");
        disk_files.push(disk(&private_path, SHA_C));
        let remote_manifest = manifest(remote_files);
        let old_snapshot = snapshot(old_files);
        let plan = diff(&remote_manifest, Some(&old_snapshot), &disk_files).unwrap();

        assert_eq!(
            plan.next_snapshot,
            AppliedSnapshot::from_manifest(&remote_manifest)
        );

        let mut planned_paths = BTreeSet::new();
        for action in &plan.to_delete {
            let path = action.path().as_str();
            assert!(
                planned_paths.insert(path.to_owned()),
                "路径被重复规划: {path}"
            );
            let old = old_snapshot
                .files
                .iter()
                .find(|entry| entry.path.as_str() == path)
                .unwrap();
            assert_eq!(old.policy, FilePolicy::Managed);
            assert!(
                remote_manifest
                    .files
                    .iter()
                    .all(|entry| entry.path.as_str() != path)
            );
            assert!(disk_files.iter().any(|entry| entry.path.as_str() == path));
        }

        for action in &plan.to_download {
            let path = action.file.path.as_str();
            assert!(
                planned_paths.insert(path.to_owned()),
                "路径被重复规划: {path}"
            );
            let observed = disk_files.iter().find(|entry| entry.path.as_str() == path);
            match observed {
                None => {
                    let existed = old_snapshot
                        .files
                        .iter()
                        .any(|entry| entry.path.as_str() == path);
                    assert_eq!(
                        action.reason,
                        if existed {
                            DownloadReason::MissingFile
                        } else {
                            DownloadReason::NewFile
                        }
                    );
                }
                Some(observed) => {
                    assert_eq!(action.file.policy, FilePolicy::Managed);
                    assert_ne!(observed.sha1, action.file.sha1);
                    assert_eq!(action.reason, DownloadReason::ManagedHashMismatch);
                }
            }
        }

        for action in &plan.to_keep {
            let path = action.path.as_str();
            assert!(
                planned_paths.insert(path.to_owned()),
                "路径被重复规划: {path}"
            );
            let remote = remote_manifest
                .files
                .iter()
                .find(|entry| entry.path.as_str() == path);
            let observed = disk_files
                .iter()
                .find(|entry| entry.path.as_str() == path)
                .unwrap();
            match remote {
                Some(remote) if remote.sha1 == observed.sha1 => {
                    let existed = old_snapshot
                        .files
                        .iter()
                        .any(|entry| entry.path.as_str() == path);
                    assert_eq!(
                        action.reason,
                        if existed {
                            KeepReason::AlreadyCurrent
                        } else {
                            KeepReason::Adopted
                        }
                    );
                }
                Some(remote) => {
                    assert_ne!(remote.policy, FilePolicy::Managed);
                    assert_eq!(action.reason, KeepReason::PreserveUserModified);
                }
                None => {
                    let old = old_snapshot
                        .files
                        .iter()
                        .find(|entry| entry.path.as_str() == path)
                        .unwrap();
                    assert_ne!(old.policy, FilePolicy::Managed);
                    assert_eq!(action.reason, KeepReason::RetiredUserOwned);
                }
            }
        }

        for action in &plan.to_forget {
            let path = action.path.as_str();
            assert!(
                planned_paths.insert(path.to_owned()),
                "路径被重复规划: {path}"
            );
            assert!(
                remote_manifest
                    .files
                    .iter()
                    .all(|entry| entry.path.as_str() != path)
            );
            assert!(
                old_snapshot
                    .files
                    .iter()
                    .any(|entry| entry.path.as_str() == path)
            );
            assert!(disk_files.iter().all(|entry| entry.path.as_str() != path));
        }

        assert!(
            !planned_paths.contains(&private_path),
            "纯磁盘玩家文件不得进入任何操作列表"
        );
    }
}
