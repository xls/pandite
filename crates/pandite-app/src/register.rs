//! `--register` and `--unregister`: file associations for the portable
//! Windows program.
//!
//! The Store package, the macOS bundle and the Linux desktop entry carry
//! their own associations. The portable `pandite.exe` has no installer, so
//! these two options write and remove the same per-user registry entries an
//! installer would write, under `HKEY_CURRENT_USER` (no administrator
//! rights):
//!
//! - a program identifier (`pandite.image`) with the open command and icon,
//! - an `Applications\<exe>` entry with the supported types,
//! - `OpenWithProgids` for every extension, which puts pandite in the
//!   "Open with" list of each type,
//! - the per-user default program identifier of every extension, which
//!   makes pandite open each type the user has not given to a program in
//!   Settings,
//! - a `Capabilities` entry listed in `RegisteredApplications`, which puts
//!   pandite in Settings, Default apps.
//!
//! A choice the user made in Settings wins over these entries and no
//! program can change it, so `--register` also opens that Settings page.
//! `--unregister` removes a per-user default only where it still names
//! pandite.
//!
//! The entries are built as the text of a registration (`.reg`) file, which
//! is pure and tested, and applied in one step by the system `reg.exe`.

use std::path::{Path, PathBuf};

/// What gets registered: the names, the program and its extensions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    /// Application name: the `RegisteredApplications` value and the
    /// `Software\<name>` key.
    pub name: String,
    /// The application name people see in "Open with" and Default apps.
    pub display: String,
    /// The program to start, absolute.
    pub exe: PathBuf,
    /// Extensions without the dot, lowercase.
    pub extensions: Vec<String>,
}

const CLASSES: &str = r"HKEY_CURRENT_USER\Software\Classes";

/// Escape a string for a quoted value in a registration file.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if c == '\\' || c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

impl Registration {
    /// The registration of this program for every extension the codecs
    /// decode.
    pub fn current() -> std::io::Result<Self> {
        let registry = pandite_codecs::default_registry();
        Ok(Self {
            name: "pandite".into(),
            display: "Pandite".into(),
            exe: std::env::current_exe()?,
            extensions: registry
                .decodable_extensions()
                .into_iter()
                .map(|e| e.to_string())
                .collect(),
        })
    }

    /// The program identifier that the extensions point to.
    pub fn prog_id(&self) -> String {
        format!("{}.image", self.name)
    }

    /// The program file name. The registration text is Windows data on
    /// every host, so the path splits at both `/` and `\`.
    fn exe_name(&self) -> String {
        let path = self.exe.to_string_lossy();
        let name = path.rsplit(['/', '\\']).next().unwrap_or_default();
        if name.is_empty() {
            "pandite.exe".into()
        } else {
            name.to_string()
        }
    }

    fn capabilities(&self) -> String {
        format!(r"Software\{}\Capabilities", self.name)
    }

