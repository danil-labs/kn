//! Stable machine queries using Git's vocabulary and output formats.
use crate::{
    error::{Error, Result},
    git::{Engine, utf8_path},
    workspace::{Location, Workspace, read_json},
};
use std::path::Path;

pub enum Query {
    IsInsideWorkTree,
    ShowToplevel,
    GitDir,
    GitCommonDir,
    Head,
}

pub fn rev_parse(path: &Path, query: Query) -> Result<Vec<u8>> {
    let ws = Workspace::open_read_only(path)?;
    let value = match query {
        Query::IsInsideWorkTree => "true".to_owned(),
        Query::ShowToplevel => utf8_path(&ws.git.root)?.to_owned(),
        Query::GitDir => utf8_path(&ws.git.dir)?.to_owned(),
        Query::GitCommonDir => utf8_path(&ws.git.common)?.to_owned(),
        Query::Head => ws.git.head()?,
    };
    Ok(format!("{value}\n").into_bytes())
}

pub fn status(path: &Path, nul: bool) -> Result<Vec<u8>> {
    let ws = Workspace::open_read_only(path)?;
    let mut args = vec!["status", "--porcelain=v1", "--untracked-files=all"];
    if nul {
        args.push("-z");
    }
    ws.git.run(&args)
}

pub fn worktree_list(path: &Path, nul: bool) -> Result<Vec<u8>> {
    let ws = Workspace::open_read_only(path)?;
    let root = if ws.config.session.is_some() {
        read_json::<Location>(&ws.git.common.join("location.json"))?.root
    } else {
        ws.git.root.clone()
    };
    let mut args = vec!["worktree", "list", "--porcelain"];
    if nul {
        args.push("-z");
    }
    let out = ws.git.run(&args)?;
    // With an external Git dir, Git reports that dir as the primary worktree.
    // Adapt only that path to kn's registered document root; retain Git's records.
    let separator = if nul { 0 } else { b'\n' };
    let end = out
        .iter()
        .position(|b| *b == separator)
        .filter(|_| out.starts_with(b"worktree "))
        .ok_or_else(|| Error::Git("Registro principal de worktree inválido.".into()))?;
    let path = utf8_path(&root)?;
    let path = if nul {
        path.to_owned()
    } else {
        quote_path(path)
    };
    let mut result = format!("worktree {path}").into_bytes();
    result.extend_from_slice(&out[end..]);
    Ok(result)
}

/// Git's C-style path quoting with core.quotePath=false, for the adapted root only.
fn quote_path(path: &str) -> String {
    if !path
        .chars()
        .any(|c| c.is_ascii_control() || c == '"' || c == '\\')
    {
        return path.to_owned();
    }
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '\x07' => out.push_str("\\a"),
            '\x08' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\x0b' => out.push_str("\\v"),
            '\x0c' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_ascii_control() => out.push_str(&format!("\\{:03o}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
