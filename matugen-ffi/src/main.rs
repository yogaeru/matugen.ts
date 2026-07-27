use clap::Parser;
use color_eyre::Report;

use matugen_ffi::{helpers::setup_logging, util::arguments::Cli, State};

fn main() -> Result<(), Report> {
    color_eyre::install()?;

    let args = Cli::parse();

    setup_logging(&args)?;

    let state = State::new(args.clone())?;

    if args.show_source_colors.is_some_and(|x| x) {
        return Ok(());
    }

    state.run_in_term()?;

    Ok(())
}
