mod download;
mod source;

use std::{path::PathBuf, process::ExitCode};

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Download a Hugging Face file with Xet, without the Hub file cache"
)]
struct Args {
    /// Hugging Face blob or resolve URL (quote URLs containing ? or &)
    url: String,

    /// Output path (default: the filename from the URL)
    #[arg(short = 'O', long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Replace an existing output file after a successful download
    #[arg(short, long)]
    force: bool,

    /// Hide progress and success messages
    #[arg(short, long)]
    quiet: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match download::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xetget: {error:#}");
            if error.is::<download::Interrupted>() {
                ExitCode::from(130)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}
