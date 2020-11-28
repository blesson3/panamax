use std::path::PathBuf;
use structopt::StructOpt;

#[macro_use]
extern crate quick_error;
#[macro_use]
extern crate router;

mod crates;
mod download;
mod git;
mod middleware;
mod mirror;
mod progress_bar;
mod rustup;
mod serve;

/// Mirror rustup and crates.io repositories, for offline Rust and cargo usage.
#[derive(Debug, StructOpt)]
enum Panamax {
    /// Create a new mirror directory.
    #[structopt(name = "init", alias = "new")]
    Init {
        /// Directory to store the mirror.
        #[structopt(parse(from_os_str))]
        path: PathBuf,
    },

    /// Update an existing mirror directory.
    #[structopt(name = "sync", alias = "run")]
    Sync {
        /// Mirror directory.
        #[structopt(parse(from_os_str))]
        path: PathBuf,
    },

    /// Serve an existing mirror directory.
    #[structopt(name = "serve")]
    Serve {
        /// Serve directory.
        #[structopt(parse(from_os_str))]
        path: PathBuf,
    },
}

fn main() {
    env_logger::init();
    let opt = Panamax::from_args();
    match opt {
        Panamax::Init { path } => mirror::init(&path),
        Panamax::Sync { path } => mirror::sync(&path),
        Panamax::Serve { path } => serve::serve(&path),
    }
    .unwrap();
}
