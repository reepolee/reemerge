use std::collections::HashMap;
use std::process::{Command, Stdio};
use anyhow::{anyhow, Context, Result};
use colored::*;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, MultiSelect, Select};

#[derive(Debug, Clone)]
struct CommitInfo {
    hash: String,
    author: String,
    email: String,
    date: String,
    subject: String,
    files: Vec<String>,
}

#[derive(Debug, Clone)]
enum CommitSelection {
    Skip,
    Whole,
    Files(Vec<String>),
}

fn run_git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("Failed to run git {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Git error: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn check_git_repo() -> Result<String> {
    let root = run_git(&["rev-parse", "--show-toplevel"])?;
    Ok(root)
}

fn has_uncommitted_changes() -> Result<bool> {
    let output = run_git(&["status", "--porcelain"])?;
    Ok(!output.is_empty())
}

fn get_branches() -> Result<Vec<String>> {
    let local = run_git(&["branch", "--format=%(refname:short)"])?;
    let remote = run_git(&["branch", "-r", "--format=%(refname:short)"])?;

    let mut seen = std::collections::HashSet::new();
    let mut branches = Vec::new();

    for line in local.lines() {
        let b = line.trim().to_string();
        if !b.is_empty() && seen.insert(b.clone()) {
            branches.push(b);
        }
    }
    for line in remote.lines() {
        let b = line.trim().to_string();
        if !b.is_empty() && seen.insert(b.clone()) {
            branches.push(b);
        }
    }

    Ok(branches)
}

fn select_branch(branches: &[String], prompt: &str, preferred: &str) -> Result<String> {
    let default_index = branches
        .iter()
        .position(|b| b == preferred || b == &format!("origin/{}", preferred))
        .unwrap_or(0);
    let index = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .items(branches)
        .default(default_index)
        .interact()?;
    Ok(branches[index].clone())
}

fn get_commits_between(base: &str, head: &str) -> Result<Vec<CommitInfo>> {
    if base == head {
        return Ok(Vec::new());
    }
    let range = format!("{}..{}", base, head);
    let output = run_git(&["log", "--format=%H|%an|%ae|%ai|%s", &range])?;

    if output.is_empty() {
        return Ok(Vec::new());
    }

    let mut commits = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(5, '|').collect();
        if parts.len() >= 5 {
            let hash = parts[0].to_string();
            let files = get_commit_files(&hash)?;
            commits.push(CommitInfo {
                hash,
                author: parts[1].to_string(),
                email: parts[2].to_string(),
                date: parts[3].to_string(),
                subject: parts[4].to_string(),
                files,
            });
        }
    }

    Ok(commits)
}

fn get_commit_files(hash: &str) -> Result<Vec<String>> {
    let output = run_git(&["log", "-1", "--name-only", "--format=", hash])?;
    Ok(output.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
}

fn apply_whole_commit(commit: &CommitInfo) -> Result<()> {
    let output = Command::new("git")
        .args(["cherry-pick", &commit.hash])
        .output()
        .context("Failed to run git cherry-pick")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);

        // If it failed due to being a merge commit, try with -m 1
        if stderr.contains("is a merge") {
            eprintln!("  {} Merge commit detected, retrying with -m 1", "ℹ".blue());
            // Abort the failed cherry-pick
            Command::new("git")
                .args(["cherry-pick", "--abort"])
                .output()
                .ok();

            let retry = Command::new("git")
                .args(["cherry-pick", "-m", "1", &commit.hash])
                .output()
                .context("Failed to run git cherry-pick with -m 1")?;

            if !retry.status.success() {
                let err2 = String::from_utf8_lossy(&retry.stderr);
                // Abort again
                Command::new("git")
                    .args(["cherry-pick", "--abort"])
                    .output()
                    .ok();
                return Err(anyhow!("Cherry-pick failed: {}", err2.trim()));
            }
            return Ok(());
        }

        // Abort the failed cherry-pick
        Command::new("git")
            .args(["cherry-pick", "--abort"])
            .output()
            .ok();

        return Err(anyhow!("Cherry-pick failed: {}", stderr.trim()));
    }

    Ok(())
}

