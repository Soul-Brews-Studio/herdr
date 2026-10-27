use std::collections::HashMap;

use crate::api::schema::{
    Method, Request, WorkspaceCreateParams, WorkspaceMoveParams, WorkspaceRenameParams,
    WorkspaceReportMetadataParams,
};

pub(super) fn run_workspace_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(|arg| arg.as_str()) else {
        print_workspace_help();
        return Ok(2);
    };

    match subcommand {
        "list" => workspace_list(&args[1..]),
        "create" => workspace_create(&args[1..]),
        "get" => workspace_get(&args[1..]),
        "focus" => workspace_focus(&args[1..]),
        "rename" => workspace_rename(&args[1..]),
        "move" => workspace_move(&args[1..]),
        "report-metadata" => workspace_report_metadata(&args[1..]),
        "close" => workspace_close(&args[1..]),
        "help" | "--help" | "-h" => {
            print_workspace_help();
            Ok(0)
        }
        _ => {
            print_workspace_help();
            Ok(2)
        }
    }
}

fn workspace_list(args: &[String]) -> std::io::Result<i32> {
    if !args.is_empty() {
        eprintln!("usage: herdr workspace list");
        return Ok(2);
    }

    super::runtime::workspace_list()
}

fn workspace_create(args: &[String]) -> std::io::Result<i32> {
    let mut cwd = None;
    let mut focus = false;
    let mut label = None;
    let mut env = HashMap::new();

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--cwd" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --cwd");
                    return Ok(2);
                };
                cwd = Some(value.clone());
                index += 2;
            }
            "--label" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --label");
                    return Ok(2);
                };
                label = Some(value.clone());
                index += 2;
            }
            "--focus" => {
                focus = true;
                index += 1;
            }
            "--no-focus" => {
                focus = false;
                index += 1;
            }
            "--env" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --env");
                    return Ok(2);
                };
                let (key, value) = match super::parse_env_assignment(value) {
                    Ok(pair) => pair,
                    Err(err) => {
                        eprintln!("{err}");
                        return Ok(2);
                    }
                };
                env.insert(key, value);
                index += 2;
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }

    super::runtime::workspace_create(WorkspaceCreateParams {
        source_workspace_id: None,
        cwd,
        focus,
        label,
        env,
    })
}

fn workspace_get(args: &[String]) -> std::io::Result<i32> {
    let Some(raw_workspace_id) = args.first() else {
        eprintln!("usage: herdr workspace get <workspace_id>");
        return Ok(2);
    };
    if args.len() != 1 {
        eprintln!("usage: herdr workspace get <workspace_id>");
        return Ok(2);
    }

    super::runtime::workspace_get(super::normalize_workspace_id(raw_workspace_id))
}

fn workspace_focus(args: &[String]) -> std::io::Result<i32> {
    let Some(raw_workspace_id) = args.first() else {
        eprintln!("usage: herdr workspace focus <workspace_id>");
        return Ok(2);
    };
    if args.len() != 1 {
        eprintln!("usage: herdr workspace focus <workspace_id>");
        return Ok(2);
    }

    super::runtime::workspace_focus(super::normalize_workspace_id(raw_workspace_id))
}

fn workspace_rename(args: &[String]) -> std::io::Result<i32> {
    if args.len() < 2 {
        eprintln!("usage: herdr workspace rename <workspace_id> <label>");
        return Ok(2);
    }

    super::runtime::workspace_rename(WorkspaceRenameParams {
        workspace_id: super::normalize_workspace_id(&args[0]),
        label: args[1..].join(" "),
    })
}

const WORKSPACE_MOVE_USAGE: &str =
    "usage: herdr workspace move <workspace_id> (--after <workspace_id> | --index N)";

#[derive(Debug, PartialEq, Eq)]
enum MoveTarget {
    After(String),
    Index(usize),
}

