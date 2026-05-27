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

fn select_branch(branches: &[String], prompt: &str) -> Result<String> {
    let index = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .items(branches)
        .default(0)
        .interact()?;
    Ok(branches[index].clone())
}

fn get_commits_between(base: &str, head: &str) -> Result<Vec<CommitInfo>> {
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
    let output = run_git(&["diff-tree", "--no-commit-id", "-r", "--name-only", hash])?;
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
    let target = select_branch(&branches, "Which branch do you want to PR into (target)?");
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
    let source = select_branch(&branches, "Which branch contains the changes (source)?");
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

    // ── 7. For each commit, ask how to handle it ──────────────
    let mut selections: Vec<CommitSelection> = Vec::new();

    for (i, commit) in commits.iter().enumerate() {
        println!("{}", "─".repeat(64).dimmed());
        println!(
            "  {} Commit {}/{}: {} {}",
            "●".bright_blue(),
            (i + 1).to_string().yellow(),
            commits.len().to_string().yellow(),
            commit.hash[..8].bright_green(),
            commit.subject.bold()
        );
        println!(
            "    {} {} <{}>  {} {}",
            "Author:".dimmed(),
            commit.author,
            commit.email.dimmed(),
            "Date:".dimmed(),
            commit.date[..10].dimmed()
        );

        // Show files (limit to 15 for display)
        let max_display = 15;
        if commit.files.is_empty() {
            println!("    {} (no files listed)", "Files:".dimmed());
        } else if commit.files.len() <= max_display {
            for f in &commit.files {
                println!("    {} {}", "📄".blue(), f.dimmed());
            }
        } else {
            for f in &commit.files[..max_display] {
                println!("    {} {}", "📄".blue(), f.dimmed());
            }
            println!(
                "    {} ... and {} more files",
                "📄".blue(),
                (commit.files.len() - max_display).to_string().dimmed()
            );
        }
        println!();

        let choice = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("How to include this commit?")
            .item("Skip")
            .item("Whole commit")
            .item("Select individual files")
            .default(1)
            .interact()
            .context("Failed to get user input, try running in a terminal")?;

        match choice {
            0 => {
                selections.push(CommitSelection::Skip);
                println!("  {} Skipped\n", "→".dimmed());
            }
            1 => {
                selections.push(CommitSelection::Whole);
                println!("  {} Will cherry-pick entire commit\n", "✓".green());
            }
            2 => {
                if commit.files.is_empty() {
                    println!("  {} No files to select, skipping.\n", "⚠".yellow());
                    selections.push(CommitSelection::Skip);
                } else {
                    let selected_indices = MultiSelect::with_theme(&ColorfulTheme::default())
                        .with_prompt("Select files (space to toggle, enter to confirm)")
                        .items(&commit.files.iter().map(|s| s.as_str()).collect::<Vec<_>>())
                        .interact()
                        .context("Failed to get user input, try running in a terminal")?;

                    let selected_files: Vec<String> = selected_indices
                        .iter()
                        .map(|&i| commit.files[i].clone())
                        .collect();

                    if selected_files.is_empty() {
                        println!("  {} No files selected, skipping.\n", "→".dimmed());
                        selections.push(CommitSelection::Skip);
                    } else {
                        println!(
                            "  {} Selected {} file(s)\n",
                            "✓".green(),
                            selected_files.len().to_string().yellow()
                        );
                        selections.push(CommitSelection::Files(selected_files));
                    }
                }
            }
            _ => unreachable!(),
        }
    }

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
