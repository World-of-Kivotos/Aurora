mod common;

use aurora_modpack::diff::{DownloadReason, KeepReason, SyncPlan, diff};
use aurora_modpack::error::Error;
use aurora_modpack::model::FilePolicy;

use common::{SHA_A, SHA_B, disk, manifest, previous, remote, snapshot};

const PATH: &str = "mods/core.jar";

fn assert_counts(plan: &SyncPlan, download: usize, delete: usize, keep: usize, forget: usize) {
    assert_eq!(plan.to_download.len(), download);
    assert_eq!(plan.to_delete.len(), delete);
    assert_eq!(plan.to_keep.len(), keep);
    assert_eq!(plan.to_forget.len(), forget);
}

#[test]
fn row_remote_only_missing_downloads_new_file() {
    let remote_manifest = manifest(vec![remote(PATH, FilePolicy::Managed, SHA_A)]);
    let plan = diff(&remote_manifest, None, &[]).unwrap();

    assert_counts(&plan, 1, 0, 0, 0);
    assert_eq!(plan.to_download[0].reason, DownloadReason::NewFile);
    assert_eq!(plan.to_download[0].file.path.as_str(), PATH);
}

#[test]
fn row_remote_only_matching_disk_is_adopted_without_download() {
    let remote_manifest = manifest(vec![remote(PATH, FilePolicy::Managed, SHA_A)]);
    let plan = diff(&remote_manifest, None, &[disk(PATH, SHA_A)]).unwrap();

    assert_counts(&plan, 0, 0, 1, 0);
    assert_eq!(plan.to_keep[0].reason, KeepReason::Adopted);
    assert_eq!(plan.next_snapshot.files[0].path.as_str(), PATH);
}

#[test]
fn row_remote_only_conflicting_disk_obeys_policy() {
    for policy in [
        FilePolicy::Managed,
        FilePolicy::Seeded,
        FilePolicy::Optional,
    ] {
        let remote_manifest = manifest(vec![remote(PATH, policy, SHA_A)]);
        let plan = diff(&remote_manifest, None, &[disk(PATH, SHA_B)]).unwrap();

        match policy {
            FilePolicy::Managed => {
                assert_counts(&plan, 1, 0, 0, 0);
                assert_eq!(
                    plan.to_download[0].reason,
                    DownloadReason::ManagedHashMismatch
                );
            }
            FilePolicy::Seeded | FilePolicy::Optional => {
                assert_counts(&plan, 0, 0, 1, 0);
                assert_eq!(plan.to_keep[0].reason, KeepReason::PreserveUserModified);
            }
        }
    }
}

#[test]
fn row_all_present_and_disk_matches_remote_is_current() {
    let remote_manifest = manifest(vec![remote(PATH, FilePolicy::Managed, SHA_A)]);
    let old = snapshot(vec![previous(PATH, FilePolicy::Managed, SHA_B)]);
    let plan = diff(&remote_manifest, Some(&old), &[disk(PATH, SHA_A)]).unwrap();

    assert_counts(&plan, 0, 0, 1, 0);
    assert_eq!(plan.to_keep[0].reason, KeepReason::AlreadyCurrent);
    assert!(plan.filesystem_is_current());
}

#[test]
fn row_all_present_and_disk_differs_obeys_remote_policy() {
    for policy in [
        FilePolicy::Managed,
        FilePolicy::Seeded,
        FilePolicy::Optional,
    ] {
        let remote_manifest = manifest(vec![remote(PATH, policy, SHA_A)]);
        let old = snapshot(vec![previous(PATH, FilePolicy::Managed, SHA_B)]);
        let plan = diff(&remote_manifest, Some(&old), &[disk(PATH, SHA_B)]).unwrap();

        match policy {
            FilePolicy::Managed => {
                assert_counts(&plan, 1, 0, 0, 0);
                assert_eq!(
                    plan.to_download[0].reason,
                    DownloadReason::ManagedHashMismatch
                );
            }
            FilePolicy::Seeded | FilePolicy::Optional => {
                assert_counts(&plan, 0, 0, 1, 0);
                assert_eq!(plan.to_keep[0].reason, KeepReason::PreserveUserModified);
            }
        }
    }
}

