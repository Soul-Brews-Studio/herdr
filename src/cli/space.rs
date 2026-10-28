use serde::Serialize;

use crate::api::schema::{Method, Request};

const HELP: &str = "Usage:
  herdr space fold (<group>... | --all) [--json]
  herdr space unfold (<group>... | --all) [--json]
  herdr space toggle <group>... [--json]
  herdr space fold-others [<group>] [--json]
  herdr space folds [--json]

A group is a repo's main space with its worktree spaces under it, the ▾ arrow in the
sidebar. <group> is the main space's label, the repo name, a path inside the repo, or the
id of any workspace in the group. fold-others folds every other group, or unfolds all
when the others are already folded; without <group> it keeps the focused workspace's
group. Folds are this client's sidebar state: open Herdr clients apply them within a
second, and they survive a client restart.
Use `herdr --machine <label> space ...` for a saved SSH machine's groups.";

pub(super) fn run_space_command(args: &[String]) -> std::io::Result<i32> {
    match args.first().map(String::as_str) {
        Some(command @ ("fold" | "unfold" | "toggle" | "fold-others")) => {
            change_folds(command, &args[1..])
        }
        Some("folds") => folds(&args[1..]),
        Some("help" | "--help" | "-h") => {
            println!("{HELP}");
            Ok(0)
        }
        _ => {
            eprintln!("{HELP}");
            Ok(2)
        }
    }
}

/// One sidebar group: a repo with a main space and at least one worktree space.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SpaceGroup {
    key: String,
    label: String,
    repo_name: String,
    repo_root: String,
    workspace_ids: Vec<String>,
    focused: bool,
}

#[derive(Serialize)]
struct GroupFoldRow<'a> {
    key: &'a str,
    label: &'a str,
    folded: bool,
}

/// Groups the same way the sidebar does (`sidebar::workspace_entries`): workspaces that
/// share a repo key, where at least one is the main checkout and one is a linked worktree.
fn groups_from_workspace_list(workspaces: &[serde_json::Value]) -> Vec<SpaceGroup> {
    let mut groups: Vec<(SpaceGroup, bool, bool)> = Vec::new();
    for workspace in workspaces {
        let Some(worktree) = workspace.get("worktree").filter(|value| value.is_object()) else {
            continue;
        };
        let Some(key) = worktree["repo_key"].as_str() else {
            continue;
        };
        let linked = worktree["is_linked_worktree"].as_bool().unwrap_or(false);
        let id = workspace["workspace_id"].as_str().unwrap_or_default();
        let label = workspace["label"].as_str().unwrap_or_default();
        let focused = workspace["focused"].as_bool().unwrap_or(false);
        let index = match groups.iter().position(|(group, _, _)| group.key == key) {
            Some(index) => index,
            None => {
                groups.push((
                    SpaceGroup {
                        key: key.to_owned(),
                        label: String::new(),
                        repo_name: worktree["repo_name"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        repo_root: worktree["repo_root"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        workspace_ids: Vec::new(),
                        focused: false,
                    },
                    false,
                    false,
                ));
                groups.len() - 1
            }
        };
        let (group, has_main, has_linked) = &mut groups[index];
        group.workspace_ids.push(id.to_owned());
        group.focused |= focused;
        if linked {
            *has_linked = true;
        } else {
            if group.label.is_empty() {
                group.label = label.to_owned();
            }
            *has_main = true;
        }
    }
    groups
        .into_iter()
        .filter(|(_, has_main, has_linked)| *has_main && *has_linked)
        .map(|(group, _, _)| group)
        .collect()
}

fn find_group<'a>(groups: &'a [SpaceGroup], selector: &str) -> Result<&'a SpaceGroup, String> {
    let rules: [&dyn Fn(&SpaceGroup) -> bool; 3] = [
        &|group| group.key == selector || group.workspace_ids.iter().any(|id| id == selector),
        &|group| group.label == selector,
        &|group| group.repo_name == selector,
    ];
    for rule in rules {
        let mut matches = groups.iter().filter(|group| rule(group));
        if let Some(group) = matches.next() {
            if matches.next().is_some() {
                return Err(format!(
                    "space group '{selector}' is ambiguous; use a workspace id or its repo path"
                ));
            }
            return Ok(group);
        }
    }
    // a path inside a repo: the innermost repo root wins (worktrees often sit under it)
    let selector_path = selector.trim_end_matches('/');
    if let Some(group) = groups
        .iter()
        .filter(|group| {
            !group.repo_root.is_empty()
                && (selector_path == group.repo_root
                    || selector_path.starts_with(&format!("{}/", group.repo_root)))
        })
        .max_by_key(|group| group.repo_root.len())
    {
        return Ok(group);
    }
    Err(format!(
        "unknown space group '{selector}'; use `herdr space folds`"
    ))
}

