//! `tynd completions`.

use std::io::Write;

use clap::CommandFactory;
use clap_complete::Shell;

use crate::args::{Cli, CompletionShell};

/// Prints the completion script for `shell`, generated from the clap command tree.
pub(super) fn completions(shell: CompletionShell) -> anyhow::Result<()> {
    // Generated into a buffer: clap_complete panics when stdout is a pipe the
    // reader closed early (`tynd completions bash | head`).
    let mut script = Vec::new();
    write_completions(shell, &mut script);
    crate::output::write_stdout(&script)
}

fn write_completions(shell: CompletionShell, out: &mut dyn Write) {
    let shell = match shell {
        CompletionShell::Bash => Shell::Bash,
        CompletionShell::Zsh => Shell::Zsh,
        CompletionShell::Fish => Shell::Fish,
        CompletionShell::Powershell => Shell::PowerShell,
    };
    let mut command = Cli::command();
    let name = command.get_name().to_owned();
    clap_complete::generate(shell, &mut command, name, out);
}

#[cfg(test)]
mod tests {
    use super::write_completions;
    use crate::args::{Cli, CompletionShell};
    use clap::CommandFactory;

    #[test]
    fn completions_cover_every_top_level_subcommand() {
        let mut script = Vec::new();
        write_completions(CompletionShell::Fish, &mut script);
        let script = String::from_utf8(script).expect("completion script is UTF-8");
        let command = Cli::command();
        let names: Vec<&str> = command
            .get_subcommands()
            .map(clap::Command::get_name)
            .collect();
        assert!(
            names.contains(&"test"),
            "the test subcommand must be listed"
        );
        for name in names {
            assert!(
                script.contains(&format!("-a \"{name}\"")),
                "completion script misses {name}"
            );
        }
    }

    #[test]
    fn completions_cover_nested_subcommands() {
        let mut script = Vec::new();
        write_completions(CompletionShell::Bash, &mut script);
        let script = String::from_utf8(script).expect("completion script is UTF-8");
        let command = Cli::command();
        for group in ["config", "plugin", "updater"] {
            let group_command = command
                .find_subcommand(group)
                .unwrap_or_else(|| panic!("{group} group is missing"));
            assert!(
                group_command.has_subcommands(),
                "{group} has no subcommands"
            );
            for nested in group_command.get_subcommands() {
                let name = nested.get_name();
                assert!(
                    script.contains(&format!("tynd__subcmd__{group}__subcmd__{name})")),
                    "completion script misses `tynd {group} {name}`"
                );
            }
        }
        assert!(script.contains("tynd__subcmd__updater__subcmd__keygen)"));
        assert!(script.contains("tynd__subcmd__plugin__subcmd__types)"));
    }
}
