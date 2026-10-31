use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct ClientRemoteCollapsedGroups {
    pub(super) profile_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) collapsed_groups: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(super) struct ClientChromePreferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) sidebar_width: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) sidebar_section_split: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) sidebar_collapsed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) agent_panel_sort: Option<crate::config::AgentPanelSortConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) collapsed_groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) remote_collapsed_groups: Vec<ClientRemoteCollapsedGroups>,
    /// Folded sidebar machines: `"local"` or a saved SSH profile id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) collapsed_machines: Vec<String>,
}

pub(crate) const LOCAL_MACHINE_KEY: &str = "local";

pub(super) fn path_for_local_endpoint(socket_path: &Path) -> PathBuf {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in socket_path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    crate::config::state_dir()
        .join("client-shell")
        .join(format!("local-{hash:016x}.json"))
}

/// SSH preferences belong to the target/session, not its per-process bridge socket.
pub(super) fn path_for_remote_endpoint(target: &str, session: &str) -> PathBuf {
    use sha2::{Digest as _, Sha256};

    let mut hash = Sha256::new();
    for part in [target, session] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    crate::config::state_dir()
        .join("client-shell")
        .join(format!("remote-{:x}.json", hash.finalize()))
}

/// Preferences file of a local client on this socket; `herdr machine fold` edits it.
pub(crate) fn local_client_path() -> PathBuf {
    path_for_local_endpoint(&crate::server::socket_paths::client_socket_path())
}

fn read(path: &Path) -> Result<Option<ClientChromePreferences>, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to read {}: {error}", path.display())),
    };
    serde_json::from_str(&content)
        .map(Some)
        .map_err(|error| format!("invalid client shell state {}: {error}", path.display()))
}

pub(crate) fn load_collapsed_machines(path: &Path) -> Result<Vec<String>, String> {
    Ok(read(path)?
        .map(|preferences| preferences.collapsed_machines)
        .unwrap_or_default())
}

/// Replaces only the folded machines, keeping every other saved preference.
pub(crate) fn store_collapsed_machines(
    path: &Path,
    mut machines: Vec<String>,
) -> Result<(), String> {
    let mut preferences = read(path)?.unwrap_or_default();
    machines.sort();
    machines.dedup();
    preferences.collapsed_machines = machines;
    store(path, preferences)
}

pub(super) fn load(path: &Path) -> Option<ClientChromePreferences> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub(super) fn store(path: &Path, preferences: ClientChromePreferences) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid client shell state path: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create client shell state directory: {error}"))?;
    let content = serde_json::to_vec_pretty(&preferences)
        .map_err(|error| format!("failed to encode client shell state: {error}"))?;
    let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = path
        .file_name()
        .ok_or_else(|| format!("invalid client shell state path: {}", path.display()))?
        .to_os_string();
    temp_name.push(format!(".tmp-{}-{sequence}", std::process::id()));
    let temp_path = parent.join(temp_name);
    std::fs::write(&temp_path, content)
        .map_err(|error| format!("failed to write client shell state: {error}"))?;
    std::fs::rename(&temp_path, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp_path);
        format!("failed to replace client shell state: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_paths_are_stable_and_distinct() {
        let first = path_for_local_endpoint(Path::new("/run/herdr/one.sock"));
        let again = path_for_local_endpoint(Path::new("/run/herdr/one.sock"));
        let second = path_for_local_endpoint(Path::new("/run/herdr/two.sock"));
        assert_eq!(first, again);
        assert_ne!(first, second);
    }

    #[test]
    fn remote_preferences_distinguish_targets_sessions_and_component_boundaries() {
        let original = path_for_remote_endpoint("dev@build", "agents");
        assert_eq!(original, path_for_remote_endpoint("dev@build", "agents"));
        assert_ne!(original, path_for_remote_endpoint("other@build", "agents"));
        assert_ne!(original, path_for_remote_endpoint("dev@build", "other"));
        assert_ne!(
            path_for_remote_endpoint("ab", "c"),
            path_for_remote_endpoint("a", "bc")
        );
        assert_ne!(
            original,
            path_for_local_endpoint(Path::new("dev@build/agents"))
        );
    }

    #[test]
    fn legacy_preferences_default_remote_collapses() {
        let preferences: ClientChromePreferences =
            serde_json::from_str(r#"{"collapsed_groups":["/repo"]}"#)
                .expect("legacy client chrome preferences");

        assert_eq!(preferences.collapsed_groups, ["/repo"]);
        assert!(preferences.remote_collapsed_groups.is_empty());
        assert!(preferences.collapsed_machines.is_empty());
    }

    #[test]
    fn collapsed_machines_store_keeps_other_preferences() {
        let path = std::env::temp_dir().join(format!(
            "herdr-shell-collapsed-machines-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        assert!(load_collapsed_machines(&path)
            .expect("missing file")
            .is_empty());
        store(
            &path,
            ClientChromePreferences {
                sidebar_width: Some(30),
                collapsed_groups: vec!["/repo".into()],
                ..ClientChromePreferences::default()
            },
        )
        .expect("seed preferences");
        store_collapsed_machines(&path, vec!["local".into(), "b".into(), "local".into()])
            .expect("store folds");
        assert_eq!(
            load_collapsed_machines(&path).expect("folds"),
            ["b", "local"]
        );
        let saved = load(&path).expect("preferences");
        assert_eq!(saved.sidebar_width, Some(30));
        assert_eq!(saved.collapsed_groups, ["/repo"]);

        std::fs::write(&path, "not json").expect("corrupt preferences");
        assert!(store_collapsed_machines(&path, Vec::new()).is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("kept"), "not json");
        std::fs::remove_file(path).expect("remove preferences");
    }

    #[test]
    fn concurrent_stores_leave_complete_preferences() {
        let path = std::env::temp_dir().join(format!(
            "herdr-shell-concurrent-preferences-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let writers = (20..28)
            .map(|width| {
                let path = path.clone();
                std::thread::spawn(move || {
                    store(
                        &path,
                        ClientChromePreferences {
                            sidebar_width: Some(width),
                            ..ClientChromePreferences::default()
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        for writer in writers {
            writer.join().expect("preference writer").expect("store");
        }
        assert!(load(&path)
            .and_then(|saved| saved.sidebar_width)
            .is_some_and(|width| (20..28).contains(&width)));
        std::fs::remove_file(path).expect("remove preferences");
    }

    #[test]
    fn repeated_store_replaces_existing_preferences() {
        let path = std::env::temp_dir().join(format!(
            "herdr-shell-preferences-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        store(
            &path,
            ClientChromePreferences {
                sidebar_width: Some(24),
                ..ClientChromePreferences::default()
            },
        )
        .expect("first preference store");
        store(
            &path,
            ClientChromePreferences {
                sidebar_width: Some(32),
                ..ClientChromePreferences::default()
            },
        )
        .expect("replacement preference store");
        assert_eq!(load(&path).and_then(|saved| saved.sidebar_width), Some(32));
        std::fs::remove_file(path).expect("remove preferences");
    }
}
