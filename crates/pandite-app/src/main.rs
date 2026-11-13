//! pandite: a fast, offline image viewer.
//!
//! `pandite [<image>]` opens a window; see [`pandite_app::cli`] for the
//! options. On Windows the release binary uses the GUI subsystem, so no
//! console window opens; messages still reach a parent console or pipe.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use pandite_app::{cli, console, register, resident, shell};

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
    if cli.register || cli.unregister {
        std::process::exit(associations(cli.register));
    }
    if cli.quit {
        resident::ask(&resident::Request::Quit);
        return;
    }
    if shell::may_stay(&cli) {
        // A pandite that waits in the background shows the image at once.
        let taken = if cli.background {
            resident::ask(&resident::Request::Ping).is_some()
        } else {
            resident::hand_over(cli.path.as_deref())
        };
        if taken {
            return;
        }
    }
    let code = shell::run(cli);
    // Worker threads stuck in file reads are not joined.
    std::process::exit(code);
}

/// Run `--register` or `--unregister` and return the exit code.
fn associations(add: bool) -> i32 {
    if !cfg!(windows) {
        console::err("pandite: --register and --unregister are for the portable Windows program; the macOS app and the Linux desktop entry carry their own file associations");
        return 2;
    }
    let done = register::Registration::current()
        .map_err(|e| e.to_string())
        .and_then(|reg| {
            if add {
                reg.register()?;
            } else {
                reg.unregister()?;
            }
            Ok(reg)
        });
    match done {
        Ok(reg) => {
            register::notify_shell();
            if add {
                console::out(&format!(
                    "pandite: registered {} file types for this user. Types you gave to another program in Settings keep that choice; change them in Settings, Default apps.",
                    reg.extensions.len()
                ));
                register::open_default_apps(&reg);
            } else {
                console::out("pandite: file types unregistered for this user.");
            }
            0
        }
        Err(err) => {
            console::err(&format!("pandite: {err}"));
            1
        }
    }
}