/// Same rules as `herdr machine fold` and friends. `groups` are the keys present now.
fn apply_fold_command(
    command: &str,
    targets: &[String],
    groups: &[String],
    mut folded: Vec<String>,
) -> Vec<String> {
    match command {
        "fold" => folded.extend(targets.iter().cloned()),
        "unfold" => folded.retain(|group| !targets.contains(group)),
        "fold-others" => {
            let others = groups
                .iter()
                .filter(|group| !targets.contains(group))
                .cloned()
                .collect::<Vec<_>>();
            folded = if others.iter().all(|other| folded.contains(other)) {
                folded
                    .into_iter()
                    .filter(|group| !groups.contains(group))
                    .collect()
            } else {
                // keep folds of groups that are not open now; fold the others, open the target
                folded.retain(|group| !groups.contains(group));
                folded.extend(others);
                folded
            };
        }
        _ => {
            for target in targets {
                if folded.contains(target) {
                    folded.retain(|group| group != target);
                } else {
                    folded.push(target.clone());
                }
            }
        }
    }
    folded.sort();
    folded.dedup();
    folded
}

fn load_groups() -> std::io::Result<Result<Vec<SpaceGroup>, String>> {
    let response = super::send_request(&Request {
        id: "cli:space:list".into(),
        method: Method::WorkspaceList(Default::default()),
    })?;
    if response.get("error").is_some() {
        return Ok(Err(serde_json::to_string(&response).unwrap_or_default()));
    }
    let workspaces = response["result"]["workspaces"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(Ok(groups_from_workspace_list(&workspaces)))
}

fn rows<'a>(groups: &'a [SpaceGroup], folded: &[String]) -> Vec<GroupFoldRow<'a>> {
    groups
        .iter()
        .map(|group| GroupFoldRow {
            key: &group.key,
            label: if group.label.is_empty() {
                &group.repo_name
            } else {
                &group.label
            },
            folded: folded.contains(&group.key),
        })
        .collect()
}

fn print_folds(rows: &[GroupFoldRow<'_>], json: bool) -> std::io::Result<i32> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(rows).map_err(std::io::Error::other)?
        );
    } else {
        for row in rows {
            let state = if row.folded { "folded" } else { "unfolded" };
            println!("{}\t{}\t{state}", row.label, row.key);
        }
    }
    Ok(0)
}

fn folds(args: &[String]) -> std::io::Result<i32> {
    let json = match args {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => {
            eprintln!("usage: herdr space folds [--json]");
            return Ok(2);
        }
    };
    let groups = match load_groups()? {
        Ok(groups) => groups,
        Err(error) => {
            eprintln!("{error}");
            return Ok(1);
        }
    };
    let path = crate::client::shell_preferences::local_client_path();
    let profile_id = super::target::remote_profile_id();
    let folded =
        crate::client::shell_preferences::load_collapsed_groups(&path, profile_id.as_deref())
            .map_err(std::io::Error::other)?;
    print_folds(&rows(&groups, &folded), json)
}

