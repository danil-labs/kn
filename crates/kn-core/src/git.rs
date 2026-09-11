use crate::error::{Error, Result};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::OnceLock,
};

const KN_GIT: &str = "KN_GIT";

/// Único punto que crea procesos. Terminus lanza kn sin consola: sin la bandera,
/// en Windows cada hijo abre y cierra una ventana de consola.
pub fn process(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// El git que usa kn, resuelto una vez por proceso: KN_GIT o el primero de PATH.
pub fn executable() -> Result<&'static Path> {
    static RESOLVED: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    RESOLVED
        .get_or_init(|| {
            resolve(
                std::env::var_os(KN_GIT),
                std::env::var_os("PATH"),
                std::env::consts::OS,
            )
        })
        .as_deref()
        .map_err(|message| Error::GitMissing(message.to_owned()))
}

fn resolve(
    kn_git: Option<OsString>,
    path: Option<OsString>,
    os: &str,
) -> std::result::Result<PathBuf, String> {
    if let Some(value) = kn_git {
        let candidate = PathBuf::from(value);
        return if !candidate.is_absolute() {
            Err(format!(
                "KN_GIT debe ser la ruta absoluta de un ejecutable git; recibió «{}».",
                candidate.display()
            ))
        } else if !candidate.is_file() {
            Err(format!(
                "KN_GIT apunta a «{}», que no existe o no es un archivo. Corrige la ruta o quita KN_GIT para usar el git de PATH.",
                candidate.display()
            ))
        } else {
            Ok(candidate)
        };
    }
    let program = if os == "windows" { "git.exe" } else { "git" };
    // Una entrada relativa se resolvería contra la carpeta de documentos.
    path.as_deref()
        .into_iter()
        .flat_map(std::env::split_paths)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
        .ok_or_else(|| missing_git(os))
}

fn is_executable(candidate: &Path) -> bool {
    let Ok(meta) = candidate.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

fn missing_git(os: &str) -> String {
    let remedy = match os {
        "macos" => {
            "Ejecuta `xcode-select --install` o define KN_GIT con la ruta absoluta de un ejecutable git."
        }
        "linux" => {
            "Instala git con el gestor de paquetes del sistema o define KN_GIT con la ruta absoluta de un ejecutable git."
        }
        "windows" => "Instala Git for Windows o define KN_GIT con la ruta absoluta de git.exe.",
        _ => "Instala git o define KN_GIT con la ruta absoluta de un ejecutable git.",
    };
    format!("No se encontró Git: KN_GIT no está definida y no hay git en PATH. {remedy}")
}

/// All engine Git calls cross this boundary. Git owns objects, refs, index and merges.
pub trait Engine {
    fn run(&self, args: &[&str]) -> Result<Vec<u8>>;
}
#[derive(Clone)]
pub struct Git {
    pub dir: PathBuf,
    pub root: PathBuf,
    pub common: PathBuf,
}
impl Git {
    fn command(&self) -> Result<Command> {
        let mut cmd = process(executable()?);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                cmd.env_remove(key);
            }
        }
        let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
        cmd.current_dir(&self.root)
            .env("GIT_DIR", git_path(&self.dir)?)
            .env("GIT_WORK_TREE", git_path(&self.root)?)
            .env("GIT_CONFIG_GLOBAL", null)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_LITERAL_PATHSPECS", "1")
            .env("GIT_AUTHOR_NAME", "kn")
            .env("GIT_AUTHOR_EMAIL", "kn@local")
            .env("GIT_COMMITTER_NAME", "kn")
            .env("GIT_COMMITTER_EMAIL", "kn@local")
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.quotePath=false",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.untrackedCache=false",
                "-c",
                "core.bare=false",
                "-c",
                "submodule.recurse=false",
                "-c",
                "core.attributesFile=",
            ])
            .arg("-c")
            .arg(format!(
                "core.hooksPath={}",
                git_path(&self.common.join("disabled-hooks"))?
            ));
        Ok(cmd)
    }
    pub fn output(&self, args: &[&str]) -> Result<Output> {
        Ok(self.command()?.args(args).output()?)
    }
    /// Como `run`, con configuración `-c` de esta llamada delante del subcomando.
    pub fn run_with(&self, config: &[String], args: &[&str]) -> Result<Vec<u8>> {
        checked(self.command()?.args(config).args(args).output()?)
    }
    pub fn run_os(&self, args: &[&OsStr]) -> Result<Vec<u8>> {
        checked(self.command()?.args(args).output()?)
    }
    pub fn text(&self, args: &[&str]) -> Result<String> {
        String::from_utf8(self.run(args)?)
            .map(|s| s.trim_end_matches('\n').to_owned())
            .map_err(|_| Error::Unsafe("Git devolvió texto que no es UTF-8.".into()))
    }
    pub fn head(&self) -> Result<String> {
        self.text(&["rev-parse", "--verify", "HEAD"])
    }
}
impl Engine for Git {
    fn run(&self, args: &[&str]) -> Result<Vec<u8>> {
        checked(self.output(args)?)
    }
}
fn checked(out: Output) -> Result<Vec<u8>> {
    if !out.status.success() {
        return Err(Error::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(out.stdout)
}
pub fn utf8_path(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| Error::Unsafe("La ruta debe ser UTF-8.".into()))
}
pub fn nul_paths(bytes: &[u8]) -> Result<Vec<String>> {
    bytes
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| {
            String::from_utf8(p.to_vec())
                .map_err(|_| Error::Unsafe("Hay un nombre que no es UTF-8.".into()))
        })
        .collect()
}
pub fn version(sha: &str) -> String {
    format!("v_{}", &sha[..12])
}

