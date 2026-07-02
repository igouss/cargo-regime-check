//! Adapter: enumerate a workspace's gated crates via `cargo metadata`.
//!
//! Workspace mode needs to know two things about every member crate: its name
//! and its root directory (so it can look for a `regime-transition.toml` and,
//! later, run `cargo public-api -p <name>` against it). `cargo metadata
//! --no-deps --format-version 1` answers both — it lists *exactly* the workspace
//! members (dependencies excluded), each with its `name` and absolute
//! `manifest_path`; the member root is that manifest's parent directory.
//!
//! Hexagonal split (the reason this file has three layers):
//! - [`parse_members`] is a **pure** function `metadata JSON → [WorkspaceMember]`.
//!   It touches no process and no filesystem, so it is unit-testable against a
//!   captured sample.
//! - [`run_metadata`] is the **process** boundary: it invokes cargo in a given
//!   directory and hands back the raw JSON text.
//! - [`gated_crates`] composes the two and adds the **filesystem** presence
//!   check, keeping only members that carry a readable `regime-transition.toml`.
//!
//! The pure domain never learns any of this exists; dependencies point inward.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::Deserialize;

/// The `regime-transition.toml` filename a gated crate must carry at its root.
const REGIME_FILE: &str = "regime-transition.toml";

/// One workspace member as reported by `cargo metadata`: its package name and
/// its root directory (the parent of its `Cargo.toml`). This is the raw
/// enumeration, *before* the gated-crate filter — members with no regime file
/// are still present here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMember {
    pub name: String,
    pub root: PathBuf,
}

/// A workspace member that opted into gating by carrying a readable
/// `regime-transition.toml` at its root. `regime_path` is `root/REGIME_FILE`,
/// resolved once here so callers never reconstruct it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatedCrate {
    pub name: String,
    pub root: PathBuf,
    pub regime_path: PathBuf,
}

/// Failure modes of gated-crate enumeration. The process and JSON layers each
/// contribute one; the pure parse only ever yields [`CargoMetadataError::Json`].
#[derive(Debug, thiserror::Error)]
pub enum CargoMetadataError {
    #[error("could not run `cargo metadata` in {dir}: {source}")]
    Spawn { dir: String, source: std::io::Error },
    #[error("`cargo metadata` failed (exit {code}) in {dir}: {stderr}")]
    Cargo {
        dir: String,
        code: String,
        stderr: String,
    },
    #[error("`cargo metadata` output was not valid UTF-8: {0}")]
    Utf8(std::string::FromUtf8Error),
    #[error("could not parse `cargo metadata` JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// The slice of the `cargo metadata` JSON we consume. With `--no-deps`,
/// `packages` is exactly the workspace members. Every other top-level field
/// (resolve, target_directory, version, …) and every unlisted package field is
/// ignored by serde, so this stays robust across cargo versions.
#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
struct Package {
    name: String,
    manifest_path: String,
}

/// The root directory of a crate given its `Cargo.toml` path: the manifest's
/// parent. `cargo metadata` always reports an absolute path ending in a normal
/// `Cargo.toml` component, so the parent always exists; the fallback is
/// unreachable but keeps this total (no panic) rather than trusting that.
fn manifest_root(manifest_path: &str) -> PathBuf {
    Path::new(manifest_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(manifest_path))
}

/// Parse `cargo metadata --no-deps --format-version 1` output into the workspace
/// members. **Pure**: no process, no filesystem — the split that makes it
/// unit-testable against a captured sample.
pub fn parse_members(metadata_json: &str) -> Result<Vec<WorkspaceMember>, CargoMetadataError> {
    let metadata: Metadata = serde_json::from_str(metadata_json)?;
    let members: Vec<WorkspaceMember> = metadata
        .packages
        .into_iter()
        .map(|package: Package| WorkspaceMember {
            name: package.name,
            root: manifest_root(&package.manifest_path),
        })
        .collect();
    Ok(members)
}

/// Invoke `cargo metadata --no-deps --format-version 1` with `dir` as the
/// working directory and return its stdout. The **process** boundary; honours
/// the `CARGO` env var cargo sets for its subcommands, falling back to `cargo`
/// on `PATH`. `--no-deps` keeps this offline (no dependency resolution / fetch).
fn run_metadata(dir: &Path) -> Result<String, CargoMetadataError> {
    let cargo: OsString = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output: Output = Command::new(cargo)
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version")
        .arg("1")
        .current_dir(dir)
        .output()
        .map_err(|source: std::io::Error| CargoMetadataError::Spawn {
            dir: dir.display().to_string(),
            source,
        })?;

    if !output.status.success() {
        let code: String = output
            .status
            .code()
            .map(|c: i32| c.to_string())
            .unwrap_or_else(|| "signal".to_owned());
        let stderr: String = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(CargoMetadataError::Cargo {
            dir: dir.display().to_string(),
            code,
            stderr,
        });
    }

    let text: String = String::from_utf8(output.stdout).map_err(CargoMetadataError::Utf8)?;
    Ok(text)
}

/// Whether `path` is a readable regular file. `is_file` rejects directories and
/// dangling symlinks; opening it confirms read permission — so a member with an
/// unreadable regime file is simply not gated, never a false green.
fn has_readable_regime(path: &Path) -> bool {
    path.is_file() && std::fs::File::open(path).is_ok()
}

/// Enumerate the gated crates of the workspace rooted at `dir`: run
/// `cargo metadata`, parse the members, and keep only those carrying a readable
/// `regime-transition.toml` at their root. Members without one are silently
/// dropped here — opting out of gating is not an error (the orchestrator decides
/// what a *zero gated crates* run means).
pub fn gated_crates(dir: &Path) -> Result<Vec<GatedCrate>, CargoMetadataError> {
    let text: String = run_metadata(dir)?;
    let members: Vec<WorkspaceMember> = parse_members(&text)?;

    let mut gated: Vec<GatedCrate> = Vec::new();
    for member in members {
        let regime_path: PathBuf = member.root.join(REGIME_FILE);
        if has_readable_regime(&regime_path) {
            gated.push(GatedCrate {
                name: member.name,
                root: member.root,
                regime_path,
            });
        }
    }
    Ok(gated)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trimmed capture of real `cargo metadata --no-deps --format-version 1`
    // output (verified against cargo 2026-07-01): the top-level `packages`
    // array plus the sibling fields we ignore, and per package the `name` +
    // `manifest_path` we read alongside fields we ignore. Two members, one with
    // deliberately extra fields, prove unknown-field tolerance.
    const SAMPLE_MANY: &str = r#"{
      "packages": [
        {
          "name": "alpha",
          "version": "0.1.0",
          "id": "path+file:///ws/alpha#0.1.0",
          "manifest_path": "/ws/alpha/Cargo.toml",
          "edition": "2021",
          "dependencies": []
        },
        {
          "name": "beta",
          "manifest_path": "/ws/crates/beta/Cargo.toml"
        }
      ],
      "workspace_members": ["path+file:///ws/alpha#0.1.0"],
      "resolve": null,
      "target_directory": "/ws/target",
      "version": 1,
      "workspace_root": "/ws"
    }"#;

