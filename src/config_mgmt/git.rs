use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

pub struct ConfigHistoryRepo {
    path: PathBuf,
}

pub struct HistoryEntry {
    pub rev: String,
    pub timestamp: DateTime<Utc>,
    pub author: String,
    pub message: String,
}

impl ConfigHistoryRepo {
    /// Open or lazily create the config history repo.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Ensure the git repo exists, creating it if needed.
    fn ensure_repo(&self) -> Result<()> {
        if self.path.join(".git").exists() {
            Ok(())
        } else {
            std::fs::create_dir_all(&self.path)
                .context("failed to create config history directory")?;

            // ponytail: shell out to git CLI — simpler than gix API
            let output = Command::new("git")
                .args(["init", "-q"])
                .current_dir(&self.path)
                .output()
                .context("failed to run git init")?;

            if !output.status.success() {
                anyhow::bail!(
                    "git init failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            // Set user.name and user.email for the repo
            Command::new("git")
                .args(["config", "user.name", "Quartermaster"])
                .current_dir(&self.path)
                .output()
                .context("failed to set git user.name")?;

            Command::new("git")
                .args(["config", "user.email", "quartermaster@localhost"])
                .current_dir(&self.path)
                .output()
                .context("failed to set git user.email")?;

            Ok(())
        }
    }

    /// Snapshot a file into the history repo and commit it.
    /// `rel_path` is relative to the repo root (e.g., "user/mods/SAIN/config/config.json").
    /// `content` is the file's content to snapshot.
    pub fn snapshot(
        &self,
        rel_path: &Path,
        content: &str,
        author: &str,
        message: &str,
    ) -> Result<()> {
        self.ensure_repo()?;

        let full_path = self.path.join(rel_path);

        // Write the file into the repo working tree
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full_path, content)?;

        // Stage the file
        let rel_path_str = rel_path.to_str().context("non-UTF-8 path")?;

        let output = Command::new("git")
            .args(["add", rel_path_str])
            .current_dir(&self.path)
            .output()
            .context("failed to run git add")?;

        if !output.status.success() {
            anyhow::bail!(
                "git add failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Commit with specified author
        let output = Command::new("git")
            .args([
                "-c",
                &format!("user.name={author}"),
                "-c",
                "user.email=quartermaster@localhost",
                "commit",
                "-m",
                message,
            ])
            .current_dir(&self.path)
            .output()
            .context("failed to run git commit")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore "nothing to commit" errors
            if !stderr.contains("nothing to commit") {
                anyhow::bail!("git commit failed: {stderr}");
            }
        }

        Ok(())
    }

    /// List commit history for a specific file.
    pub fn history(&self, rel_path: &Path) -> Result<Vec<HistoryEntry>> {
        if !self.path.join(".git").exists() {
            return Ok(Vec::new());
        }

        let rel_path_str = rel_path.to_str().context("non-UTF-8 path")?;

        // git log --format=%H%n%ct%n%an%n%s%x00 -- <file>
        let output = Command::new("git")
            .args(["log", "--format=%H%n%ct%n%an%n%s%x00", "--", rel_path_str])
            .current_dir(&self.path)
            .output()
            .context("failed to run git log")?;

        if !output.status.success() {
            anyhow::bail!(
                "git log failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut entries = Vec::new();

        for chunk in stdout.split('\0') {
            let chunk = chunk.trim();
            if chunk.is_empty() {
                continue;
            }

            let lines: Vec<&str> = chunk.lines().collect();
            if lines.len() < 4 {
                continue;
            }

            let rev = lines[0].to_string();
            let timestamp_secs = lines[1]
                .parse::<i64>()
                .context("failed to parse timestamp")?;
            let timestamp = DateTime::from_timestamp(timestamp_secs, 0).unwrap_or_default();
            let author = lines[2].to_string();
            let message = lines[3..].join("\n");

            entries.push(HistoryEntry {
                rev,
                timestamp,
                author,
                message,
            });
        }

        Ok(entries)
    }

    /// Get file content at a specific revision.
    pub fn content_at_rev(&self, rel_path: &Path, rev: &str) -> Result<String> {
        let rel_path_str = rel_path.to_str().context("non-UTF-8 path")?;

        let output = Command::new("git")
            .args(["show", &format!("{rev}:{rel_path_str}")])
            .current_dir(&self.path)
            .output()
            .context("failed to run git show")?;

        if !output.status.success() {
            anyhow::bail!(
                "git show failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let content =
            String::from_utf8(output.stdout).context("file content is not valid UTF-8")?;

        Ok(content)
    }

    /// Check if a file has ever been committed to the history repo.
    pub fn has_file(&self, rel_path: &Path) -> bool {
        if !self.path.join(".git").exists() {
            return false;
        }

        let rel_path_str = match rel_path.to_str() {
            Some(s) => s,
            None => return false,
        };

        // git cat-file -e HEAD:<path>
        let output = Command::new("git")
            .args(["cat-file", "-e", &format!("HEAD:{rel_path_str}")])
            .current_dir(&self.path)
            .output();

        match output {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    }
}