/// Rust canonical Windows paths use a verbatim prefix Git for Windows cannot
/// consistently consume. Adapt only the process boundary, retaining native paths
/// for all filesystem identity checks and preserving Unix backslashes literally.
pub fn git_path(path: &Path) -> Result<String> {
    let value = utf8_path(path)?;
    #[cfg(windows)]
    {
        Ok(windows_git_path(value))
    }
    #[cfg(not(windows))]
    {
        Ok(value.to_owned())
    }
}

#[cfg(any(windows, test))]
fn windows_git_path(value: &str) -> String {
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!("//{}", unc.replace('\\', "/"))
    } else {
        value
            .strip_prefix(r"\\?\")
            .unwrap_or(value)
            .replace('\\', "/")
    }
}

#[cfg(test)]
mod tests {
    use super::{missing_git, resolve, windows_git_path};
    use std::{
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
    };

    fn executable_file(path: &Path) {
        fs::write(path, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn path_of(dirs: &[&Path]) -> Option<OsString> {
        Some(std::env::join_paths(dirs).unwrap())
    }

    #[test]
    fn kn_git_pointing_to_a_file_wins_over_path() {
        let tmp = tempfile::tempdir().unwrap();
        let own = tmp.path().join("mi-git");
        executable_file(&own);
        let bin = tmp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        executable_file(&bin.join("git"));
        executable_file(&bin.join("git.exe"));
        let found = resolve(
            Some(own.clone().into()),
            path_of(&[&bin]),
            std::env::consts::OS,
        );
        assert_eq!(found, Ok(own));
    }

    #[test]
    fn kn_git_that_is_missing_relative_or_a_folder_does_not_fall_back_to_path() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        executable_file(&bin.join("git"));
        executable_file(&bin.join("git.exe"));
        let missing = tmp.path().join("no-existe").join("git");
        for (value, expected) in [
            (missing.clone(), missing.display().to_string()),
            (bin.clone(), bin.display().to_string()),
            (PathBuf::from("git"), "ruta absoluta".to_owned()),
            (PathBuf::new(), "ruta absoluta".to_owned()),
        ] {
            let err =
                resolve(Some(value.into()), path_of(&[&bin]), std::env::consts::OS).unwrap_err();
            assert!(err.starts_with("KN_GIT"), "{err}");
            assert!(err.contains(&expected), "{err}");
        }
    }

    #[test]
    fn path_lookup_uses_the_platform_program_name() {
        let tmp = tempfile::tempdir().unwrap();
        let unix = tmp.path().join("unix");
        let windows = tmp.path().join("windows");
        fs::create_dir(&unix).unwrap();
        fs::create_dir(&windows).unwrap();
        executable_file(&unix.join("git"));
        executable_file(&windows.join("git.exe"));
        let both = path_of(&[&unix, &windows]);
        assert_eq!(
            resolve(None, both.clone(), "windows"),
            Ok(windows.join("git.exe"))
        );
        assert_eq!(resolve(None, both, "linux"), Ok(unix.join("git")));
        assert!(resolve(None, path_of(&[&windows]), "macos").is_err());
    }

    #[test]
    fn missing_git_names_the_remedy_of_each_platform() {
        let empty = tempfile::tempdir().unwrap();
        for (os, remedy) in [
            ("macos", "xcode-select --install"),
            ("linux", "gestor de paquetes"),
            ("windows", "Git for Windows"),
            ("freebsd", "Instala git"),
        ] {
            let err = resolve(None, path_of(&[empty.path()]), os).unwrap_err();
            assert_eq!(err, missing_git(os));
            assert!(err.contains(remedy) && err.contains("KN_GIT"), "{err}");
        }
        assert_eq!(resolve(None, None, "linux"), Err(missing_git("linux")));
    }

    #[test]
    fn every_process_is_built_through_process() {
        let needle = concat!("Command", "::new(");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut pending = vec![manifest.join("src"), manifest.join("../kn/src")];
        let mut hits = Vec::new();
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = fs::read_to_string(&path).unwrap();
                    hits.extend(text.matches(needle).map(|_| path.clone()));
                }
            }
        }
        assert_eq!(
            hits.len(),
            1,
            "crea los procesos con git::process para que Windows no abra consolas: {hits:?}"
        );
    }

    #[test]
    fn windows_git_paths_keep_drive_and_unc_roots() {
        assert_eq!(
            windows_git_path(r"\\?\C:\Users\Persona\Documentos"),
            "C:/Users/Persona/Documentos"
        );
        assert_eq!(
            windows_git_path(r"\\?\UNC\server\share\docs"),
            "//server/share/docs"
        );
        assert_eq!(
            windows_git_path(r"C:\Users\Persona\políticas"),
            "C:/Users/Persona/políticas"
        );
    }
}
