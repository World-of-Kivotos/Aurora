use std::path::{Path, PathBuf};

use aurora_core::{
    Aurora, AuroraConfig, ManagedModpackStatus, ModpackCacheSource, ModpackSubscription,
    ModpackSyncFailure, ModpackSyncStage,
};
use aurora_instance::IsolationPolicy;
use aurora_modpack::{
    AppliedSnapshot, FilePolicy, PackManifest, SnapshotStore, SnapshotWorkingDirectory,
};
use sha1::{Digest, Sha1};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const INSTANCE_ID: &str = "forge-test";

fn sha1_hex(bytes: &[u8]) -> String {
    Sha1::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) {
    match std::os::windows::fs::symlink_dir(target, link) {
        Ok(()) => {}
        Err(source) if source.raw_os_error() == Some(1314) => {
            let status = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .status()
                .expect("调用 mklink 创建测试 junction");
            assert!(status.success(), "创建测试 junction 失败: {status}");
        }
        Err(source) => panic!("创建目录符号链接失败: {source}"),
    }
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("测试需要创建目录符号链接");
}

fn aurora_at(mc: &Path) -> Aurora {
    let mut config = AuroraConfig {
        game_directory: Some(mc.to_path_buf()),
        isolation_policy: IsolationPolicy::All,
        ..AuroraConfig::default()
    };
    config.download_concurrency = 2;
    Aurora::open(config, mc.to_path_buf(), mc.join("config.json")).unwrap()
}

async fn put_instance(mc: &Path) -> PathBuf {
    let version_dir = mc.join("versions").join(INSTANCE_ID);
    tokio::fs::create_dir_all(&version_dir).await.unwrap();
    tokio::fs::write(
        version_dir.join(format!("{INSTANCE_ID}.json")),
        format!(
            r#"{{"id":"{INSTANCE_ID}","type":"release","mainClass":"test.Main","inheritsFrom":"1.20.1","libraries":[{{"name":"net.minecraftforge:forge:1.20.1-47.4.20"}}]}}"#
        ),
    )
    .await
    .unwrap();
    version_dir
}

async fn subscribe(aurora: &Aurora, pointer_url: String) {
    aurora
        .set_modpack_subscription(
            INSTANCE_ID,
            &ModpackSubscription {
                pack_id: "wok".to_owned(),
                pointer_url,
            },
        )
        .await
        .unwrap();
}

fn write_subscription(version_dir: &Path, pointer_url: String) {
    let metadata_dir = version_dir.join(".aurora");
    std::fs::create_dir_all(&metadata_dir).unwrap();
    let subscription = ModpackSubscription {
        pack_id: "wok".to_owned(),
        pointer_url,
    };
    std::fs::write(
        metadata_dir.join("modpack-subscription.json"),
        serde_json::to_vec_pretty(&subscription).unwrap(),
    )
    .unwrap();
}

fn pointer_json(base: &str, version: &str, min_launcher: &str) -> String {
    format!(
        r#"{{
            "pack_id":"wok",
            "version":"{version}",
            "manifest_url":"{base}/manifest.json",
            "released_at":"2026-08-17T12:00:00Z",
            "note":"test release",
            "min_launcher_version":"{min_launcher}"
        }}"#
    )
}

fn manifest_json(_base: &str, version: &str, files: &str) -> String {
    manifest_json_with_runtime(version, "1.20.1", "forge", "47.4.20", files)
}

fn manifest_json_with_runtime(
    version: &str,
    minecraft: &str,
    loader_kind: &str,
    loader_version: &str,
    files: &str,
) -> String {
    format!(
        r#"{{
            "schema":1,
            "pack_id":"wok",
            "version":"{version}",
            "minecraft":"{minecraft}",
            "loader":{{"kind":"{loader_kind}","version":"{loader_version}"}},
            "files":[{files}]
        }}"#
    )
}