fn parse_move_args(args: &[String]) -> Result<(String, MoveTarget), String> {
    match args {
        [workspace_id, flag, value] if flag == "--after" => Ok((
            super::normalize_workspace_id(workspace_id),
            MoveTarget::After(super::normalize_workspace_id(value)),
        )),
        [workspace_id, flag, value] if flag == "--index" => {
            let index = value
                .parse::<usize>()
                .map_err(|_| format!("invalid value for --index: {value}"))?;
            Ok((
                super::normalize_workspace_id(workspace_id),
                MoveTarget::Index(index),
            ))
        }
        _ => Err(WORKSPACE_MOVE_USAGE.to_string()),
    }
}

/// `insert_index` is a gap index counted before the moved workspace is taken
/// out (the server subtracts one when moving forward), so "after X" is always
/// X's current position + 1, whichever side of X the workspace starts on.
fn insert_index_after(order: &[String], after: &str) -> Result<usize, String> {
    order
        .iter()
        .position(|id| id == after)
        .map(|position| position + 1)
        .ok_or_else(|| format!("workspace not found: {after}"))
}

fn workspace_move(args: &[String]) -> std::io::Result<i32> {
    let (workspace_id, target) = match parse_move_args(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}");
            return Ok(2);
        }
    };

    let insert_index = match target {
        MoveTarget::Index(index) => index,
        MoveTarget::After(after) => {
            let response = super::send_request(&Request {
                id: "cli:workspace:move:list".into(),
                method: Method::WorkspaceList(Default::default()),
            })?;
            if response.get("error").is_some() {
                eprintln!("{}", serde_json::to_string(&response).unwrap());
                return Ok(1);
            }
            let order = response["result"]["workspaces"]
                .as_array()
                .map(|workspaces| {
                    workspaces
                        .iter()
                        .filter_map(|workspace| workspace["workspace_id"].as_str())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            match insert_index_after(&order, &after) {
                Ok(index) => index,
                Err(message) => {
                    eprintln!("{message}");
                    return Ok(1);
                }
            }
        }
    };

    super::runtime::workspace_move(WorkspaceMoveParams {
        workspace_id,
        insert_index,
    })
}

fn workspace_report_metadata(args: &[String]) -> std::io::Result<i32> {
    let Some(raw_workspace_id) = args.first() else {
        eprintln!("usage: herdr workspace report-metadata <workspace_id> --source ID [--token NAME=VALUE] [--clear-token NAME] [--seq N] [--ttl-ms N]");
        return Ok(2);
    };
    let workspace_id = super::normalize_workspace_id(raw_workspace_id);
    let mut source = None;
    let mut tokens = HashMap::new();
    let mut seq = None;
    let mut ttl_ms = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --source");
                    return Ok(2);
                };
                source = Some(value.clone());
                index += 2;
            }
            "--token" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --token");
                    return Ok(2);
                };
                let (key, value) = match super::parse_token_assignment(value) {
                    Ok(token) => token,
                    Err(message) => {
                        eprintln!("{message}");
                        return Ok(2);
                    }
                };
                tokens.insert(key, value);
                index += 2;
            }
            "--clear-token" => {
                let Some(key) = args.get(index + 1) else {
                    eprintln!("missing value for --clear-token");
                    return Ok(2);
                };
                tokens.insert(key.clone(), None);
                index += 2;
            }
            "--seq" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --seq");
                    return Ok(2);
                };
                seq = Some(super::parse_u64_flag("--seq", value)?);
                index += 2;
            }
            "--ttl-ms" => {
                let Some(value) = args.get(index + 1) else {
                    eprintln!("missing value for --ttl-ms");
                    return Ok(2);
                };
                ttl_ms = Some(super::parse_u64_flag("--ttl-ms", value)?);
                index += 2;
            }
            other => {
                eprintln!("unknown option: {other}");
                return Ok(2);
            }
        }
    }
    let Some(source) = source.filter(|source| !source.trim().is_empty()) else {
        eprintln!("missing required --source");
        return Ok(2);
    };
    if tokens.is_empty() {
        eprintln!("missing token to set or clear");
        return Ok(2);
    }
    super::send_ok_request(Method::WorkspaceReportMetadata(
        WorkspaceReportMetadataParams {
            workspace_id,
            source,
            tokens,
            seq,
            ttl_ms,
        },
    ))
}