    /// The registration file that adds every entry.
    pub fn register_file(&self) -> String {
        let exe = self.exe.to_string_lossy();
        let command = format!("\"{exe}\" \"%1\"");
        let prog = self.prog_id();
        let app = format!(r"{CLASSES}\Applications\{}", self.exe_name());
        let caps = format!(r"HKEY_CURRENT_USER\{}", self.capabilities());
        let mut t = String::from("Windows Registry Editor Version 5.00\r\n\r\n");
        let key = |t: &mut String, path: &str, values: &[(&str, String)]| {
            t.push_str(&format!("[{path}]\r\n"));
            for (name, value) in values {
                let name = if name.is_empty() {
                    "@".to_string()
                } else {
                    quoted(name)
                };
                t.push_str(&format!("{name}={value}\r\n"));
            }
            t.push_str("\r\n");
        };
        key(
            &mut t,
            &format!(r"{CLASSES}\{prog}"),
            &[("", quoted(&format!("{} image", self.display)))],
        );
        key(
            &mut t,
            &format!(r"{CLASSES}\{prog}\DefaultIcon"),
            &[("", quoted(&format!("\"{exe}\",0")))],
        );
        key(
            &mut t,
            &format!(r"{CLASSES}\{prog}\shell\open"),
            &[("FriendlyAppName", quoted(&self.display))],
        );
        key(
            &mut t,
            &format!(r"{CLASSES}\{prog}\shell\open\command"),
            &[("", quoted(&command))],
        );
        key(&mut t, &app, &[("FriendlyAppName", quoted(&self.display))]);
        key(
            &mut t,
            &format!(r"{app}\shell\open\command"),
            &[("", quoted(&command))],
        );
        let dotted: Vec<String> = self.extensions.iter().map(|e| format!(".{e}")).collect();
        let types: Vec<(&str, String)> = dotted.iter().map(|e| (e.as_str(), quoted(""))).collect();
        key(&mut t, &format!(r"{app}\SupportedTypes"), &types);
        for ext in &dotted {
            // The per-user default handler of the type.
            key(&mut t, &format!(r"{CLASSES}\{ext}"), &[("", quoted(&prog))]);
            // An empty value of type "none": only its name matters.
            key(
                &mut t,
                &format!(r"{CLASSES}\{ext}\OpenWithProgids"),
                &[(prog.as_str(), "hex(0):".to_string())],
            );
        }
        key(
            &mut t,
            &caps,
            &[
                ("ApplicationName", quoted(&self.display)),
                (
                    "ApplicationDescription",
                    quoted("A fast, offline image viewer"),
                ),
            ],
        );
        let assoc: Vec<(&str, String)> =
            dotted.iter().map(|e| (e.as_str(), quoted(&prog))).collect();
        key(&mut t, &format!(r"{caps}\FileAssociations"), &assoc);
        key(
            &mut t,
            r"HKEY_CURRENT_USER\Software\RegisteredApplications",
            &[(self.name.as_str(), quoted(&self.capabilities()))],
        );
        t
    }

    /// The registration file that removes every entry `register_file` adds.
    /// Entries of other programs under the same extensions stay. The
    /// per-user default is removed only for `owned_defaults`: the
    /// extensions whose default still names this program.
    pub fn unregister_file(&self, owned_defaults: &[String]) -> String {
        let prog = self.prog_id();
        let mut t = String::from("Windows Registry Editor Version 5.00\r\n\r\n");
        t.push_str(&format!("[-{CLASSES}\\{prog}]\r\n\r\n"));
        t.push_str(&format!(
            "[-{CLASSES}\\Applications\\{}]\r\n\r\n",
            self.exe_name()
        ));
        for ext in &self.extensions {
            if owned_defaults.contains(ext) {
                t.push_str(&format!("[{CLASSES}\\.{ext}]\r\n@=-\r\n\r\n"));
            }
            t.push_str(&format!(
                "[{CLASSES}\\.{ext}\\OpenWithProgids]\r\n{}=-\r\n\r\n",
                quoted(&prog)
            ));
        }
        t.push_str(&format!(
            "[-HKEY_CURRENT_USER\\{}]\r\n\r\n",
            self.capabilities()
        ));
        t.push_str(&format!(
            "[HKEY_CURRENT_USER\\Software\\RegisteredApplications]\r\n{}=-\r\n\r\n",
            quoted(&self.name)
        ));
        t
    }

    /// Write the entries to the registry of the current user.
    pub fn register(&self) -> Result<(), String> {
        import(&self.register_file())
    }

    /// Remove the entries from the registry of the current user.
    pub fn unregister(&self) -> Result<(), String> {
        import(&self.unregister_file(&self.owned_defaults()))
    }

    /// The extensions whose per-user default names this program now.
    fn owned_defaults(&self) -> Vec<String> {
        let prog = self.prog_id();
        self.extensions
            .iter()
            .filter(|ext| {
                std::process::Command::new(system_tool("reg.exe"))
                    .args(["query", &format!(r"HKCU\Software\Classes\.{ext}"), "/ve"])
                    .output()
                    .is_ok_and(|out| {
                        out.status.success()
                            && String::from_utf8_lossy(&out.stdout)
                                .lines()
                                .any(|l| l.trim_end().ends_with(&prog))
                    })
            })
            .cloned()
            .collect()
    }

