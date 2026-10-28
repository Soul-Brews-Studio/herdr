use clap::{Arg, ArgAction, Command};

use super::json_flag;

pub(super) fn command() -> Command {
    Command::new("space")
        .about("Fold space groups in this client's sidebar")
        .subcommand(
            group_command("fold", "Fold space groups in this client's sidebar").arg(all_flag()),
        )
        .subcommand(
            group_command("unfold", "Unfold space groups in this client's sidebar").arg(all_flag()),
        )
        .subcommand(group_command(
            "toggle",
            "Fold or unfold space groups in this client's sidebar",
        ))
        .subcommand(
            Command::new("fold-others")
                .about("Fold every other space group, or unfold all when the others are folded")
                .arg(
                    Arg::new("group")
                        .value_name("GROUP")
                        .help("Group to keep open (default: the focused workspace's group)"),
                )
                .arg(json_flag()),
        )
        .subcommand(
            Command::new("folds")
                .about("Show which space groups this client's sidebar folds")
                .arg(json_flag()),
        )
}

fn group_command(name: &'static str, about: &'static str) -> Command {
    Command::new(name)
        .about(about)
        .arg(Arg::new("group").value_name("GROUP").num_args(1..).help(
            "Main space label, repo name, a path in the repo, or a workspace id in the group",
        ))
        .arg(json_flag())
}

fn all_flag() -> Arg {
    Arg::new("all")
        .long("all")
        .action(ArgAction::SetTrue)
        .help("Apply to every space group")
}
