use super::*;

fn parse(arguments: &[&str]) -> Cli {
    Cli::try_parse_from(std::iter::once("tynd").chain(arguments.iter().copied()))
        .expect("arguments should parse")
}

fn test_args(arguments: &[&str]) -> TestArgs {
    match parse(arguments).command {
        Command::Test(args) => args,
        other => panic!("expected the test subcommand, got {other:?}"),
    }
}

#[test]
fn test_subcommand_has_sensible_defaults() {
    let args = test_args(&["test"]);
    assert_eq!(args.cwd, PathBuf::from("."));
    assert_eq!(args.timeout_ms, DEFAULT_TEST_TIMEOUT_MS);
    assert!(!args.update_snapshots);
    assert_eq!(args.args, [] as [String; 0]);
}

#[test]
fn test_subcommand_forwards_extra_arguments() {
    let args = test_args(&["test", "e2e/login.test.ts", "--bail", "-t", "opens"]);
    assert_eq!(args.args, ["e2e/login.test.ts", "--bail", "-t", "opens"]);
}

#[test]
fn test_subcommand_reads_own_flags_before_forwarded_arguments() {
    let args = test_args(&[
        "test",
        "--cwd",
        "app",
        "--timeout-ms",
        "1500",
        "--update-snapshots",
        "--",
        "--timeout",
        "9",
    ]);
    assert_eq!(args.cwd, PathBuf::from("app"));
    assert_eq!(args.timeout_ms, 1500);
    assert!(args.update_snapshots);
    assert_eq!(args.args, ["--timeout", "9"]);
}

fn parse_publish(arguments: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(
        ["tynd", "updater", "publish", "1.2.3"]
            .into_iter()
            .chain(arguments.iter().copied()),
    )
}

fn publish_args(arguments: &[&str]) -> PublishArgs {
    match parse_publish(arguments)
        .expect("arguments should parse")
        .command
    {
        Command::Updater(UpdaterCommand::Publish(args)) => args,
        other => panic!("expected the publish subcommand, got {other:?}"),
    }
}

#[test]
fn publish_channel_and_rollout_default_to_none() {
    let args = publish_args(&[]);
    assert_eq!(args.channel, None);
    assert_eq!(args.rollout, None);
}

#[test]
fn publish_accepts_channel_and_rollout() {
    let args = publish_args(&["--channel", "beta", "--rollout", "25"]);
    assert_eq!(args.channel.as_deref(), Some("beta"));
    assert_eq!(args.rollout, Some(25));
    assert_eq!(publish_args(&["--rollout", "0"]).rollout, Some(0));
    assert_eq!(publish_args(&["--rollout", "100"]).rollout, Some(100));
}

#[test]
fn publish_rejects_out_of_range_or_non_numeric_rollout() {
    for value in ["101", "-1", "256", "abc", ""] {
        assert!(
            parse_publish(&["--rollout", value]).is_err(),
            "rollout {value:?} should be rejected"
        );
    }
}

fn release_args(arguments: &[&str]) -> ReleaseArgs {
    match parse(&[&["updater", "release"], arguments].concat()).command {
        Command::Updater(UpdaterCommand::Release(args)) => args,
        other => panic!("expected the release subcommand, got {other:?}"),
    }
}

#[test]
fn release_accepts_every_publish_option() {
    let args = release_args(&[
        "1.2.0",
        "--channel",
        "beta",
        "--rollout",
        "10",
        "--max-diffs",
        "2",
        "--notes-file",
        "NOTES.md",
        "--bundle",
        "app",
        "--dry-run",
    ]);
    assert_eq!(args.publish.version, "1.2.0");
    assert_eq!(args.publish.channel.as_deref(), Some("beta"));
    assert_eq!(args.publish.rollout, Some(10));
    assert_eq!(args.publish.max_diffs, 2);
    assert_eq!(args.publish.notes_file, Some(PathBuf::from("NOTES.md")));
    assert_eq!(args.bundle.as_deref(), Some("app"));
    assert!(args.dry_run);
}

#[test]
fn release_shares_publish_defaults() {
    let release = release_args(&["1.2.0"]).publish;
    let publish = publish_args(&[]);
    assert_eq!(release.patch_strategy, publish.patch_strategy);
    assert_eq!(release.max_diffs, publish.max_diffs);
    assert_eq!(release.max_patches, publish.max_patches);
    assert_eq!(release.channel, None);
}

fn build_args(arguments: &[&str]) -> BuildArgs {
    match parse(&[&["build"], arguments].concat()).command {
        Command::Build(args) => args,
        other => panic!("expected the build subcommand, got {other:?}"),
    }
}

#[test]
fn build_disables_the_debug_protocol_by_default() {
    assert!(!build_args(&[]).debug_protocol);
}

#[test]
fn build_debug_protocol_flag_is_opt_in_and_not_the_verbose_alias() {
    let cli = parse(&["build", "--debug-protocol"]);
    assert!(
        !cli.verbose,
        "--debug-protocol must not turn on verbose logging"
    );
    assert!(build_args(&["--debug-protocol"]).debug_protocol);
}

