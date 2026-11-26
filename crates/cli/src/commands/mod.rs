mod add;
mod completions;
mod plugins;
mod profile;
mod project;
mod report;
mod test;
mod updater;

use anyhow::Result;

use crate::args::{Cli, Command, ConfigCommand, PluginCommand, UpdaterCommand};
use crate::config::find_project_root;

/// Applies the global flags, then runs the selected command.
pub fn run(cli: Cli) -> Result<()> {
    apply_global_flags(&cli)?;
    dispatch(cli.command)
}

/// Logging setup plus the `TYND_ENV` / `NO_COLOR` process environment.
fn apply_global_flags(cli: &Cli) -> Result<()> {
    crate::output::configure(
        cli.verbose,
        cli.quiet,
        cli.log_format,
        cli.log_file.as_deref(),
    )?;
    crate::output::debug(format!(
        "Tynd CLI {} verbose logging enabled",
        crate::VERSION
    ));
    if let Some(environment) = &cli.env {
        std::env::set_var("TYND_ENV", environment);
    }
    if cli.no_color {
        std::env::set_var("NO_COLOR", "1");
    }
    Ok(())
}

/// Exhaustive dispatcher; every arm delegates behavior to its owning module.
fn dispatch(command: Command) -> Result<()> {
    match command {
        Command::Create(args) => project::create(&args),
        Command::Init(args) => project::init(&args),
        Command::Add(args) => add::add(args),
        Command::Remove(args) => add::remove(&args),
        Command::Dev(args) => project::dev(&args),
        Command::Start(args) => project::start(&args),
        Command::Test(args) => test::test(&test::request(args)?),
        Command::Build(args) => {
            project::warn_open_scopes(&args.cwd);
            project::build(&args)
        },
        Command::Icon(args) => project::icon(&args),
        Command::Clean(args) => project::clean(&args),
        Command::Upgrade(args) => project::upgrade(&args),
        Command::Validate(args) => project::validate(&args),
        Command::Doctor(args) => project::doctor(&args),
        Command::Info(args) => project::info(&args),
        Command::Size(args) => project::size(&args),
        Command::Explain(args) => project::explain(&args),
        Command::Config(ConfigCommand::Show(args)) => project::config_show(&args),
        Command::Plugin(command) => dispatch_plugin(command),
        Command::Updater(command) => dispatch_updater(command),
        Command::Report(args) => report::report(&report::ReportRequest {
            cwd: find_project_root(&args.cwd)?,
            out: args.out,
            json: args.json,
        })
        .map(|_| ()),
        Command::Profile(args) => profile::profile(&profile::ProfileRequest {
            cwd: find_project_root(&args.cwd)?,
            flamegraph: args.cpu,
            bun_memory: args.memory,
            ipc: args.ipc,
            out: args.out,
        }),
        Command::Completions(args) => completions::completions(args.shell),
    }
}

/// `tynd plugin <command>`.
fn dispatch_plugin(command: PluginCommand) -> Result<()> {
    match command {
        PluginCommand::New(args) => plugins::new(args),
        PluginCommand::Check(args) => plugins::plugin_check(&plugins::PluginCheckRequest {
            directory: args.cwd,
            json: args.json,
        })
        .map(|_| ()),
        PluginCommand::Dev(args) => plugins::plugin_dev(&plugins::PluginDevRequest {
            plugin: args.plugin,
            cwd: args.cwd,
            scratch: args.scratch,
            force: args.force,
            watch: !args.no_watch,
        }),
        PluginCommand::List(args) => plugins::list(args),
        PluginCommand::Types(args) => plugins::gen_types(&plugins::GenTypesRequest {
            cwd: args.cwd,
            plugin: args.plugin,
            dry_run: args.dry_run,
            check: args.check,
        }),
    }
}

/// `tynd updater <command>`.
fn dispatch_updater(command: UpdaterCommand) -> Result<()> {
    match command {
        UpdaterCommand::Keygen(args) => updater::keygen(args),
        UpdaterCommand::Sign(args) => updater::sign(&updater::SignRequest {
            file: args.file,
            key: args.key,
            out: args.out,
        })
        .map(|_| ()),
        UpdaterCommand::Publish(args) => updater::publish(args).map(|_| ()),
        UpdaterCommand::Release(args) => updater::release(args),
        UpdaterCommand::Server(args) => updater::update_server(&updater::UpdateServerRequest {
            cwd: find_project_root(&args.cwd)?,
            serve: args.serve,
            port: args.port,
            provider: args.deploy,
            public_url: args.manifest_url,
            public_key: args.public_key,
        }),
    }
}