#[test]
fn row_remote_and_snapshot_present_but_disk_missing_redownloads_every_policy() {
    for policy in [
        FilePolicy::Managed,
        FilePolicy::Seeded,
        FilePolicy::Optional,
    ] {
        let remote_manifest = manifest(vec![remote(PATH, policy, SHA_A)]);
        let old = snapshot(vec![previous(PATH, policy, SHA_B)]);
        let plan = diff(&remote_manifest, Some(&old), &[]).unwrap();

        assert_counts(&plan, 1, 0, 0, 0);
        assert_eq!(plan.to_download[0].reason, DownloadReason::MissingFile);
    }
}

#[test]
fn row_removed_but_present_deletes_only_snapshot_managed_entry() {
    let remote_manifest = manifest(vec![]);

    let managed = snapshot(vec![previous(PATH, FilePolicy::Managed, SHA_A)]);
    let managed_plan = diff(&remote_manifest, Some(&managed), &[disk(PATH, SHA_B)]).unwrap();
    assert_counts(&managed_plan, 0, 1, 0, 0);
    assert_eq!(managed_plan.to_delete[0].path().as_str(), PATH);
    assert_eq!(managed_plan.to_delete[0].previous_sha1().as_str(), SHA_A);

    for policy in [FilePolicy::Seeded, FilePolicy::Optional] {
        let old = snapshot(vec![previous(PATH, policy, SHA_A)]);
        let plan = diff(&remote_manifest, Some(&old), &[disk(PATH, SHA_B)]).unwrap();
        assert_counts(&plan, 0, 0, 1, 0);
        assert_eq!(plan.to_keep[0].reason, KeepReason::RetiredUserOwned);
    }
}

#[test]
fn row_removed_and_already_missing_only_forgets_snapshot_entry() {
    let remote_manifest = manifest(vec![]);
    for policy in [
        FilePolicy::Managed,
        FilePolicy::Seeded,
        FilePolicy::Optional,
    ] {
        let old = snapshot(vec![previous(PATH, policy, SHA_A)]);
        let plan = diff(&remote_manifest, Some(&old), &[]).unwrap();
        assert_counts(&plan, 0, 0, 0, 1);
        assert_eq!(plan.to_forget[0].path.as_str(), PATH);
    }
}

#[test]
fn row_disk_only_is_player_file_and_is_not_planned() {
    let plan = diff(&manifest(vec![]), None, &[disk(PATH, SHA_A)]).unwrap();
    assert_counts(&plan, 0, 0, 0, 0);
    assert!(plan.filesystem_is_current());
}

#[test]
fn first_install_is_exactly_empty_snapshot_diff() {
    let remote_manifest = manifest(vec![
        remote("mods/a.jar", FilePolicy::Managed, SHA_A),
        remote("options.txt", FilePolicy::Seeded, SHA_B),
    ]);
    let plan = diff(&remote_manifest, None, &[]).unwrap();

    assert_counts(&plan, 2, 0, 0, 0);
    assert_eq!(
        plan.to_download
            .iter()
            .map(|action| action.file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["mods/a.jar", "options.txt"]
    );
    assert_eq!(plan.next_snapshot.files.len(), 2);
}

#[test]
fn different_pack_snapshot_is_rejected_before_deletion_planning() {
    let remote_manifest = manifest(vec![]);
    let mut old = snapshot(vec![previous(PATH, FilePolicy::Managed, SHA_A)]);
    old.pack_id = "other-pack".to_owned();

    assert!(matches!(
        diff(&remote_manifest, Some(&old), &[disk(PATH, SHA_A)]),
        Err(Error::SnapshotPackMismatch { .. })
    ));
}

#[test]
fn case_aliases_in_disk_inventory_are_rejected() {
    let remote_manifest = manifest(vec![remote(PATH, FilePolicy::Managed, SHA_A)]);
    let disk_files = [disk("mods/Core.jar", SHA_A), disk("MODS/core.JAR", SHA_A)];

    assert!(matches!(
        diff(&remote_manifest, None, &disk_files),
        Err(Error::DuplicatePath { .. })
    ));
}