fn rejects(arguments: &[&str]) -> bool {
    Cli::try_parse_from(std::iter::once("tynd").chain(arguments.iter().copied())).is_err()
}

#[test]
fn verbose_has_no_debug_alias() {
    assert!(parse(&["--verbose", "info"]).verbose);
    assert!(rejects(&["--debug", "info"]));
}

#[test]
fn grouped_commands_parse_as_nested_subcommands() {
    assert!(matches!(
        parse(&["config", "show", "--json"]).command,
        Command::Config(ConfigCommand::Show(ConfigShowArgs { json: true, .. }))
    ));
    assert!(matches!(
        parse(&["plugin", "types", "--check"]).command,
        Command::Plugin(PluginCommand::Types(PluginTypesArgs { check: true, .. }))
    ));
    assert!(matches!(
        parse(&["updater", "server", "--serve"]).command,
        Command::Updater(UpdaterCommand::Server(ServerArgs { serve: true, .. }))
    ));
    assert!(matches!(
        parse(&["updater", "sign", "app.tar.gz", "-o", "app.sig"]).command,
        Command::Updater(UpdaterCommand::Sign(SignArgs { out: Some(_), .. }))
    ));
}

#[test]
fn removed_commands_and_flags_are_rejected() {
    for arguments in [
        &["fix"][..],
        &["keygen"],
        &["sign", "file"],
        &["publish", "1.0.0"],
        &["release", "1.0.0"],
        &["config-show"],
        &["plugin-add", "@acme/plugin-x"],
        &["gen-types"],
        &["update-server"],
        &["updater-keygen"],
        &["add", "fs", "--skip-install"],
        &["build", "--outfile", "app"],
        &["profile", "--cpu", "--output", "cpu.svg"],
        &["updater", "publish", "1.0.0", "--access-key", "k"],
        &["updater", "publish", "1.0.0", "--secret-key", "k"],
        &["updater", "publish", "1.0.0", "--skip-diffs"],
    ] {
        assert!(rejects(arguments), "{arguments:?} should be rejected");
    }
}

#[test]
fn short_f_is_force_everywhere() {
    let Command::Create(create) = parse(&["create", "app", "-f"]).command else {
        panic!("expected create");
    };
    assert!(create.force);
    assert_eq!(create.framework, None);
    assert!(rejects(&["create", "app", "-f", "react"]));
    let Command::Create(create) = parse(&["create", "app", "--framework", "vue"]).command else {
        panic!("expected create");
    };
    assert_eq!(create.framework.as_deref(), Some("vue"));
}

#[test]
fn negative_flags_use_the_no_prefix() {
    let Command::Add(add) = parse(&["add", "fs", "--no-install"]).command else {
        panic!("expected add");
    };
    assert!(add.no_install);
    let Command::Remove(remove) = parse(&["remove", "@acme/plugin-x", "--no-uninstall"]).command
    else {
        panic!("expected remove");
    };
    assert!(remove.no_uninstall);
    assert!(rejects(&["remove"]), "remove needs at least one input");
    assert!(publish_args(&["--no-diffs"]).no_diffs);
}

#[test]
fn output_paths_are_out_with_short_o() {
    assert_eq!(
        build_args(&["-o", "dist/app"]).out,
        Some(PathBuf::from("dist/app"))
    );
    let Command::Profile(profile) = parse(&["profile", "--cpu", "-o", "cpu.svg"]).command else {
        panic!("expected profile");
    };
    assert_eq!(profile.out, Some(PathBuf::from("cpu.svg")));
}

#[test]
fn plugin_new_takes_the_target_path() {
    let Command::Plugin(PluginCommand::New(args)) =
        parse(&["plugin", "new", "plugins/clock", "--native"]).command
    else {
        panic!("expected plugin new");
    };
    assert_eq!(args.path, PathBuf::from("plugins/clock"));
    assert!(args.native);
    assert!(rejects(&["plugin", "new", "clock", "--cwd", "plugins"]));
}

fn keygen_args(arguments: &[&str]) -> KeygenArgs {
    match parse(&[&["updater", "keygen"], arguments].concat()).command {
        Command::Updater(UpdaterCommand::Keygen(args)) => args,
        other => panic!("expected updater keygen, got {other:?}"),
    }
}

#[test]
fn keygen_app_id_selects_the_app_scoped_key() {
    assert_eq!(keygen_args(&[]).app_id, None);
    assert_eq!(keygen_args(&["--app-id", "-f"]).app_id, Some(None));
    assert_eq!(
        keygen_args(&["--app-id", "com.example.app"]).app_id,
        Some(Some("com.example.app".to_owned()))
    );
    assert!(rejects(&["updater", "keygen", "--no-config-edit"]));
    assert!(keygen_args(&["--app-id", "--no-config-edit"]).no_config_edit);
}