fn change_folds(command: &str, args: &[String]) -> std::io::Result<i32> {
    let usage = match command {
        "fold" | "unfold" => format!("usage: herdr space {command} (<group>... | --all) [--json]"),
        "fold-others" => "usage: herdr space fold-others [<group>] [--json]".to_owned(),
        _ => "usage: herdr space toggle <group>... [--json]".to_owned(),
    };
    let json = args.iter().any(|arg| arg == "--json");
    let all = args.iter().any(|arg| arg == "--all");
    let selectors = args
        .iter()
        .filter(|arg| *arg != "--json" && *arg != "--all")
        .collect::<Vec<_>>();
    let valid = match command {
        "fold" | "unfold" => all == selectors.is_empty(),
        "fold-others" => !all && selectors.len() <= 1,
        _ => !all && !selectors.is_empty(),
    };
    if !valid || selectors.iter().any(|arg| arg.starts_with('-')) {
        eprintln!("{usage}");
        return Ok(2);
    }
    let groups = match load_groups()? {
        Ok(groups) => groups,
        Err(error) => {
            eprintln!("{error}");
            return Ok(1);
        }
    };
    let mut targets = Vec::with_capacity(selectors.len().max(1));
    for selector in &selectors {
        match find_group(&groups, selector) {
            Ok(group) => targets.push(group.key.clone()),
            Err(error) => {
                eprintln!("{error}");
                return Ok(2);
            }
        }
    }
    if command == "fold-others" && targets.is_empty() {
        match groups.iter().find(|group| group.focused) {
            Some(group) => targets.push(group.key.clone()),
            None => {
                eprintln!("no focused space group; name one: {usage}");
                return Ok(2);
            }
        }
    }
    let keys = groups
        .iter()
        .map(|group| group.key.clone())
        .collect::<Vec<_>>();
    if all {
        targets = keys.clone();
    }
    let path = crate::client::shell_preferences::local_client_path();
    let profile_id = super::target::remote_profile_id();
    let current =
        crate::client::shell_preferences::load_collapsed_groups(&path, profile_id.as_deref())
            .map_err(std::io::Error::other)?;
    let next = apply_fold_command(command, &targets, &keys, current);
    crate::client::shell_preferences::store_collapsed_groups(
        &path,
        profile_id.as_deref(),
        next.clone(),
    )
    .map_err(std::io::Error::other)?;
    print_folds(&rows(&groups, &next), json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(
        id: &str,
        label: &str,
        key: &str,
        linked: bool,
        focused: bool,
    ) -> serde_json::Value {
        serde_json::json!({
            "workspace_id": id,
            "label": label,
            "focused": focused,
            "worktree": {
                "repo_key": key,
                "repo_name": key.trim_end_matches("/.git").rsplit('/').next().unwrap(),
                "repo_root": key.trim_end_matches("/.git"),
                "is_linked_worktree": linked,
            },
        })
    }

    fn sample() -> Vec<SpaceGroup> {
        groups_from_workspace_list(&[
            workspace("w1", "manager-team", "/c/manager-team/.git", false, false),
            workspace("w2", "wt/athena-oracle", "/c/manager-team/.git", true, true),
            workspace("w3", "rd-team", "/c/rd-team/.git", false, false),
            workspace("w4", "wt/neo-oracle", "/c/rd-team/.git", true, false),
            // a main checkout without worktree spaces is not a group
            workspace("w5", "solo", "/c/solo/.git", false, false),
            // worktree spaces without their main space are not a group either
            workspace("w6", "wt/x", "/c/orphan/.git", true, false),
            serde_json::json!({"workspace_id": "w7", "label": "plain", "worktree": null}),
        ])
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn groups_match_the_sidebar() {
        let groups = sample();
        assert_eq!(
            groups
                .iter()
                .map(|group| group.label.as_str())
                .collect::<Vec<_>>(),
            ["manager-team", "rd-team"]
        );
        assert_eq!(groups[0].workspace_ids, ["w1", "w2"]);
        assert!(groups[0].focused);
        assert!(!groups[1].focused);
    }

    #[test]
    fn find_group_by_label_repo_path_key_or_member() {
        let groups = sample();
        for selector in [
            "manager-team",
            "/c/manager-team",
            "/c/manager-team/",
            "/c/manager-team/wt/athena-oracle",
            "/c/manager-team/.git",
            "w2",
        ] {
            assert_eq!(
                find_group(&groups, selector).expect(selector).key,
                "/c/manager-team/.git",
                "{selector}"
            );
        }
        assert!(find_group(&groups, "solo").is_err());
        assert!(find_group(&groups, "/c/manager-team-2").is_err());
    }

    #[test]
    fn ambiguous_label_asks_for_an_id() {
        let mut groups = sample();
        groups[1].label = "manager-team".into();
        assert!(find_group(&groups, "manager-team")
            .unwrap_err()
            .contains("ambiguous"));
        assert_eq!(
            find_group(&groups, "w4").expect("id").key,
            "/c/rd-team/.git"
        );
    }

    #[test]
    fn fold_commands_follow_sidebar_rules() {
        let groups = strings(&["/a", "/b", "/c"]);
        assert_eq!(
            apply_fold_command("fold", &strings(&["/b"]), &groups, strings(&["/old"])),
            ["/b", "/old"]
        );
        assert_eq!(
            apply_fold_command("unfold", &strings(&["/b"]), &groups, strings(&["/a", "/b"])),
            ["/a"]
        );
        assert_eq!(
            apply_fold_command("toggle", &strings(&["/a", "/b"]), &groups, strings(&["/a"])),
            ["/b"]
        );
        let folded = apply_fold_command(
            "fold-others",
            &strings(&["/a"]),
            &groups,
            strings(&["/a", "/old"]),
        );
        assert_eq!(folded, ["/b", "/c", "/old"]);
        assert_eq!(
            apply_fold_command("fold-others", &strings(&["/a"]), &groups, folded),
            ["/old"]
        );
    }
}