fn apply_file_selection(commit: &CommitInfo, files: &[String]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }

    let has_parent = Command::new("git")
        .args(["rev-parse", "--verify", &format!("{}^", commit.hash)])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !has_parent {
        // Root commit: checkout files directly
        let mut args = vec!["checkout", &commit.hash, "--"];
        args.extend(files.iter().map(|s| s.as_str()));
        run_git(&args)?;
    } else {
        // Non-root commit: get diff for selected files
        let mut diff_cmd = Command::new("git");
        diff_cmd.args(["diff", "--binary", &format!("{}^", commit.hash), &commit.hash, "--"]);
        for f in files {
            diff_cmd.arg(f);
        }
        diff_cmd.stdout(Stdio::piped());

        let output = diff_cmd.output().context("Failed to run git diff")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("git diff failed: {}", stderr.trim()));
        }

        // Check for empty diff to avoid hanging git apply
        if output.stdout.is_empty() {
            return Err(anyhow!(
                "No diff output for selected files in commit {}. They may be binary or permissions-only changes.",
                &commit.hash[..8]
            ));
        }

        // Apply the patch by piping diff into git apply
        use std::io::Write;

        let mut apply_child = Command::new("git")
            .args(["apply", "--3way", "--index"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn git apply")?;

        // Write the diff to git apply's stdin in a block so stdin is dropped (closing the pipe)
        {
            let mut stdin = apply_child.stdin.take()
                .ok_or_else(|| anyhow!("Failed to capture git apply stdin"))?;
            stdin.write_all(&output.stdout)
                .context("Failed to write diff to git apply")?;
        }

        let apply_result = apply_child.wait_with_output()?;
        if !apply_result.status.success() {
            let stderr = String::from_utf8_lossy(&apply_result.stderr);
            return Err(anyhow!("Failed to apply patch: {}", stderr.trim()));
        }
    }

    // Create a commit with the original message
    let msg = format!("{} (cherry-picked from {})", commit.subject, commit.hash);
    run_git(&["commit", "-m", &msg, "--no-verify"])?;

    Ok(())
}

