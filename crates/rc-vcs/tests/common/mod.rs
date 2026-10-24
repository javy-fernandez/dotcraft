//! Temporary repositories driven by the real `git`. Only the temp repos' local config is
//! written; options that a user's global config could change are pinned locally.

#![allow(dead_code)]

use rc_vcs::Repository;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct TempRepo {
    pub dir: tempfile::TempDir,
    pub repo: Repository,
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(dir).args(args).env("LC_ALL", "C").output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn configure(dir: &Path) {
    for (k, v) in [
        ("user.name", "Test User"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        ("core.autocrlf", "false"),
        ("core.hooksPath", "no-hooks"),
        ("pull.rebase", "false"),
        ("merge.conflictStyle", "merge"),
        ("diff.noprefix", "false"),
        ("init.defaultBranch", "main"),
    ] {
        git(dir, &["config", k, v]);
    }
}

impl TempRepo {
    pub async fn new() -> TempRepo {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        configure(dir.path());
        let repo = Repository::discover(dir.path()).await.unwrap();
        TempRepo { dir, repo }
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.repo.root().join(rel)
    }

    pub fn write(&self, rel: &str, text: &str) {
        let p = self.path(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    pub fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path(rel)).unwrap()
    }

    pub fn git(&self, args: &[&str]) -> String {
        git(self.repo.root(), args)
    }

    /// Writes files and commits them all.
    pub fn commit_files(&self, files: &[(&str, &str)], message: &str) {
        for (p, t) in files {
            self.write(p, t);
        }
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
    }
}

pub fn numbered(n: usize) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}
