use crate::error::{Error, Result};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Output},
};

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
        let mut cmd = Command::new("git");
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
    use super::windows_git_path;
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