fn main() -> Result<()> {
    // Parse CLI args
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("reemerge {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let show_diff_flag = std::env::args().any(|a| a == "--diff");

    println!();
    println!("{}", "╔═══════════════════════════════════════════╗".bright_green());
    println!("{}", "║           PR Prep - Cherry Pick           ║".bright_green().bold());
    println!("{}", "║    Prepare hand-selected branches for PRs ║".bright_green());
    println!("{}", "╚═══════════════════════════════════════════╝".bright_green());
    println!();

    // ── 1. Check we're in a git repo ──────────────────────────
    let repo_root = check_git_repo()?;
    println!("  {} Repository: {}\n", "✓".green(), repo_root.yellow());

    // ── 2. Check for uncommitted changes ──────────────────────
    if has_uncommitted_changes()? {
        println!("  {} Warning: You have uncommitted changes.", "⚠".yellow());
        println!("           It's recommended to stash or commit them first.\n");
        if !Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Continue anyway?")
            .default(false)
            .interact()?
        {
            println!("  Exiting. Please commit or stash your changes first.");
            return Ok(());
        }
        println!();
    }

    // ── 3. Fetch branches ─────────────────────────────────────
    println!("  {} Fetching branches...", "⟳".blue());
    let branches = get_branches()?;
    if branches.is_empty() {
        return Err(anyhow!("No branches found in this repository"));
    }
    println!("  {} Found {} branches\n", "✓".green(), branches.len().to_string().yellow());

    // ── 4. Select target branch ───────────────────────────────
    let target = select_branch(&branches, "Which branch do you want to PR into (target)?", "main");
    let target = match target {
        Ok(b) => b,
        Err(e) => {
            // If selection fails (e.g. dialoguer issue), fall back to input
            eprintln!("  {} Selection failed: {}", "⚠".yellow(), e);
            Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Enter target branch name")
                .interact_text()?
        }
    };
    println!("  {} Target branch: {}", "✓".green(), target.cyan());
    println!();

    // ── 5. Select source branch ───────────────────────────────
    let source = select_branch(&branches, "Which branch contains the changes (source)?", "develop");
    let source = match source {
        Ok(b) => b,
        Err(e) => {
            eprintln!("  {} Selection failed: {}", "⚠".yellow(), e);
            Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Enter source branch name")
                .interact_text()?
        }
    };
    println!("  {} Source branch: {}", "✓".green(), source.cyan());
    println!();

    // ── 6. Get commits ────────────────────────────────────────
    println!("  {} Finding commits on {} not on {}...", "⟳".blue(), source.cyan(), target.cyan());
    let commits = get_commits_between(&target, &source)?;
    if commits.is_empty() {
        println!("  {} No new commits found.", "ℹ".blue());
        println!("  The source branch is already up to date with the target.");
        return Ok(());
    }

    println!("  {} Found {} commit(s)\n", "✓".green(), commits.len().to_string().yellow());

    // ── 7. Let user select commits to include ────────────────
    println!("{}", "─".repeat(64).dimmed());
    println!("  {} Commits on {} not on {}:", "📋".bold(), source.cyan(), target.cyan());
    println!();

    for (i, commit) in commits.iter().enumerate() {
        let file_count = commit.files.len();
        println!(
            "  {:>2}.  {}  {}  {} files:{}",
            (i + 1).to_string().yellow(),
            commit.hash[..8].bright_green(),
            commit.subject.bold(),
            file_count.to_string().dimmed(),
            if file_count == 0 { " (no files)".dimmed().to_string() } else { String::new() }
        );
        println!(
            "      {} {} <{}>",
            "Author:".dimmed(),
            commit.author,
            commit.email.dimmed()
        );
        println!(
            "      {} {}",
            "Date:".dimmed(),
            commit.date[..10].dimmed()
        );
    }
    println!();

    let commit_items: Vec<String> = commits
        .iter()
        .map(|c| format!("{}  {}", c.hash[..8].to_string(), c.subject))
        .collect();
    let item_refs: Vec<&str> = commit_items.iter().map(|s| s.as_str()).collect();

    let selected_indices = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select commits to include (space: toggle, ↑/↓: navigate, enter: confirm)")
        .items(&item_refs)
        .defaults(&vec![false; commits.len()])
        .interact()
        .context("Failed to get user input, try running in a terminal")?;

    println!();
    println!(
        "  {} Selected {} of {} commit(s)\n",
        "✓".green(),
        selected_indices.len().to_string().yellow(),
        commits.len().to_string().yellow()
    );

    // ── 7b. Ask about showing diffs ──────────────────────────
    let show_diff = if show_diff_flag {
        true
    } else if !selected_indices.is_empty() {
        Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt("Show diff preview for selected commits?")
            .default(false)
            .interact()?
    } else {
        false
    };
    if show_diff {
        println!();
    }

    // ── 7c. File-level selection for each selected commit ──
    let mut file_customizations: HashMap<usize, Vec<String>> = HashMap::new();

    for &commit_idx in &selected_indices {
        let commit = &commits[commit_idx];

        if commit.files.is_empty() {
            continue;
        }

        println!("{}", "─".repeat(64).dimmed());
        println!("  {} {}  {}", "📄".blue(), commit.hash[..8].bright_green(), commit.subject.bold());
        println!();

        // Show colored diff preview
        if show_diff {
            if let Ok(diff) = run_git(&["show", "--color=always", "--format=", &commit.hash]) {
                let diff = diff.trim();
                if !diff.is_empty() {
                    let line_count = diff.lines().count();
                    for line in diff.lines() {
                        println!("  │ {}", line);
                    }
                    if line_count > 0 {
                        println!();
                    }
                }
            }
        }

        let file_items: Vec<&str> = commit.files.iter().map(|s| s.as_str()).collect();
        let picked_files = MultiSelect::with_theme(&ColorfulTheme::default())
            .with_prompt("Select files (space: toggle, enter: confirm — deselect to exclude)")
            .items(&file_items)
            .defaults(&vec![true; file_items.len()])
            .interact()
            .context("Failed to get user input")?;

        if picked_files.len() < commit.files.len() && !picked_files.is_empty() {
            let selected_files: Vec<String> = picked_files.iter().map(|&fi| commit.files[fi].clone()).collect();
            file_customizations.insert(commit_idx, selected_files);
            println!("  {} Selected {} of {} file(s)\n", "✓".green(), picked_files.len().to_string().yellow(), commit.files.len().to_string().yellow());
        } else if picked_files.is_empty() {
            println!("  {} No files selected, skipping commit.\n", "→".dimmed());
            // Remove from selected_indices effectively — handled below in selection building
            let _ = file_customizations.insert(commit_idx, vec![]);
        } else {
            println!("  {} Including all files\n", "✓".green());
        }
    }

    // ── 7d. Build selections ─────────────────────────────────
    let selections: Vec<CommitSelection> = commits
        .iter()
        .enumerate()
        .map(|(i, _)| {
            if !selected_indices.contains(&i) {
                CommitSelection::Skip
            } else if let Some(files) = file_customizations.get(&i) {
                if files.is_empty() {
                    CommitSelection::Skip
                } else {
                    CommitSelection::Files(files.clone())
                }
            } else {
                CommitSelection::Whole
            }
        })
        .collect();

    // ── 8. Show summary and confirm ───────────────────────────
    println!("{}", "═".repeat(64).bright_green());
    println!("  {} Summary", "📋".bold());
    println!("{}", "═".repeat(64).bright_green());
    println!("  {}  Target: {}", "◉".white(), target.cyan());
    println!("  {}  Source: {}", "◉".white(), source.cyan());

    let whole_count = selections
        .iter()
        .filter(|s| matches!(s, CommitSelection::Whole))
        .count();
    let file_count = selections
        .iter()
        .filter(|s| matches!(s, CommitSelection::Files(_)))
        .count();
    let skip_count = selections
        .iter()
        .filter(|s| matches!(s, CommitSelection::Skip))
        .count();

    println!(
        "  {} {} whole {}, {} partial {}, {} skipped",
        "▶".white(),
        whole_count.to_string().yellow(),
        if whole_count == 1 { "commit" } else { "commits" },
        file_count.to_string().yellow(),
        if file_count == 1 { "commit" } else { "commits" },
        skip_count.to_string().dimmed()
    );
    println!();

    let total_active = whole_count + file_count;
    if total_active == 0 {
        println!("  {} No commits selected. Nothing to do.", "ℹ".blue());
        return Ok(());
    }

    // ── 9. Ask for new branch name ──────────────────────────
    let suggested_name = format!(
        "pr/{}",
        source
            .trim_start_matches("origin/")
            .trim_start_matches("remotes/origin/")
    );
    let new_branch: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Name for the new PR branch")
        .default(suggested_name)
        .interact_text()?;

    println!();
    if !Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "Create branch '{}' and apply {} selection(s)?",
            new_branch.cyan(),
            total_active.to_string().yellow()
        ))
        .default(true)
        .interact()?
    {
        println!("  Aborted.");
        return Ok(());
    }
    println!();

    // ── 10. Create new branch from target ────────────────────
    println!("  {} Creating branch from {}...", "▶".blue(), target.cyan());
    run_git(&["checkout", "-b", &new_branch, &target])
        .context("Failed to create new branch")?;
    println!(
        "  {} Branch '{}' created from {}\n",
        "✓".green(),
        new_branch.yellow(),
        target.cyan()
    );

    // ── 11. Apply selections in order ────────────────────────
    let mut applied = 0;
    for (_, (commit, selection)) in commits.iter().zip(selections.iter()).enumerate() {
        match selection {
            CommitSelection::Skip => continue,
            CommitSelection::Whole => {
                applied += 1;
                println!(
                    "  [{}/{}] Cherry-picking {} {}...",
                    applied.to_string().yellow(),
                    total_active.to_string().yellow(),
                    commit.hash[..8].bright_green(),
                    commit.subject.truncated(50)
                );
                if let Err(e) = apply_whole_commit(commit) {
                    eprintln!("  {} Failed: {}", "✗".red(), e);
                    eprintln!("  {} You may need to resolve conflicts manually.", "ℹ".blue());
                    eprintln!("  {} The branch '{}' has been created but the operation was interrupted.", "ℹ".blue(), new_branch.yellow());
                    eprintln!("  {} Run 'git status' to see the current state.", "ℹ".blue());
                    return Ok(());
                }
                println!("  {} Applied\n", "✓".green());
            }
            CommitSelection::Files(files) => {
                applied += 1;
                println!(
                    "  [{}/{}] Applying files from {} {}...",
                    applied.to_string().yellow(),
                    total_active.to_string().yellow(),
                    commit.hash[..8].bright_green(),
                    commit.subject.truncated(50)
                );
                if let Err(e) = apply_file_selection(commit, files) {
                    eprintln!("  {} Failed: {}", "✗".red(), e);
                    eprintln!("  {} The branch '{}' has been created but the operation was interrupted.", "ℹ".blue(), new_branch.yellow());
                    eprintln!("  {} Run 'git status' to see the current state.", "ℹ".blue());
                    return Ok(());
                }
                println!("  {} Applied\n", "✓".green());
            }
        }
    }

    println!(
        "  {} All {} selection(s) applied successfully!",
        "✓".bright_green(),
        total_active.to_string().yellow()
    );
    println!();

    // ── 12. Push ──────────────────────────────────────────────
    if Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Push '{}' to origin?", new_branch.cyan()))
        .default(true)
        .interact()?
    {
        println!("  {} Pushing to origin...", "▶".blue());
        match run_git(&["push", "origin", &new_branch]) {
            Ok(_) => println!(
                "  {} Branch '{}' pushed to origin",
                "✓".green(),
                new_branch.yellow()
            ),
            Err(e) => {
                eprintln!("  {} Push failed: {}", "✗".red(), e);
                eprintln!("  {} You can push manually later.", "ℹ".blue());
            }
        }
    } else {
        println!("  {} Skipped push. You can push manually:", "→".dimmed());
        println!("    git push origin {}", new_branch.yellow());
    }

    println!();
    println!(
        "  {} Branch '{}' is ready for a PR into {}!",
        "🎉".bright_green(),
        new_branch.bright_green(),
        target.cyan()
    );
    println!("  {}", "Happy coding! 🚀".dimmed());
    println!();

    Ok(())
}