    // ---- zero: an empty workspace yields no members ----
    #[test]
    fn parses_zero_members() {
        let members: Vec<WorkspaceMember> = parse_members(r#"{"packages": []}"#).unwrap();
        assert!(members.is_empty());
    }

    // ---- one: a single package yields one member with its dirname as root ----
    #[test]
    fn parses_one_member_with_manifest_dirname_as_root() {
        let json: &str =
            r#"{"packages": [{"name": "solo", "manifest_path": "/ws/solo/Cargo.toml"}]}"#;
        let members: Vec<WorkspaceMember> = parse_members(json).unwrap();
        assert_eq!(
            members,
            vec![WorkspaceMember {
                name: "solo".to_owned(),
                root: PathBuf::from("/ws/solo"),
            }]
        );
    }

    // ---- many: every package becomes a member, unknown fields ignored ----
    #[test]
    fn parses_many_members_ignoring_unknown_fields() {
        let members: Vec<WorkspaceMember> = parse_members(SAMPLE_MANY).unwrap();
        assert_eq!(
            members,
            vec![
                WorkspaceMember {
                    name: "alpha".to_owned(),
                    root: PathBuf::from("/ws/alpha"),
                },
                WorkspaceMember {
                    name: "beta".to_owned(),
                    root: PathBuf::from("/ws/crates/beta"),
                },
            ]
        );
    }

    // ---- malformed JSON is a Json error, never a silent empty enumeration ----
    #[test]
    fn rejects_malformed_json() {
        let err: CargoMetadataError = parse_members("{ not json").unwrap_err();
        assert!(matches!(err, CargoMetadataError::Json(_)));
    }

    // ---- the member root is exactly the manifest's parent directory ----
    #[test]
    fn manifest_root_is_the_parent_directory() {
        assert_eq!(
            manifest_root("/ws/crates/beta/Cargo.toml"),
            PathBuf::from("/ws/crates/beta")
        );
    }
}
