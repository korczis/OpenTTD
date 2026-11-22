//! What the next commit can hold, and the facts git gives about the repository.
//!
//! The record listing is the one `tools/research/readme-scoreboard` and `tools/surfaces` use: what
//! git tracks plus new files it would not ignore, so a render and its `--check` agree before and
//! after a `git add`; inside a git hook (git exports `GIT_INDEX_FILE` to pre-commit) only the index.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Repo {
    pub root: PathBuf,
    pub files: BTreeSet<String>,
    pub dirs: BTreeSet<String>,
    /// `owner/name` from `GITHUB_REPOSITORY` or the `origin` remote, when it is on GitHub.
    pub slug: Option<String>,
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(root).args(args);
    if let Ok(index) = std::env::var("GIT_INDEX_FILE") {
        // pre-commit may export it relative to the hook's directory
        let abs = std::fs::canonicalize(&index).unwrap_or_else(|_| PathBuf::from(&index));
        cmd.env("GIT_INDEX_FILE", abs);
    }
    let out = cmd.output().map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `owner/name` of a GitHub remote URL (`git@github.com:o/n.git`, `https://github.com/o/n`).
pub fn slug_from_remote(url: &str) -> Option<String> {
    let at = url.find("github.com")? + "github.com".len();
    let rest = url[at..].trim_start_matches([':', '/']).trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

impl Repo {
    pub fn load(root: PathBuf) -> Result<Repo, String> {
        let mut args = vec!["ls-files", "-z", "--cached"];
        if std::env::var_os("GIT_INDEX_FILE").is_none() {
            args.extend(["--others", "--exclude-standard"]);
        }
        let listed = git(&root, &args)?;
        let files: BTreeSet<String> = listed
            .split('\0')
            .filter(|p| !p.is_empty() && root.join(p).exists())
            .map(str::to_string)
            .collect();
        let mut dirs = BTreeSet::new();
        for f in &files {
            let mut acc = String::new();
            for part in f.split('/').collect::<Vec<_>>().iter().rev().skip(1).rev() {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(part);
                dirs.insert(acc.clone());
            }
        }
        let slug = std::env::var("GITHUB_REPOSITORY").ok().filter(|s| !s.is_empty()).or_else(|| {
            git(&root, &["remote", "get-url", "origin"]).ok().and_then(|u| slug_from_remote(u.trim()))
        });
        Ok(Repo { root, files, dirs, slug })
    }

    /// The repository's GitHub Pages URL, derived like `tools/surfaces` derives it.
    pub fn pages_url(&self) -> Option<String> {
        let slug = self.slug.as_ref()?;
        let (owner, name) = slug.split_once('/')?;
        let owner = owner.to_lowercase();
        if name.to_lowercase() == format!("{owner}.github.io") {
            Some(format!("https://{owner}.github.io/"))
        } else {
            Some(format!("https://{owner}.github.io/{name}/"))
        }
    }

    /// Tracked files directly in `dir` whose name starts with `prefix` and ends with `suffix`.
    pub fn records(&self, dir: &str, prefix: &str, suffix: &str) -> Vec<String> {
        self.files
            .iter()
            .filter(|p| {
                p.strip_prefix(dir)
                    .and_then(|r| r.strip_prefix('/'))
                    .is_some_and(|name| !name.contains('/') && name.starts_with(prefix) && name.ends_with(suffix))
            })
            .cloned()
            .collect()
    }

    /// Whether a repository-relative path (a file or a directory, no anchor) is tracked.
    pub fn has(&self, path: &str) -> bool {
        let p = path.trim_end_matches('/');
        self.files.contains(p) || self.dirs.contains(p)
    }

    /// The revisions of `revs` that name a commit this repository holds.
    pub fn known_commits(&self, revs: &BTreeSet<String>) -> BTreeSet<String> {
        if revs.is_empty() {
            return BTreeSet::new();
        }
        let mut child = match Command::new("git")
            .current_dir(&self.root)
            .args(["cat-file", "--batch-check"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return BTreeSet::new(),
        };
        let input: String = revs.iter().map(|r| format!("{r}^{{commit}}\n")).collect();
        {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(input.as_bytes());
            }
        }
        let out = match child.wait_with_output() {
            Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
            Err(_) => return BTreeSet::new(),
        };
        revs.iter()
            .zip(out.lines())
            .filter(|(_, line)| line.contains(" commit "))
            .map(|(r, _)| r.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::slug_from_remote;

    #[test]
    fn slugs_of_github_remotes() {
        assert_eq!(slug_from_remote("git@github.com:korczis/OpenTTD.git").as_deref(), Some("korczis/OpenTTD"));
        assert_eq!(slug_from_remote("https://github.com/o/n").as_deref(), Some("o/n"));
        assert_eq!(slug_from_remote("https://gitlab.com/o/n"), None);
    }
}