/// Truncate string to a max width, adding "..." if needed.
trait Truncate {
    fn truncated(&self, max: usize) -> String;
}

impl Truncate for str {
    fn truncated(&self, max: usize) -> String {
        let chars: Vec<char> = self.chars().collect();
        if chars.len() <= max {
            self.to_string()
        } else {
            let truncated: String = chars[..max.saturating_sub(3)].iter().collect();
            format!("{}...", truncated)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    // ──────────────────────────────────────────────
    // Truncate trait tests (pure logic, no git)
    // ──────────────────────────────────────────────

    #[test]
    fn test_truncate_short_string() {
        let s = "hello";
        assert_eq!(s.truncated(10), "hello");
    }

    #[test]
    fn test_truncate_exact_fit() {
        let s = "1234567890";
        assert_eq!(s.truncated(10), "1234567890");
    }

    #[test]
    fn test_truncate_needs_ellipsis() {
        let s = "hello world this is long";
        let result = s.truncated(10);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn test_truncate_empty_string() {
        let s = "";
        assert_eq!(s.truncated(10), "");
    }

    #[test]
    fn test_truncate_unicode() {
        let s = "héllo wörld";
        let result = s.truncated(8);
        assert!(result.ends_with("..."));
        assert_eq!(result.chars().count(), 8);
    }

    #[test]
    fn test_truncate_max_zero() {
        let s = "test";
        // saturating_sub(3) gives 0 when max < 3, so we get empty string + "..."
        // but actual behavior: chars[..0] is empty, then format!("{}...", "") -> "..."
        let result = s.truncated(1);
        assert_eq!(result, "...");
    }

    // ──────────────────────────────────────────────
    // CommitSelection tests (pure logic, no git)
    // ──────────────────────────────────────────────

    #[test]
    fn test_commit_selection_skip() {
        let sel = CommitSelection::Skip;
        assert!(matches!(sel, CommitSelection::Skip));
    }

    #[test]
    fn test_commit_selection_whole() {
        let sel = CommitSelection::Whole;
        assert!(matches!(sel, CommitSelection::Whole));
    }

    #[test]
    fn test_commit_selection_files() {
        let sel = CommitSelection::Files(vec!["a.ts".to_string(), "b.ts".to_string()]);
        if let CommitSelection::Files(files) = &sel {
            assert_eq!(files.len(), 2);
            assert_eq!(files[0], "a.ts");
        } else {
            panic!("Expected Files variant");
        }
    }

    #[test]
    fn test_commit_selection_files_empty() {
        let sel = CommitSelection::Files(vec![]);
        if let CommitSelection::Files(files) = &sel {
            assert!(files.is_empty());
        } else {
            panic!("Expected Files variant");
        }
    }

    // ──────────────────────────────────────────────
    // CommitInfo creation test (pure logic)
    // ──────────────────────────────────────────────

    #[test]
    fn test_commit_info_creation() {
        let info = CommitInfo {
            hash: "abc123".to_string(),
            author: "Alice".to_string(),
            email: "alice@example.com".to_string(),
            date: "2026-01-15".to_string(),
            subject: "Fix bug".to_string(),
            files: vec!["src/main.rs".to_string()],
        };
        assert_eq!(info.hash, "abc123");
        assert_eq!(info.author, "Alice");
        assert_eq!(info.files.len(), 1);
    }

    // ──────────────────────────────────────────────
    // Git integration tests (need git installed)
    // ──────────────────────────────────────────────

    use std::sync::atomic::{AtomicU16, Ordering};

    static TEST_COUNTER: AtomicU16 = AtomicU16::new(0);

    /// Helper: create a temporary git repo and return its path.
    fn setup_temp_repo() -> std::path::PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("reemerge_test_{}_{}", std::process::id(), id));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("Failed to create temp dir");

        // Init git repo
        let mut cmd = Command::new("git");
        cmd.args(["init", "-b", "main"]).current_dir(&dir);
        let output = cmd.output().expect("Failed to init git repo");
        assert!(output.status.success(), "git init failed");

        // Set default git config for the repo
        let mut cmd = Command::new("git");
        cmd.args(["config", "user.name", "Test User"])
            .current_dir(&dir);
        cmd.output().expect("Failed to set user.name");

        let mut cmd = Command::new("git");
        cmd.args(["config", "user.email", "test@example.com"])
            .current_dir(&dir);
        cmd.output().expect("Failed to set user.email");

        dir
    }

    /// Helper: create a file and commit it in the given repo.
    fn create_and_commit(repo: &std::path::Path, filename: &str, contents: &str, msg: &str) {
        let file_path = repo.join(filename);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&file_path, contents).expect("Failed to write file");

        let mut cmd = Command::new("git");
        cmd.args(["add", filename]).current_dir(repo);
        cmd.output().expect("Failed to git add");

        let mut cmd = Command::new("git");
        cmd.args(["commit", "-m", msg]).current_dir(repo);
        let output = cmd.output().expect("Failed to git commit");
        assert!(output.status.success(), "git commit failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn test_check_git_repo_in_repo() {
        let repo = setup_temp_repo();
        std::env::set_current_dir(&repo).ok();
        let result = check_git_repo();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(!path.is_empty());
    }

    #[test]
    fn test_check_git_repo_outside_repo() {
        let outside = std::env::temp_dir().join(format!("reemerge_test_no_git_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&outside);
        std::env::set_current_dir(&outside).ok();
        let result = check_git_repo();
        assert!(result.is_err());
        std::fs::remove_dir_all(&outside).ok();
    }

    #[test]
    fn test_has_uncommitted_changes_clean() {
        let repo = setup_temp_repo();
        std::env::set_current_dir(&repo).ok();
        let result = has_uncommitted_changes();
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_has_uncommitted_changes_dirty() {
        let repo = setup_temp_repo();
        std::env::set_current_dir(&repo).ok();

        // Create a dirty file
        std::fs::write(repo.join("dirty.txt"), "dirty").ok();

        let result = has_uncommitted_changes();
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn test_run_git_basic() {
        let repo = setup_temp_repo();
        std::env::set_current_dir(&repo).ok();
        let result = run_git(&["rev-parse", "--git-dir"]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), ".git");
    }

    #[test]
    fn test_run_git_failure() {
        let result = run_git(&["this-command-does-not-exist"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_commits_between_same_ref() {
        let repo = setup_temp_repo();
        std::env::set_current_dir(&repo).ok();

        create_and_commit(&repo, "init.txt", "init\n", "Initial commit");

        // Same ref — no commits between
        let result = get_commits_between("HEAD", "HEAD");
        assert!(result.is_ok(), "get_commits_between failed: {:?}", result.err());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_get_commits_between_with_commits() {
        let repo = setup_temp_repo();
        std::env::set_current_dir(&repo).ok();

        // Create initial commit on main
        create_and_commit(&repo, "README.md", "# Project\n", "Initial commit");

        // Create a branch and add more commits
        let mut cmd = Command::new("git");
        cmd.args(["checkout", "-b", "feature"]).current_dir(&repo);
        cmd.output().ok();

        create_and_commit(&repo, "src/lib.rs", "fn hello() {}\n", "Add lib");
        create_and_commit(&repo, "src/main.rs", "fn main() {}\n", "Add main");

        // Switch back to main
        let mut cmd = Command::new("git");
        cmd.args(["checkout", "main"]).current_dir(&repo);
        cmd.output().ok();

        let result = get_commits_between("main", "feature");
        assert!(result.is_ok());
        let commits = result.unwrap();
        assert_eq!(commits.len(), 2);
        assert!(commits[0].subject.contains("Add") || commits[1].subject.contains("Add"));
        assert_eq!(commits[0].author, "Test User");
        assert_eq!(commits[0].email, "test@example.com");
    }

    #[test]
    fn test_get_commit_files() {
        let repo = setup_temp_repo();
        std::env::set_current_dir(&repo).ok();

        create_and_commit(&repo, "file_a.txt", "content a\n", "Add file A");
        create_and_commit(&repo, "file_b.txt", "content b\n", "Add file B");

        // Get the hash of the second commit
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo)
            .output()
            .expect("Failed to get HEAD");
        let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

        let result = get_commit_files(&hash);
        assert!(result.is_ok());
        let files = result.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], "file_b.txt");
    }

    #[test]
    fn test_get_branches_local_only() {
        let repo = setup_temp_repo();
        std::env::set_current_dir(&repo).ok();

        create_and_commit(&repo, "initial.txt", "init\n", "Initial");

        // Create another branch
        let mut cmd = Command::new("git");
        cmd.args(["branch", "develop"]).current_dir(&repo);
        cmd.output().ok();

        let result = get_branches();
        assert!(result.is_ok());
        let branches = result.unwrap();
        assert!(branches.contains(&"main".to_string()));
        assert!(branches.contains(&"develop".to_string()));
    }
}
