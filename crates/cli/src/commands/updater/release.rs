//! `tynd updater release`: validate, build, then publish.

use anyhow::Result;

use crate::args::ReleaseArgs;
use crate::commands::project;
use crate::output::write_line;

pub(in crate::commands) fn release(args: ReleaseArgs) -> Result<()> {
    let ReleaseArgs {
        publish: publish_args,
        bundle,
        dry_run,
    } = args;
    if dry_run {
        write_line(format_args!(
            "validate --ci
build{}
publish {}",
            bundle
                .as_ref()
                .map(|value| format!(" --bundle {value}"))
                .unwrap_or_default(),
            publish_args.version
        ))?;
        return Ok(());
    }
    project::validate(&crate::args::ValidateArgs {
        cwd: publish_args.cwd.clone(),
        json: false,
        ci: true,
    })?;
    project::build(&crate::args::BuildArgs {
        cwd: publish_args.cwd.clone(),
        out: None,
        bundle,
        debug_protocol: false,
    })?;
    super::publish(publish_args)?;
    Ok(())
}
