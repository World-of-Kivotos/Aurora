#![allow(dead_code)]

use aurora_modpack::diff::DiskFile;
use aurora_modpack::model::{
    FilePolicy, LoaderKind, LoaderSpec, ManifestFile, PackManifest, SCHEMA_VERSION, Sha1Digest,
};
use aurora_modpack::path::SafeRelativePath;
use aurora_modpack::snapshot::{AppliedSnapshot, SnapshotEntry, SnapshotWorkingDirectory};

pub const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccc";
pub const WORKING_DIRECTORY: SnapshotWorkingDirectory =
    SnapshotWorkingDirectory::IsolatedVersionDirectory;

pub fn safe(path: &str) -> SafeRelativePath {
    SafeRelativePath::new(path).unwrap()
}

pub fn sha(value: &str) -> Sha1Digest {
    Sha1Digest::new(value).unwrap()
}

pub fn remote(path: &str, policy: FilePolicy, digest: &str) -> ManifestFile {
    ManifestFile {
        path: safe(path),
        sha1: sha(digest),
        size: 128,
        policy,
        urls: vec![format!("https://cdn.example.test/{path}")],
    }
}

pub fn manifest(files: Vec<ManifestFile>) -> PackManifest {
    PackManifest {
        schema: SCHEMA_VERSION,
        pack_id: "wok".to_owned(),
        version: "2.0.0".to_owned(),
        minecraft: "1.20.1".to_owned(),
        loader: LoaderSpec {
            kind: LoaderKind::Forge,
            version: "47.4.20".to_owned(),
        },
        files,
    }
}

pub fn previous(path: &str, policy: FilePolicy, digest: &str) -> SnapshotEntry {
    SnapshotEntry {
        path: safe(path),
        sha1: sha(digest),
        policy,
    }
}

pub fn snapshot(files: Vec<SnapshotEntry>) -> AppliedSnapshot {
    AppliedSnapshot {
        schema: SCHEMA_VERSION,
        pack_id: "wok".to_owned(),
        version: "1.0.0".to_owned(),
        working_directory: WORKING_DIRECTORY,
        files,
    }
}

pub fn disk(path: &str, digest: &str) -> DiskFile {
    DiskFile::new(safe(path), sha(digest))
}
