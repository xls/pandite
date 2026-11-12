//! Command line parsing.
//!
//! ```text
//! pandite [options] [--] [<image>]
//!   <image>              open this file (file association, drag onto the exe)
//!   --smoke <image>      open the image in a real window, render frames,
//!                        print "SMOKE OK" and exit (release self test)
//!   --smoke-stall        with --smoke: reads never finish; the smoke run
//!                        presses Escape on the loading screen and must exit
//!   --config-dir <dir>   keep settings in this folder instead of the
//!                        per-user configuration folder
//!   --help, -h           print usage
//!   --version, -V        print the version
//! ```
//!
//! Parsing is pure: relative paths are made absolute against the given
//! working directory without touching the file system.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Usage text printed by `--help`.
pub const USAGE: &str = "\
Usage: pandite [options] [--] [<image>]

Options:
  --smoke <image>     open the image, render a few frames, print SMOKE OK, exit
  --smoke-stall       with --smoke: simulate a file that never finishes loading
  --config-dir <dir>  keep settings in this folder
  -h, --help          show this help
  -V, --version       show the version
";

/// What the command line asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cli {
    /// The image to open, made absolute.
    pub path: Option<PathBuf>,
    /// Smoke test mode: open this image, render, report and exit.
    pub smoke: Option<PathBuf>,
    /// In smoke mode, make every read stall and escape from the loading
    /// screen.
    pub smoke_stall: bool,
    /// Settings folder override.
    pub config_dir: Option<PathBuf>,
    /// Print usage and exit.
    pub help: bool,
    /// Print the version and exit.
    pub version: bool,
    /// Extra positional arguments that were ignored (only the first image
    /// opens).
    pub ignored: Vec<PathBuf>,
}

/// A command line the program cannot run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliError(pub String);

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

/// Join `path` to `cwd` when it is relative. Lexical only: no disk access.
pub fn absolutize(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    // `std::path::absolute` is lexical on every platform; fall back to a
    // plain join if it refuses (an empty path, for example).
    let joined = cwd.join(path);
    std::path::absolute(&joined).unwrap_or(joined)
}

fn is_flag(arg: &OsStr) -> bool {
    arg.to_str()
        .is_some_and(|s| s.starts_with('-') && s.len() > 1)
}

/// Parse the arguments after the program name. `cwd` resolves relative
/// paths.
pub fn parse<I>(args: I, cwd: &Path) -> Result<Cli, CliError>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    let mut cli = Cli::default();
    let mut args = args.into_iter().map(Into::into);
    let mut positional: Vec<OsString> = Vec::new();
    let mut only_paths = false;
    while let Some(arg) = args.next() {
        if only_paths || !is_flag(&arg) {
            positional.push(arg);
            continue;
        }
        let text = arg.to_string_lossy().into_owned();
        // `--name=value` is accepted as well as `--name value`.
        let (name, inline) = match text.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(OsString::from(v))),
            _ => (text.clone(), None),
        };
        let mut value = |what: &str| -> Result<OsString, CliError> {
            match inline.clone().or_else(|| args.next()) {
                Some(v) if !v.is_empty() => Ok(v),
                _ => Err(CliError(format!("{what} needs a value"))),
            }
        };
        match name.as_str() {
            "--" => only_paths = true,
            "-h" | "--help" | "/?" => cli.help = true,
            "-V" | "--version" => cli.version = true,
            "--smoke" => {
                let v = value("--smoke")?;
                cli.smoke = Some(absolutize(Path::new(&v), cwd));
            }
            "--smoke-stall" => cli.smoke_stall = true,
            "--config-dir" => {
                let v = value("--config-dir")?;
                cli.config_dir = Some(absolutize(Path::new(&v), cwd));
            }
            // macOS adds a process serial number when Finder starts an app.
            s if s.starts_with("-psn_") => {}
            _ => return Err(CliError(format!("unknown option: {text}"))),
        }
    }
    let mut positional = positional.into_iter().filter(|p| !p.is_empty());
    cli.path = positional.next().map(|p| absolutize(Path::new(&p), cwd));
    cli.ignored = positional.map(|p| absolutize(Path::new(&p), cwd)).collect();
    if cli.smoke_stall && cli.smoke.is_none() {
        return Err(CliError("--smoke-stall needs --smoke <image>".into()));
    }
    Ok(cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\work")
        } else {
            PathBuf::from("/work")
        }
    }

    fn p(rel: &str) -> PathBuf {
        cwd().join(rel)
    }

    #[test]
    fn cli_parses_path_and_flags() {
        // Nothing: the empty state.
        assert_eq!(
            parse(Vec::<OsString>::new(), &cwd()).unwrap(),
            Cli::default()
        );

        // A relative path is made absolute against the working directory.
        let cli = parse(["photos/a.jpg"], &cwd()).unwrap();
        assert_eq!(cli.path, Some(p("photos/a.jpg")));
        assert!(cli.smoke.is_none() && !cli.help && !cli.version);

        // An absolute path stays as given, including non-ASCII names.
        let abs = if cfg!(windows) {
            "D:\\Bilder\\Gr\u{fc}\u{df}e\\caf\u{e9} 1.png"
        } else {
            "/srv/Bilder/Gr\u{fc}\u{df}e/caf\u{e9} 1.png"
        };
        let cli = parse([abs], &cwd()).unwrap();
        assert_eq!(cli.path.as_deref(), Some(Path::new(abs)));

        // Flags in both spellings, and only the first image opens.
        let cli = parse(
            [
                "--smoke",
                "fix/smoke.gif",
                "--config-dir=cfg",
                "x.png",
                "y.png",
            ],
            &cwd(),
        )
        .unwrap();
        assert_eq!(cli.smoke, Some(p("fix/smoke.gif")));
        assert_eq!(cli.config_dir, Some(p("cfg")));
        assert_eq!(cli.path, Some(p("x.png")));
        assert_eq!(cli.ignored, vec![p("y.png")]);

        let cli = parse(["--smoke", "s.png", "--smoke-stall"], &cwd()).unwrap();
        assert!(cli.smoke_stall);
        assert!(parse(["--smoke-stall"], &cwd()).is_err());

        assert!(parse(["-h"], &cwd()).unwrap().help);
        assert!(parse(["--version"], &cwd()).unwrap().version);
        assert!(parse(["-V"], &cwd()).unwrap().version);

        // After "--" a name that starts with a dash is a file.
        let cli = parse(["--", "-odd-.png"], &cwd()).unwrap();
        assert_eq!(cli.path, Some(p("-odd-.png")));

        // Errors name the problem.
        let err = parse(["--frobnicate"], &cwd()).unwrap_err();
        assert!(err.to_string().contains("--frobnicate"));
        assert!(parse(["--smoke"], &cwd()).is_err());
        assert!(parse(["--config-dir="], &cwd()).is_err());

        // macOS process serial numbers are ignored.
        let cli = parse(["-psn_0_12345", "a.png"], &cwd()).unwrap();
        assert_eq!(cli.path, Some(p("a.png")));

        // A single dash is a file name, not a flag.
        assert_eq!(parse(["-"], &cwd()).unwrap().path, Some(p("-")));
    }
}
