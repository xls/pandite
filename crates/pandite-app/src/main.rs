//! pandite: a fast, offline image viewer.
//!
//! `pandite [<image>]` opens a window; see [`pandite_app::cli`] for the
//! options. On Windows the release binary uses the GUI subsystem, so no
//! console window opens; messages still reach a parent console or pipe.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use pandite_app::{cli, console, shell};

fn main() {
    let cwd = std::env::current_dir().unwrap_or_default();
    let cli = match cli::parse(std::env::args_os().skip(1), &cwd) {
        Ok(cli) => cli,
        Err(err) => {
            console::err(&format!("pandite: {err}"));
            console::err(cli::USAGE);
            std::process::exit(2);
        }
    };
    if cli.help {
        console::out(cli::USAGE);
        return;
    }
    if cli.version {
        console::out(&format!("pandite {}", env!("CARGO_PKG_VERSION")));
        return;
    }
    let code = shell::run(cli);
    // Worker threads stuck in file reads are not joined.
    std::process::exit(code);
}