async fn save_previous_snapshot(version_dir: &Path, file_name: &str, bytes: &[u8]) {
    tokio::fs::write(version_dir.join(file_name), bytes)
        .await
        .unwrap();
    let old_manifest = PackManifest::from_json_str(&manifest_json(
        "https://unused.example",
        "1.0.0",
        &format!(
            r#"{{
                "path":"{file_name}",
                "sha1":"{}",
                "size":{},
                "policy":"managed",
                "urls":["https://unused.example/{file_name}"]
            }}"#,
            sha1_hex(bytes),
            bytes.len()
        ),
    ))
    .unwrap();
    SnapshotStore::for_version_dir(version_dir)
        .save(&AppliedSnapshot::from_manifest(
            &old_manifest,
            SnapshotWorkingDirectory::IsolatedVersionDirectory,
        ))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pointer_cache_is_used_for_server_failures_and_transport_errors() {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    put_instance(tmp.path()).await;
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, format!("{base}/latest")).await;

    let first = aurora
        .managed_modpack_status(INSTANCE_ID)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        first,
        ManagedModpackStatus::Ready {
            source: ModpackCacheSource::Network,
            ref versions,
            ..
        } if versions.latest.version == "2.0.0" && versions.installed_version.is_none()
    ));

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let cached_after_server_failure = aurora
        .managed_modpack_status(INSTANCE_ID)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        cached_after_server_failure,
        ManagedModpackStatus::Ready {
            source: ModpackCacheSource::Cache,
            ref versions,
            ..
        } if versions.latest.version == "2.0.0"
    ));

    drop(server);
    let cached_after_transport_error = aurora
        .managed_modpack_status(INSTANCE_ID)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        cached_after_transport_error,
        ManagedModpackStatus::Ready {
            source: ModpackCacheSource::Cache,
            ref versions,
            ..
        } if versions.latest.version == "2.0.0"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pointer_client_error_never_falls_back_to_a_valid_cache() {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    put_instance(tmp.path()).await;
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, format!("{base}/latest")).await;
    let primed = aurora
        .managed_modpack_status(INSTANCE_ID)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        primed,
        ManagedModpackStatus::Ready {
            source: ModpackCacheSource::Network,
            ..
        }
    ));

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let rejected = aurora
        .managed_modpack_status(INSTANCE_ID)
        .await
        .unwrap()
        .unwrap();
    match rejected {
        ManagedModpackStatus::Unavailable {
            last_known, detail, ..
        } => {
            assert_eq!(last_known.unwrap().latest.version, "2.0.0");
            assert!(detail.contains("HTTP 404"), "{detail}");
        }
        other => panic!("HTTP 404 must fail closed instead of returning cached Ready: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn incompatible_network_pointer_is_explicit_and_does_not_hide_behind_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_string(pointer_json(
            &server.uri(),
            "2.0.0",
            "0.1.0",
        )))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    put_instance(tmp.path()).await;
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, format!("{}/latest", server.uri())).await;
    let primed = aurora.managed_modpack_status(INSTANCE_ID).await.unwrap();
    assert!(matches!(primed, Some(ManagedModpackStatus::Ready { .. })));

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_string(pointer_json(
            &server.uri(),
            "3.0.0",
            "99.0.0",
        )))
        .mount(&server)
        .await;

    let rejected = aurora
        .managed_modpack_status(INSTANCE_ID)
        .await
        .unwrap()
        .unwrap();
    match rejected {
        ManagedModpackStatus::Unavailable {
            last_known, detail, ..
        } => {
            assert_eq!(last_known.unwrap().latest.version, "2.0.0");
            assert!(detail.contains("要求 Aurora >= 99.0.0"), "{detail}");
            assert!(detail.contains("当前版本为 0.1.0"), "{detail}");
        }
        other => panic!("应明确拒绝过新的启动器要求，实际为 {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn minimum_version_uses_the_explicit_launcher_product_version() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_string(pointer_json(
            &server.uri(),
            "2.0.0",
            "5.0.0",
        )))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    put_instance(tmp.path()).await;
    let aurora = aurora_at(tmp.path())
        .with_launcher_version("5.1.0")
        .unwrap();
    subscribe(&aurora, format!("{}/latest", server.uri())).await;

    let status = aurora
        .managed_modpack_status(INSTANCE_ID)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        status,
        ManagedModpackStatus::Ready {
            source: ModpackCacheSource::Network,
            ref versions,
            ..
        } if versions.latest.min_launcher_version == "5.0.0"
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manifest_cache_rejects_client_errors_but_allows_server_and_transport_failures() {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json(&base, "2.0.0", "")))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    put_instance(tmp.path()).await;
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, format!("{base}/latest")).await;

    let primed = aurora
        .sync_managed_modpack(INSTANCE_ID, "2.0.0", None)
        .await
        .unwrap();
    assert_eq!(primed.installed_version, "2.0.0");

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let rejected = aurora
        .sync_managed_modpack(INSTANCE_ID, "2.0.0", None)
        .await
        .unwrap_err();
    assert_eq!(rejected.stage, ModpackSyncStage::ResolvingManifest);
    assert!(matches!(
        rejected.failure,
        ModpackSyncFailure::Conflict { ref detail } if detail.contains("HTTP 404")
    ));

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let cached_after_server_failure = aurora
        .sync_managed_modpack(INSTANCE_ID, "2.0.0", None)
        .await
        .unwrap();
    assert_eq!(cached_after_server_failure.installed_version, "2.0.0");

    drop(server);
    let cached_after_transport_error = aurora
        .sync_managed_modpack(INSTANCE_ID, "2.0.0", None)
        .await
        .unwrap();
    assert_eq!(cached_after_transport_error.installed_version, "2.0.0");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_download_never_deletes_previous_managed_file_or_advances_snapshot() {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json(
            &base,
            "2.0.0",
            &format!(
                r#"{{
                    "path":"mods/new.jar",
                    "sha1":"{}",
                    "size":8,
                    "policy":"managed",
                    "urls":["{base}/new.jar"]
                }}"#,
                sha1_hex(b"new-file")
            ),
        )))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let version_dir = put_instance(tmp.path()).await;
    tokio::fs::create_dir_all(version_dir.join("mods"))
        .await
        .unwrap();
    save_previous_snapshot(&version_dir, "mods/old.jar", b"old-file").await;
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, format!("{base}/latest")).await;

    let error = aurora
        .sync_managed_modpack(INSTANCE_ID, "2.0.0", None)
        .await
        .unwrap_err();
    assert_eq!(error.stage, ModpackSyncStage::DownloadingFiles);
    assert!(matches!(
        error.failure,
        ModpackSyncFailure::Network { ref file_path, .. } if file_path == "mods/new.jar"
    ));
    assert_eq!(
        tokio::fs::read(version_dir.join("mods/old.jar"))
            .await
            .unwrap(),
        b"old-file"
    );
    let snapshot = SnapshotStore::for_version_dir(&version_dir)
        .load()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.version, "1.0.0");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn successful_sync_downloads_then_deletes_and_atomically_advances_snapshot() {
    let server = MockServer::start().await;
    let base = server.uri();
    let new_bytes = b"verified-new-file";
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json(
            &base,
            "2.0.0",
            &format!(
                r#"{{
                    "path":"mods/new.jar",
                    "sha1":"{}",
                    "size":{},
                    "policy":"managed",
                    "urls":["{base}/new.jar"]
                }}"#,
                sha1_hex(new_bytes),
                new_bytes.len()
            ),
        )))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/new.jar"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(new_bytes.to_vec()))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let version_dir = put_instance(tmp.path()).await;
    tokio::fs::create_dir_all(version_dir.join("mods"))
        .await
        .unwrap();
    save_previous_snapshot(&version_dir, "mods/old.jar", b"old-file").await;
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, format!("{base}/latest")).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let outcome = aurora
        .sync_managed_modpack(INSTANCE_ID, "2.0.0", Some(&tx))
        .await
        .unwrap();
    drop(tx);
    let mut stages = Vec::new();
    while let Some(event) = rx.recv().await {
        if let aurora_core::CoreEvent::ModpackSync(progress) = event
            && stages.last() != Some(&progress.stage)
        {
            stages.push(progress.stage);
        }
    }

    assert_eq!(outcome.installed_version, "2.0.0");
    assert_eq!(outcome.downloaded_files, 1);
    assert_eq!(outcome.deleted_files, 1);
    assert_eq!(
        tokio::fs::read(version_dir.join("mods/new.jar"))
            .await
            .unwrap(),
        new_bytes
    );
    assert!(!version_dir.join("mods/old.jar").exists());
    let snapshot = SnapshotStore::for_version_dir(&version_dir)
        .load()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.version, "2.0.0");
    assert_eq!(snapshot.files.len(), 1);
    assert_eq!(
        stages,
        vec![
            ModpackSyncStage::ResolvingManifest,
            ModpackSyncStage::DownloadingFiles,
            ModpackSyncStage::DeletingFiles,
            ModpackSyncStage::WritingSnapshot,
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_target_with_symlink_ancestor_is_rejected_before_external_write() {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json(
            &base,
            "2.0.0",
            &format!(
                r#"{{
                    "path":"mods/escape.jar",
                    "sha1":"{}",
                    "size":6,
                    "policy":"managed",
                    "urls":["{base}/escape.jar"]
                }}"#,
                sha1_hex(b"escape")
            ),
        )))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/escape.jar"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"escape".to_vec()))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let version_dir = put_instance(tmp.path()).await;
    let outside = tmp.path().join("outside-download");
    std::fs::create_dir_all(&outside).unwrap();
    symlink_dir(&outside, &version_dir.join("mods"));
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, format!("{base}/latest")).await;

    let error = aurora
        .sync_managed_modpack(INSTANCE_ID, "2.0.0", None)
        .await
        .unwrap_err();
    assert!(matches!(
        error.failure,
        ModpackSyncFailure::Filesystem { ref file_path, ref detail }
            if file_path == "mods/escape.jar" && detail.contains("符号链接或 reparse point")
    ));
    assert!(!outside.join("escape.jar").exists());
    let requests = server.received_requests().await.unwrap();
    assert!(
        !requests
            .iter()
            .any(|request| request.url.path() == "/escape.jar")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deletion_target_with_symlink_ancestor_never_touches_external_file() {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json(&base, "2.0.0", "")))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let version_dir = put_instance(tmp.path()).await;
    let old_manifest = PackManifest::from_json_str(&manifest_json(
        "https://unused.example",
        "1.0.0",
        &format!(
            r#"{{
                "path":"mods/old.jar",
                "sha1":"{}",
                "size":8,
                "policy":"managed",
                "urls":["https://unused.example/old.jar"]
            }}"#,
            sha1_hex(b"old-file")
        ),
    ))
    .unwrap();
    SnapshotStore::for_version_dir(&version_dir)
        .save(&AppliedSnapshot::from_manifest(
            &old_manifest,
            SnapshotWorkingDirectory::IsolatedVersionDirectory,
        ))
        .await
        .unwrap();
    let outside = tmp.path().join("outside-delete");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("old.jar"), b"old-file").unwrap();
    symlink_dir(&outside, &version_dir.join("mods"));
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, format!("{base}/latest")).await;

    let error = aurora
        .sync_managed_modpack(INSTANCE_ID, "2.0.0", None)
        .await
        .unwrap_err();
    assert!(matches!(
        error.failure,
        ModpackSyncFailure::Filesystem { ref file_path, ref detail }
            if file_path == "mods/old.jar" && detail.contains("符号链接或 reparse point")
    ));
    assert_eq!(std::fs::read(outside.join("old.jar")).unwrap(), b"old-file");
    let snapshot = SnapshotStore::for_version_dir(&version_dir)
        .load()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.version, "1.0.0");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manifest_cannot_claim_launcher_or_player_reserved_namespaces() {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json(
            &base,
            "2.0.0",
            &format!(
                r#"{{
                    "path":"saves/world/level.dat",
                    "sha1":"{}",
                    "size":6,
                    "policy":"managed",
                    "urls":["{base}/level.dat"]
                }}"#,
                sha1_hex(b"world!")
            ),
        )))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    put_instance(tmp.path()).await;
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, format!("{base}/latest")).await;

    let error = aurora
        .sync_managed_modpack(INSTANCE_ID, "2.0.0", None)
        .await
        .unwrap_err();
    assert!(matches!(
        error.failure,
        ModpackSyncFailure::InvalidMetadata { ref detail }
            if detail.contains("整合包清单 JSON 解析失败")
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_pack_mismatch_is_rejected_before_any_candidate_path_is_hashed() {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json(&base, "2.0.0", "")))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let version_dir = put_instance(tmp.path()).await;
    let other_pack = PackManifest::from_json_str(
        &manifest_json(
            "https://unused.example",
            "1.0.0",
            &format!(
                r#"{{
                    "path":"mods/outside.jar",
                    "sha1":"{}",
                    "size":7,
                    "policy":"managed",
                    "urls":["https://unused.example/outside.jar"]
                }}"#,
                sha1_hex(b"outside")
            ),
        )
        .replace(r#""pack_id":"wok""#, r#""pack_id":"other""#),
    )
    .unwrap();
    SnapshotStore::for_version_dir(&version_dir)
        .save(&AppliedSnapshot::from_manifest(
            &other_pack,
            SnapshotWorkingDirectory::IsolatedVersionDirectory,
        ))
        .await
        .unwrap();
    let outside = tmp.path().join("outside-pack-mismatch");
    std::fs::create_dir_all(&outside).unwrap();
    symlink_dir(&outside, &version_dir.join("mods"));
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, format!("{base}/latest")).await;

    let error = aurora
        .sync_managed_modpack(INSTANCE_ID, "2.0.0", None)
        .await
        .unwrap_err();
    assert_eq!(error.stage, ModpackSyncStage::ResolvingManifest);
    assert!(matches!(
        error.failure,
        ModpackSyncFailure::Conflict { ref detail }
            if detail.contains("snapshot belongs to pack other")
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pointer_cache_target_reparse_point_is_rejected_before_cache_write() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_string(pointer_json(
            &server.uri(),
            "2.0.0",
            "0.1.0",
        )))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let version_dir = put_instance(tmp.path()).await;
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, format!("{}/latest", server.uri())).await;
    let outside = tmp.path().join("outside-cache");
    std::fs::create_dir_all(&outside).unwrap();
    symlink_dir(
        &outside,
        &version_dir
            .join(".aurora")
            .join("modpack-latest-cache.json"),
    );

    let error = aurora
        .managed_modpack_status(INSTANCE_ID)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        aurora_core::CoreError::UnsafeModpackPath { ref path, .. }
            if path.ends_with("modpack-latest-cache.json")
    ));
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_directory_reparse_point_blocks_cache_write_outside_game_directory() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_string(pointer_json(
            &server.uri(),
            "2.0.0",
            "0.1.0",
        )))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let game_dir = tmp.path().join("game");
    let versions_dir = game_dir.join("versions");
    let outside = tmp.path().join("outside-version-cache");
    std::fs::create_dir_all(&versions_dir).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    write_subscription(&outside, format!("{}/latest", server.uri()));
    symlink_dir(&outside, &versions_dir.join(INSTANCE_ID));
    let aurora = aurora_at(&game_dir);

    let error = aurora
        .managed_modpack_status(INSTANCE_ID)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        aurora_core::CoreError::UnsafeModpackPath { ref reason, .. }
            if reason.contains("符号链接或 reparse point")
    ));
    assert!(
        !outside
            .join(".aurora")
            .join("modpack-latest-cache.json")
            .exists()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_directory_reparse_point_blocks_managed_delete_outside_game_directory() {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(manifest_json(&base, "2.0.0", "")))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let game_dir = tmp.path().join("game");
    let versions_dir = game_dir.join("versions");
    let outside = tmp.path().join("outside-version-delete");
    std::fs::create_dir_all(&versions_dir).unwrap();
    std::fs::create_dir_all(outside.join("mods")).unwrap();
    std::fs::write(outside.join("mods/old.jar"), b"old-file").unwrap();
    write_subscription(&outside, format!("{base}/latest"));
    save_previous_snapshot(&outside, "mods/old.jar", b"old-file").await;
    symlink_dir(&outside, &versions_dir.join(INSTANCE_ID));
    let aurora = aurora_at(&game_dir);

    let error = aurora
        .sync_managed_modpack(INSTANCE_ID, "2.0.0", None)
        .await
        .unwrap_err();
    assert_eq!(error.stage, ModpackSyncStage::ResolvingManifest);
    assert!(matches!(
        error.failure,
        ModpackSyncFailure::Filesystem { ref detail, .. }
            if detail.contains("符号链接或 reparse point")
    ));
    assert_eq!(
        std::fs::read(outside.join("mods/old.jar")).unwrap(),
        b"old-file"
    );
    let snapshot = SnapshotStore::for_version_dir(&outside)
        .load()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.version, "1.0.0");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn traversal_instance_id_is_rejected_before_managed_reads_or_writes() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let game_dir = tmp.path().join("game");
    let outside = game_dir.join("outside");
    std::fs::create_dir_all(game_dir.join("versions")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    write_subscription(&outside, format!("{}/latest", server.uri()));
    let sentinel = outside.join(".aurora").join("modpack-latest-cache.json");
    std::fs::write(&sentinel, b"sentinel").unwrap();
    let aurora = aurora_at(&game_dir);
    let traversal_id = "../outside";

    let status_error = aurora
        .managed_modpack_status(traversal_id)
        .await
        .unwrap_err();
    assert!(matches!(
        status_error,
        aurora_core::CoreError::UnsafeModpackPath { ref path, .. }
            if path == traversal_id
    ));

    let files_error = aurora
        .managed_modpack_files(traversal_id)
        .await
        .unwrap_err();
    assert!(matches!(
        files_error,
        aurora_core::CoreError::UnsafeModpackPath { ref path, .. }
            if path == traversal_id
    ));

    let sync_error = aurora
        .sync_managed_modpack(traversal_id, "2.0.0", None)
        .await
        .unwrap_err();
    assert_eq!(sync_error.stage, ModpackSyncStage::ResolvingManifest);
    assert!(matches!(
        sync_error.failure,
        ModpackSyncFailure::Filesystem { ref file_path, .. }
            if file_path == traversal_id
    ));
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"sentinel");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn managed_file_query_uses_only_the_validated_success_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let version_dir = put_instance(tmp.path()).await;
    let manifest = PackManifest::from_json_str(&manifest_json(
        "https://unused.example",
        "1.0.0",
        &format!(
            r#"{{
                "path":"mods/managed.jar",
                "sha1":"{}",
                "size":7,
                "policy":"managed",
                "urls":["https://unused.example/managed.jar"]
            }},{{
                "path":"config/player.cfg",
                "sha1":"{}",
                "size":6,
                "policy":"seeded",
                "urls":["https://unused.example/player.cfg"]
            }}"#,
            sha1_hex(b"managed"),
            sha1_hex(b"player")
        ),
    ))
    .unwrap();
    SnapshotStore::for_version_dir(&version_dir)
        .save(&AppliedSnapshot::from_manifest(
            &manifest,
            SnapshotWorkingDirectory::IsolatedVersionDirectory,
        ))
        .await
        .unwrap();
    let aurora = aurora_at(tmp.path());
    subscribe(&aurora, "https://unused.example/latest".to_owned()).await;

    let files = aurora
        .managed_modpack_files(INSTANCE_ID)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, "mods/managed.jar");
    assert_eq!(files[0].policy, FilePolicy::Managed);
    assert_eq!(files[1].path, "config/player.cfg");
    assert_eq!(files[1].policy, FilePolicy::Seeded);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_click_install_rejects_the_exact_planned_id_before_installer_writes() {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(pointer_json(&base, "2.0.0", "0.1.0")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifest.json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(manifest_json_with_runtime(
                "2.0.0",
                INSTANCE_ID,
                "vanilla",
                "vanilla",
                "",
            )),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let version_dir = put_instance(tmp.path()).await;
    let version_json = version_dir.join(format!("{INSTANCE_ID}.json"));
    let before = tokio::fs::read(&version_json).await.unwrap();
    let aurora = aurora_at(tmp.path());

    let error = aurora
        .install_managed_modpack(&format!("{base}/latest"), None)
        .await
        .unwrap_err();
    assert_eq!(error.stage, ModpackSyncStage::ResolvingManifest);
    assert!(matches!(
        error.failure,
        ModpackSyncFailure::Conflict { ref detail }
            if detail.contains("planned instance id forge-test has incompatible version metadata")
    ));
    assert_eq!(tokio::fs::read(version_json).await.unwrap(), before);

    let broken_tmp = tempfile::tempdir().unwrap();
    let broken_dir = broken_tmp.path().join("versions").join(INSTANCE_ID);
    tokio::fs::create_dir_all(&broken_dir).await.unwrap();
    let broken_aurora = aurora_at(broken_tmp.path());
    let broken_error = broken_aurora
        .install_managed_modpack(&format!("{base}/latest"), None)
        .await
        .unwrap_err();
    assert_eq!(broken_error.stage, ModpackSyncStage::ResolvingManifest);
    assert!(matches!(
        broken_error.failure,
        ModpackSyncFailure::Conflict { ref detail }
            if detail.contains("planned instance id forge-test is occupied by a broken instance")
    ));
    assert_eq!(std::fs::read_dir(&broken_dir).unwrap().count(), 0);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.url.path())
            .collect::<Vec<_>>(),
        vec!["/latest", "/manifest.json", "/latest", "/manifest.json"]
    );
}
