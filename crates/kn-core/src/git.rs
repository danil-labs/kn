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
    fn command(&self) -> Command {
        let mut cmd = Command::new("git");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                cmd.env_remove(key);
            }
        }
        let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
        cmd.current_dir(&self.root)
            .env("GIT_DIR", &self.dir)
            .env("GIT_WORK_TREE", &self.root)
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
                self.common.join("disabled-hooks").display()
            ));
        cmd
    }
    pub fn output(&self, args: &[&str]) -> Result<Output> {
        Ok(self.command().args(args).output()?)
    }
    pub fn run_os(&self, args: &[&OsStr]) -> Result<Vec<u8>> {
        checked(self.command().args(args).output()?)
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
