use clap::{Arg, Command};

use super::{json_flag, option};

pub(super) fn command() -> Command {
    Command::new("machine")
        .about("Manage saved SSH machines")
        .subcommand(
            Command::new("list")
                .about("List saved SSH machines")
                .arg(json_flag()),
        )
        .subcommand(
            Command::new("status")
                .about("Check saved machines without prompting for authentication")
                .arg(Arg::new("machine").value_name("LABEL_OR_ID"))
                .arg(json_flag()),
        )
        .subcommand(
            Command::new("reconnect")
                .about("Authenticate a saved machine in this terminal and verify connectivity")
                .arg(Arg::new("machine").value_name("LABEL_OR_ID").required(true)),
        )
        .subcommand(
            Command::new("add")
                .about("Prepare the remote Herdr server and save an SSH machine")
                .arg(
                    Arg::new("ssh-target")
                        .value_name("SSH_TARGET")
                        .required(true),
                )
                .arg(
                    option("label", "LABEL").help(
                        "Set the machine label shown in the sidebar (defaults to the SSH host, or host/session)",
                    ),
                )
                .arg(
                    option("remote-session", "NAME")
                        .help("Select a session explicitly (default without an interactive terminal)"),
                ),
        )
        .subcommand(
            profile_command("rename", "Rename a saved SSH machine").arg(
                option("label", "LABEL")
                    .required(true)
                    .help("Set the machine label shown in the sidebar"),
            ),
        )
        .subcommand(profile_command("remove", "Remove a saved SSH machine"))
        .subcommand(profile_command("enable", "Enable a saved SSH machine"))
        .subcommand(profile_command("disable", "Disable a saved SSH machine"))
        .subcommand(fold_command("fold", "Fold machines in this client's sidebar").arg(all_flag()))
        .subcommand(fold_command("unfold", "Unfold machines in this client's sidebar").arg(all_flag()))
        .subcommand(fold_command("toggle", "Fold or unfold machines in this client's sidebar"))
        .subcommand(
            Command::new("fold-others")
                .about("Fold every other machine, or unfold all when the others are folded")
                .arg(
                    Arg::new("machine")
                        .value_name("MACHINE")
                        .required_unless_present("same-session"),
                )
                .arg(
                    Arg::new("same-session")
                        .long("same-session")
                        .action(clap::ArgAction::SetTrue)
                        .conflicts_with("machine")
                        .help("Open Local and the machines saved for this client's session; fold the rest"),
                )
                .arg(json_flag()),
        )
        .subcommand(
            Command::new("folds")
                .about("Show which machines this client's sidebar folds")
                .arg(json_flag()),
        )
}

fn fold_command(name: &'static str, about: &'static str) -> Command {
    Command::new(name)
        .about(about)
        .arg(
            Arg::new("machine")
                .value_name("MACHINE")
                .num_args(1..)
                .help("`local`, a machine label, or a profile ID"),
        )
        .arg(json_flag())
}

fn all_flag() -> Arg {
    Arg::new("all")
        .long("all")
        .action(clap::ArgAction::SetTrue)
        .help("Apply to every machine")
}

fn profile_command(name: &'static str, about: &'static str) -> Command {
    Command::new(name).about(about).arg(
        Arg::new("profile-id")
            .value_name("PROFILE_ID")
            .required(true),
    )
}
