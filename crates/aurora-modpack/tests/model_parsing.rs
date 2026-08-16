use aurora_modpack::error::Error;
use aurora_modpack::model::{FilePolicy, LoaderKind, PackManifest, PackPointer};

const SHA1: &str = "0123456789abcdef0123456789abcdef01234567";

fn valid_manifest() -> String {
    format!(
        r#"{{
            "schema": 1,
            "pack_id": "wok",
            "version": "2.0.0",
            "minecraft": "1.20.1",
            "loader": {{"kind": "forge", "version": "47.4.20"}},
            "files": [{{
                "path": "mods/wok-core.jar",
                "sha1": "{SHA1}",
                "size": 0,
                "policy": "managed",
                "urls": ["https://cdn.example.test/files/wok-core.jar?token=abc"]
            }}]
        }}"#
    )
}

#[test]
fn parses_pointer_and_manifest_contract() {
    let pointer = PackPointer::from_json_str(
        r#"{
            "pack_id":"wok",
            "version":"2.0.0",
            "manifest_url":"https://api.example.test/api/v1/pack/manifest/2.0.0",
            "released_at":"2026-08-17T12:00:00Z",
            "note":"新周目",
            "min_launcher_version":"0.3.0"
        }"#,
    )
    .unwrap();
    assert_eq!(pointer.pack_id, "wok");
    assert_eq!(pointer.note.as_deref(), Some("新周目"));

    let manifest = PackManifest::from_json_str(&valid_manifest()).unwrap();
    assert_eq!(manifest.schema, 1);
    assert_eq!(manifest.loader.kind, LoaderKind::Forge);
    assert_eq!(manifest.files[0].policy, FilePolicy::Managed);
    assert_eq!(manifest.files[0].size, 0, "空配置文件是合法边界值");
}

#[test]
fn pointer_rejects_unknown_missing_and_invalid_semver_fields() {
    let unknown = r#"{
        "pack_id":"wok","version":"2.0.0",
        "manifest_url":"https://api.example.test/manifest",
        "released_at":"2026-08-17T12:00:00Z",
        "min_launcher_version":"0.3.0","extra":true
    }"#;
    assert!(matches!(
        PackPointer::from_json_str(unknown),
        Err(Error::Json { .. })
    ));

    let missing = r#"{
        "pack_id":"wok","version":"2.0.0",
        "manifest_url":"https://api.example.test/manifest",
        "released_at":"2026-08-17T12:00:00Z"
    }"#;
    assert!(matches!(
        PackPointer::from_json_str(missing),
        Err(Error::Json { .. })
    ));

    let invalid_semver = r#"{
        "pack_id":"wok","version":"2.0.0",
        "manifest_url":"https://api.example.test/manifest",
        "released_at":"2026-08-17T12:00:00Z",
        "min_launcher_version":"latest"
    }"#;
    assert!(matches!(
        PackPointer::from_json_str(invalid_semver),
        Err(Error::InvalidField { ref field, .. }) if field == "min_launcher_version"
    ));
}

#[test]
fn pointer_rejects_non_http_or_ambiguous_manifest_urls() {
    for url in [
        "file:///C:/manifest.json",
        "https://user:secret@example.test/manifest",
        "https://example.test/manifest#fragment",
        "relative/manifest.json",
    ] {
        let json = format!(
            r#"{{
                "pack_id":"wok","version":"2.0.0",
                "manifest_url":"{url}",
                "released_at":"2026-08-17T12:00:00Z",
                "min_launcher_version":"0.3.0"
            }}"#
        );
        assert!(
            matches!(PackPointer::from_json_str(&json), Err(Error::InvalidField { ref field, .. }) if field == "manifest_url"),
            "不安全 URL 未被拒绝: {url}"
        );
    }
}

#[test]
fn manifest_rejects_future_schema_and_unknown_nested_fields() {
    let future = valid_manifest().replacen("\"schema\": 1", "\"schema\": 2", 1);
    assert!(matches!(
        PackManifest::from_json_str(&future),
        Err(Error::UnsupportedSchema {
            expected: 1,
            actual: 2,
            ..
        })
    ));

    let unknown_loader = valid_manifest().replacen(
        "\"version\": \"47.4.20\"",
        "\"version\": \"47.4.20\", \"extra\": true",
        1,
    );
    assert!(matches!(
        PackManifest::from_json_str(&unknown_loader),
        Err(Error::Json { .. })
    ));

    let unknown_file = valid_manifest().replacen(
        "\"policy\": \"managed\"",
        "\"policy\": \"managed\", \"kind\": \"custom\"",
        1,
    );
    assert!(matches!(
        PackManifest::from_json_str(&unknown_file),
        Err(Error::Json { .. })
    ));
}

#[test]
fn manifest_rejects_bad_hash_policy_loader_and_urls() {
    let short_hash = valid_manifest().replacen(SHA1, "abcd", 1);
    assert!(matches!(
        PackManifest::from_json_str(&short_hash),
        Err(Error::Json { .. })
    ));

    let non_hex = valid_manifest().replacen(SHA1, &"z".repeat(40), 1);
    assert!(matches!(
        PackManifest::from_json_str(&non_hex),
        Err(Error::Json { .. })
    ));

    let bad_policy = valid_manifest().replacen("\"managed\"", "\"ignore\"", 1);
    assert!(matches!(
        PackManifest::from_json_str(&bad_policy),
        Err(Error::Json { .. })
    ));

    let bad_loader = valid_manifest().replacen("\"forge\"", "\"unknown\"", 1);
    assert!(matches!(
        PackManifest::from_json_str(&bad_loader),
        Err(Error::Json { .. })
    ));

    let no_urls = valid_manifest().replacen(
        "[\"https://cdn.example.test/files/wok-core.jar?token=abc\"]",
        "[]",
        1,
    );
    assert!(matches!(
        PackManifest::from_json_str(&no_urls),
        Err(Error::InvalidField { ref field, .. }) if field == "files[0].urls"
    ));

    let unsafe_path = valid_manifest().replacen("mods/wok-core.jar", "../../saves/world", 1);
    assert!(matches!(
        PackManifest::from_json_str(&unsafe_path),
        Err(Error::Json { .. })
    ));

    let local_url = valid_manifest().replacen(
        "https://cdn.example.test/files/wok-core.jar?token=abc",
        "file:///C:/payload.jar",
        1,
    );
    assert!(matches!(
        PackManifest::from_json_str(&local_url),
        Err(Error::InvalidField { ref field, .. }) if field == "files[0].urls[0]"
    ));
}

#[test]
fn sha1_is_canonicalized_and_case_insensitive_duplicates_are_rejected() {
    let uppercase = valid_manifest().replacen(SHA1, &SHA1.to_uppercase(), 1);
    let parsed = PackManifest::from_json_str(&uppercase).unwrap();
    assert_eq!(parsed.files[0].sha1.as_str(), SHA1);

    let duplicate = valid_manifest().replacen(
        "]\n        }",
        &format!(
            r#",{{
                "path":"MODS/WOK-CORE.JAR","sha1":"{SHA1}","size":1,
                "policy":"optional","urls":["https://cdn.example.test/duplicate"]
            }}]
        }}"#
        ),
        1,
    );
    assert!(matches!(
        PackManifest::from_json_str(&duplicate),
        Err(Error::DuplicatePath { .. })
    ));
}