    /// The address of the Settings page that lists this program's types.
    pub fn settings_uri(&self) -> String {
        format!("ms-settings:defaultapps?registeredAppUser={}", self.name)
    }
}

fn system_tool(name: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Path::new(&root).join("System32").join(name)
}

/// Apply registration file text with the system `reg.exe`.
fn import(text: &str) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("file registration is for the Windows program only".into());
    }
    // Registration files are UTF-16 with a byte order mark, so a program
    // path with any character survives.
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let file = std::env::temp_dir().join(format!("pandite-register-{}.reg", std::process::id()));
    std::fs::write(&file, bytes).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    let result = std::process::Command::new(system_tool("reg.exe"))
        .arg("import")
        .arg(&file)
        .output();
    let _ = std::fs::remove_file(&file);
    match result {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "reg.exe failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(e) => Err(format!("cannot run reg.exe: {e}")),
    }
}

/// Tell the shell that associations changed, so "Open with" lists and
/// icons update without a sign-out.
pub fn notify_shell() {
    #[cfg(windows)]
    ffi::assoc_changed();
}

/// Open the Settings page where the user makes pandite the default.
pub fn open_default_apps(reg: &Registration) {
    if cfg!(windows) {
        let _ = std::process::Command::new(system_tool("cmd.exe"))
            .args(["/C", "start", "", &reg.settings_uri()])
            .spawn();
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod ffi {
    const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
    const SHCNF_IDLIST: u32 = 0;

    #[link(name = "shell32")]
    extern "system" {
        fn SHChangeNotify(
            event: i32,
            flags: u32,
            item1: *const std::ffi::c_void,
            item2: *const std::ffi::c_void,
        );
    }

    pub fn assoc_changed() {
        // SAFETY: SHCNE_ASSOCCHANGED takes no items; both pointers are null
        // as the function documents for this event.
        unsafe {
            SHChangeNotify(
                SHCNE_ASSOCCHANGED,
                SHCNF_IDLIST,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Registration {
        Registration {
            name: "pandite".into(),
            display: "Pandite".into(),
            exe: PathBuf::from(r"C:\Tools\pan dite\pandite.exe"),
            extensions: vec!["png".into(), "arw".into()],
        }
    }

    #[test]
    fn register_file_lists_every_extension() {
        let reg = Registration::current().unwrap();
        assert!(reg.extensions.len() >= 30, "{}", reg.extensions.len());
        let text = reg.register_file();
        for ext in &reg.extensions {
            let open_with = format!(
                "[HKEY_CURRENT_USER\\Software\\Classes\\.{ext}\\OpenWithProgids]\r\n\"pandite.image\"=hex(0):"
            );
            assert!(text.contains(&open_with), "{ext} open-with");
            let default = format!(
                "[HKEY_CURRENT_USER\\Software\\Classes\\.{ext}]\r\n@=\"pandite.image\"\r\n"
            );
            assert!(text.contains(&default), "{ext} default");
            assert!(
                text.contains(&format!("\".{ext}\"=\"pandite.image\"")),
                "{ext} capability"
            );
            assert!(
                text.contains(&format!("\".{ext}\"=\"\"")),
                "{ext} supported type"
            );
        }
    }

    #[test]
    fn register_file_quotes_the_program_path() {
        let text = sample().register_file();
        assert!(text.starts_with("Windows Registry Editor Version 5.00\r\n"));
        // "C:\Tools\pan dite\pandite.exe" "%1", escaped for the file.
        assert!(text.contains(r#"@="\"C:\\Tools\\pan dite\\pandite.exe\" \"%1\"""#));
        assert!(text.contains(
            "[HKEY_CURRENT_USER\\Software\\RegisteredApplications]\r\n\"pandite\"=\"Software\\\\pandite\\\\Capabilities\""
        ));
        assert!(text.contains(r"\Applications\pandite.exe\shell\open\command]"));
        assert!(text.contains("\"FriendlyAppName\"=\"Pandite\""));
        assert!(text.contains("\"ApplicationName\"=\"Pandite\""));
        assert!(text.contains("@=\"Pandite image\""));
        assert!(text.is_ascii());
    }

    #[test]
    fn exe_name_splits_at_either_separator() {
        let name = |exe: &str| {
            Registration {
                exe: PathBuf::from(exe),
                ..sample()
            }
            .exe_name()
        };
        assert_eq!(name(r"C:\Tools\pan dite\pandite.exe"), "pandite.exe");
        assert_eq!(name("/opt/pandite/viewer.exe"), "viewer.exe");
        assert_eq!(name(r"D:/mixed\dir/p.exe"), "p.exe");
        assert_eq!(name(r"C:\Tools\"), "pandite.exe");
        assert_eq!(name(""), "pandite.exe");
    }

    #[test]
    fn unregister_file_removes_what_register_adds() {
        let reg = sample();
        let add = reg.register_file();
        let remove = reg.unregister_file(&reg.extensions);
        // A default that another program took over stays.
        let partial = reg.unregister_file(&["png".to_string()]);
        assert!(partial.contains("\\.png]\r\n@=-"));
        assert!(!partial.contains("\\.arw]\r\n@=-"));
        // Every key the registration writes is either deleted as a whole,
        // lies under a deleted key, or has its one value deleted.
        let deleted: Vec<&str> = remove
            .lines()
            .filter_map(|l| l.strip_prefix("[-")?.strip_suffix(']'))
            .collect();
        for key in add
            .lines()
            .filter_map(|l| l.strip_prefix('[')?.strip_suffix(']'))
        {
            let under_deleted = deleted
                .iter()
                .any(|d| key == *d || key.starts_with(&format!("{d}\\")));
            let value_deleted = remove.contains(&format!("[{key}]\r\n"));
            assert!(under_deleted || value_deleted, "{key} is left behind");
        }
        assert!(remove.contains("\"pandite.image\"=-"));
        assert!(remove
            .contains("[HKEY_CURRENT_USER\\Software\\RegisteredApplications]\r\n\"pandite\"=-"));
        // Extension keys themselves are never deleted: other programs use them.
        assert!(!remove.contains("[-HKEY_CURRENT_USER\\Software\\Classes\\."));
    }

    /// The real registry: register a throwaway name and extension, read the
    /// entries back, unregister and see them gone.
    #[cfg(windows)]
    #[test]
    fn register_round_trip_in_registry() {
        let name = format!("pandite-selftest-{}", std::process::id());
        let reg = Registration {
            name: name.clone(),
            display: "Pandite selftest".into(),
            exe: PathBuf::from(format!(r"C:\No Such Folder\{name}.exe")),
            extensions: vec![format!("{name}-ext")],
        };
        let query = |key: &str| {
            std::process::Command::new(system_tool("reg.exe"))
                .args(["query", key, "/s"])
                .output()
                .unwrap()
        };
        let command_key = format!(r"HKCU\Software\Classes\{name}.image\shell\open\command");
        let ext_key = format!(r"HKCU\Software\Classes\.{name}-ext\OpenWithProgids");
        let caps_key = format!(r"HKCU\Software\{name}\Capabilities");
        reg.register().unwrap();
        let out = query(&command_key);
        let listed = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(out.status.success(), "{listed}");
        assert!(
            listed.contains(&format!(r#""C:\No Such Folder\{name}.exe" "%1""#)),
            "{listed}"
        );
        assert!(String::from_utf8_lossy(&query(&ext_key).stdout).contains(&format!("{name}.image")));
        assert!(query(&caps_key).status.success());
        assert_eq!(reg.owned_defaults(), reg.extensions);
        reg.unregister().unwrap();
        assert!(!query(&command_key).status.success());
        assert!(!query(&caps_key).status.success());
        assert!(
            !String::from_utf8_lossy(&query(&ext_key).stdout).contains(&format!("{name}.image"))
        );
        // Remove the emptied throwaway keys as well.
        for key in [
            format!(r"HKCU\Software\Classes\.{name}-ext"),
            format!(r"HKCU\Software\{name}"),
        ] {
            let _ = std::process::Command::new(system_tool("reg.exe"))
                .args(["delete", &key, "/f"])
                .output();
        }
    }
}
