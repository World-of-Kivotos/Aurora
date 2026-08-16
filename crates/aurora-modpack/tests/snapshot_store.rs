use aurora_modpack::error::Error;
use aurora_modpack::model::{FilePolicy, PackManifest, SCHEMA_VERSION, Sha1Digest};
use aurora_modpack::path::SafeRelativePath;
use aurora_modpack::snapshot::{
    APPLIED_SNAPSHOT_FILE, AppliedSnapshot, SnapshotEntry, SnapshotStore,
};

const SHA1: &str = "0123456789abcdef0123456789abcdef01234567";

fn manifest(version: &str, path: &str) -> PackManifest {
    PackManifest::from_json_str(&format!(
        r#"{{
            "schema":1,"pack_id":"wok","version":"{version}","minecraft":"1.20.1",
            "loader":{{"kind":"forge","version":"47.4.20"}},
            "files":[{{
                "path":"{path}","sha1":"{SHA1}","size":42,"policy":"managed",
                "urls":["https://cdn.example.test/{version}/file.jar"]
            }}]
        }}"#
    ))
    .unwrap()
}

#[tokio::test]
async fn missing_snapshot_is_first_install_and_path_is_version_local() {
    let temp = tempfile::tempdir().unwrap();
    let store = SnapshotStore::for_version_dir(temp.path());
    assert_eq!(
        store.path(),
        temp.path().join(".aurora").join(APPLIED_SNAPSHOT_FILE)
    );
    assert_eq!(store.load().await.unwrap(), None);
}

#[tokio::test]
async fn round_trip_is_atomic_and_only_persists_diff_fields() {
    let temp = tempfile::tempdir().unwrap();
    let store = SnapshotStore::for_version_dir(temp.path());
    let snapshot = AppliedSnapshot::from_manifest(&manifest("2.0.0", "mods/core.jar"));

    store.save(&snapshot).await.unwrap();
    assert_eq!(store.load().await.unwrap(), Some(snapshot.clone()));

    let text = tokio::fs::read_to_string(store.path()).await.unwrap();
    assert!(text.contains("\"pack_id\": \"wok\""));
    assert!(!text.contains("minecraft"));
    assert!(!text.contains("urls"));
    assert!(!text.contains("size"));

    let mut entries = tokio::fs::read_dir(store.path().parent().unwrap())
        .await
        .unwrap();
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    assert_eq!(names, vec![APPLIED_SNAPSHOT_FILE.to_owned()]);
}

#[tokio::test]
async fn save_replaces_previous_snapshot_without_residue() {
    let temp = tempfile::tempdir().unwrap();
    let store = SnapshotStore::for_version_dir(temp.path());
    let first = AppliedSnapshot::from_manifest(&manifest("1.0.0", "mods/old.jar"));
    let second = AppliedSnapshot::from_manifest(&manifest("2.0.0", "mods/new.jar"));

    store.save(&first).await.unwrap();
    store.save(&second).await.unwrap();

    let loaded = store.load().await.unwrap().unwrap();
    assert_eq!(loaded.version, "2.0.0");
    assert_eq!(loaded.files[0].path.as_str(), "mods/new.jar");
    let count = std::fs::read_dir(store.path().parent().unwrap())
        .unwrap()
        .count();
    assert_eq!(count, 1, "原子替换不应遗留临时文件");
}

#[tokio::test]
async fn corrupt_or_future_snapshot_never_silently_resets() {
    let temp = tempfile::tempdir().unwrap();
    let store = SnapshotStore::at(temp.path().join("snapshot.json"));

    tokio::fs::write(store.path(), b"{\"files\":[")
        .await
        .unwrap();
    assert!(matches!(store.load().await, Err(Error::Json { .. })));

    let future = r#"{"schema":2,"pack_id":"wok","version":"2.0.0","files":[]}"#;
    tokio::fs::write(store.path(), future).await.unwrap();
    assert!(matches!(
        store.load().await,
        Err(Error::UnsupportedSchema {
            expected: SCHEMA_VERSION,
            actual: 2,
            ..
        })
    ));
}

#[test]
fn snapshot_parser_rejects_unknown_fields() {
    let json = format!(
        r#"{{
            "schema":1,"pack_id":"wok","version":"2.0.0","files":[{{
                "path":"mods/core.jar","sha1":"{SHA1}","policy":"managed","url":"https://bad"
            }}]
        }}"#
    );
    assert!(matches!(
        AppliedSnapshot::from_json_str(&json),
        Err(Error::Json { .. })
    ));
}

#[tokio::test]
async fn invalid_manual_snapshot_is_rejected_before_any_write() {
    let temp = tempfile::tempdir().unwrap();
    let store = SnapshotStore::at(temp.path().join("snapshot.json"));
    let entry = SnapshotEntry {
        path: SafeRelativePath::new("mods/Core.jar").unwrap(),
        sha1: Sha1Digest::new(SHA1).unwrap(),
        policy: FilePolicy::Managed,
    };
    let snapshot = AppliedSnapshot {
        schema: SCHEMA_VERSION,
        pack_id: "wok".to_owned(),
        version: "2.0.0".to_owned(),
        files: vec![
            entry,
            SnapshotEntry {
                path: SafeRelativePath::new("MODS/core.JAR").unwrap(),
                sha1: Sha1Digest::new(SHA1).unwrap(),
                policy: FilePolicy::Seeded,
            },
        ],
    };

    assert!(matches!(
        store.save(&snapshot).await,
        Err(Error::DuplicatePath { .. })
    ));
    assert!(!store.path().exists());
}