fn workspace_close(args: &[String]) -> std::io::Result<i32> {
    let (raw_workspace_id, close_group) = match args {
        [workspace_id] => (workspace_id, false),
        [workspace_id, flag] if flag == "--group" => (workspace_id, true),
        _ => {
            eprintln!("usage: herdr workspace close <workspace_id> [--group]");
            return Ok(2);
        }
    };

    super::runtime::workspace_close(crate::api::schema::WorkspaceCloseParams {
        workspace_id: super::normalize_workspace_id(raw_workspace_id),
        close_group,
    })
}

fn print_workspace_help() {
    eprintln!("herdr workspace commands:");
    eprintln!("  herdr workspace list");
    eprintln!("  herdr workspace create [--cwd PATH] [--label TEXT] [--env KEY=VALUE] [--focus] [--no-focus]");
    eprintln!("  herdr workspace get <workspace_id>");
    eprintln!("  herdr workspace focus <workspace_id>");
    eprintln!("  herdr workspace rename <workspace_id> <label>");
    eprintln!("  herdr workspace move <workspace_id> (--after <workspace_id> | --index N)");
    eprintln!("  herdr workspace report-metadata <workspace_id> --source ID [--token NAME=VALUE] [--clear-token NAME] [--seq N] [--ttl-ms N]");
    eprintln!("  herdr workspace close <workspace_id> [--group]");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn order(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn parse_move_after_and_index() {
        assert_eq!(
            parse_move_args(&args(&["w3", "--after", "w1"])),
            Ok(("w3".into(), MoveTarget::After("w1".into())))
        );
        assert_eq!(
            parse_move_args(&args(&["w3", "--index", "0"])),
            Ok(("w3".into(), MoveTarget::Index(0)))
        );
    }

    #[test]
    fn parse_move_rejects_bad_input() {
        assert!(parse_move_args(&args(&["w3"])).is_err());
        assert!(parse_move_args(&args(&["w3", "--index", "-1"])).is_err());
        assert!(parse_move_args(&args(&["w3", "--after", "w1", "--index", "2"])).is_err());
        assert!(parse_move_args(&args(&["w3", "--before", "w1"])).is_err());
    }

    #[test]
    fn insert_index_after_is_target_position_plus_one() {
        let ids = order(&["w1", "w2", "w3", "w4"]);
        assert_eq!(insert_index_after(&ids, "w1"), Ok(1));
        assert_eq!(insert_index_after(&ids, "w4"), Ok(4));
        assert!(insert_index_after(&ids, "w9").is_err());
    }

    /// Mirrors `AppState::move_workspace`: remove first, then insert at
    /// `insert - 1` when moving forward.
    fn apply_move(ids: &[String], source: &str, insert: usize) -> Vec<String> {
        let mut out = ids.to_vec();
        let from = out.iter().position(|id| id == source).unwrap();
        let to = if from < insert { insert - 1 } else { insert };
        let moved = out.remove(from);
        out.insert(to, moved);
        out
    }

    #[test]
    fn after_lands_right_after_target_in_both_directions() {
        let ids = order(&["w1", "w2", "w3", "w4"]);
        // forward: source before target
        let index = insert_index_after(&ids, "w3").unwrap();
        assert_eq!(
            apply_move(&ids, "w1", index),
            order(&["w2", "w3", "w1", "w4"])
        );
        // backward: source after target
        let index = insert_index_after(&ids, "w1").unwrap();
        assert_eq!(
            apply_move(&ids, "w4", index),
            order(&["w1", "w4", "w2", "w3"])
        );
    }
}
