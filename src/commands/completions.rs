//! `vmanager completions` — emit a shell completion script on stdout.
//!
//! ```text
//! vmanager completions bash > /etc/bash_completion.d/vmanager
//! vmanager completions fish > ~/.config/fish/completions/vmanager.fish
//! vmanager completions zsh  > ~/.zfunc/_vmanager
//! ```

use clap::CommandFactory;
use clap_complete::Shell;

use crate::cli::Cli;
use crate::commands::Context;
use crate::error::Result;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Shell to generate completions for
    #[arg(value_enum)]
    shell: Shell,
}

pub fn run(args: &Args, _ctx: &Context) -> Result<()> {
    let mut command = Cli::command();
    let name = command.get_name().to_string();

    clap_complete::generate(args.shell, &mut command, name, &mut std::io::stdout());
    Ok(())
}
