//! End-to-end tests using local bare repos as remotes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const MAKEFILE: &str = "install-release:\n\t@echo installed >> \"$(INSTALL_COUNTER)\"\n\t@test -z \"$(FAIL_INSTALL)\"\n";

struct World {
    tmp: TempDir,
}

fn isolate_git(cmd: &mut Command) -> &mut Command {
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = isolate_git(Command::new("git").args(args).current_dir(dir))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn short(sha: &str) -> &str {
    &sha[..7]
}

impl World {
    fn new() -> Self {
        Self {
            tmp: TempDir::new().unwrap(),
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.tmp.path().join(rel)
    }

    fn home(&self) -> PathBuf {
        self.path("home")
    }

    fn counter(&self) -> PathBuf {
        self.path("install-count")
    }

    fn installs(&self) -> usize {
        fs::read_to_string(self.counter())
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    fn tools_toml(&self) -> String {
        fs::read_to_string(self.home().join("tools.toml")).unwrap_or_default()
    }

    /// A development clone at `dev/<name>` with a bare `origin` at `remotes/<name>.git`.
    fn tool_repo(&self, name: &str) -> PathBuf {
        let bare = self.path(&format!("remotes/{name}.git"));
        fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "--quiet", "--bare", "-b", "main"]);

        let dev = self.path(&format!("dev/{name}"));
        fs::create_dir_all(dev.join("src/nested")).unwrap();
        git(&dev, &["init", "--quiet", "-b", "main"]);
        fs::write(dev.join("Makefile"), MAKEFILE).unwrap();
        fs::write(dev.join("src/nested/lib.txt"), "v1\n").unwrap();
        git(&dev, &["add", "."]);
        git(&dev, &["commit", "--quiet", "-m", "initial"]);
        git(&dev, &["remote", "add", "origin", bare.to_str().unwrap()]);
        git(&dev, &["push", "--quiet", "-u", "origin", "main"]);
        dev
    }

    /// Push a new commit to `dev`'s remote main from another clone, as if from
    /// another machine, so `dev` falls behind. Returns the new SHA.
    fn push_commit(&self, dev: &Path, content: &str) -> String {
        let name = dev.file_name().unwrap().to_str().unwrap();
        let other = self.path(&format!("elsewhere/{name}"));
        if other.exists() {
            git(&other, &["pull", "--quiet", "--ff-only", "origin", "main"]);
        } else {
            let bare = self.path(&format!("remotes/{name}.git"));
            fs::create_dir_all(other.parent().unwrap()).unwrap();
            git(
                other.parent().unwrap(),
                &["clone", "--quiet", bare.to_str().unwrap(), name],
            );
        }
        fs::write(other.join("src/nested/lib.txt"), content).unwrap();
        git(&other, &["commit", "--quiet", "-am", content]);
        git(&other, &["push", "--quiet", "origin", "main"]);
        git(&other, &["rev-parse", "HEAD"])
    }

    fn head(&self, dev: &Path) -> String {
        git(dev, &["rev-parse", "HEAD"])
    }

    fn run_env(&self, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tsync"));
        isolate_git(&mut cmd)
            .args(args)
            .current_dir(cwd)
            .env("TOOLSYNC_HOME", self.home())
            .env("INSTALL_COUNTER", self.counter())
            .env_remove("FAIL_INSTALL");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.run_env(cwd, args, &[])
    }

    fn ok(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.run(cwd, args);
        assert!(out.status.success(), "{args:?} failed:\n{}", text(&out));
        text(&out)
    }

    fn fails(&self, args: &[&str]) -> String {
        let out = self.run(self.tmp.path(), args);
        assert!(!out.status.success(), "{args:?} unexpectedly succeeded");
        text(&out)
    }

    fn sync(&self) -> String {
        self.ok(self.tmp.path(), &["sync"])
    }

    fn status_line(&self, tool: &str) -> String {
        self.ok(self.tmp.path(), &["status"])
            .lines()
            .find(|l| l.starts_with(&format!("{tool} ")))
            .unwrap_or_else(|| panic!("no status row for {tool}"))
            .to_string()
    }
}

#[test]
fn add_from_subfolder_on_feature_branch_records_repo_without_touching_it() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    git(&dev, &["checkout", "--quiet", "-b", "feature"]);
    fs::write(dev.join("src/nested/lib.txt"), "wip\n").unwrap();
    let head_before = git(&dev, &["rev-parse", "HEAD"]);

    let out = w.ok(&dev.join("src/nested"), &["add"]);
    assert!(out.contains("Added my-tool"), "{out}");

    let list = w.tools_toml();
    assert!(list.contains("[tools.my-tool]"), "{list}");
    assert!(list.contains("my-tool.git"), "{list}");
    assert!(
        list.contains("install = \"make install-release\""),
        "{list}"
    );
    assert!(!list.contains("pull"), "{list}");
    assert!(!list.contains("feature"), "{list}");
    let root = fs::canonicalize(&dev).unwrap();
    assert!(
        list.contains(&format!("path = \"{}\"", root.display())),
        "{list}"
    );

    assert_eq!(git(&dev, &["branch", "--show-current"]), "feature");
    assert_eq!(git(&dev, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        fs::read_to_string(dev.join("src/nested/lib.txt")).unwrap(),
        "wip\n"
    );
    // Registration does not pull or install.
    assert_eq!(w.installs(), 0);
}

#[test]
fn add_merges_flags_into_existing_entries() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");

    w.ok(&dev, &["add", "--pull-only"]);
    assert!(!w.tools_toml().contains("install"));

    // A plain add keeps an existing entry's settings instead of re-detecting them.
    let out = w.ok(&dev, &["add"]);
    assert!(out.contains("already registered"), "{out}");
    assert!(!w.tools_toml().contains("install"));

    w.ok(&dev, &["add", "--install"]);
    assert!(
        w.tools_toml()
            .contains("install = \"make install-release\"")
    );

    // Plain add never silently disables install.
    let out = w.ok(&dev, &["add", "--yes"]);
    assert!(out.contains("already registered"), "{out}");
    assert!(
        w.tools_toml()
            .contains("install = \"make install-release\"")
    );

    w.ok(&dev, &["add", "--pull", "stax rs"]);
    let list = w.tools_toml();
    assert!(list.contains("pull = \"stax rs\""), "{list}");
    assert!(
        list.contains("install = \"make install-release\""),
        "{list}"
    );

    w.ok(&dev, &["add", "--install", "cargo install --path ."]);
    assert!(
        w.tools_toml()
            .contains("install = \"cargo install --path .\"")
    );

    w.ok(&dev, &["add", "--pull-only"]);
    let list = w.tools_toml();
    assert!(!list.contains("install"), "{list}");
    assert!(list.contains("pull = \"stax rs\""), "{list}");

    w.ok(&dev, &["add", "--pull", ""]);
    assert!(!w.tools_toml().contains("pull"));
}

#[test]
fn new_entry_without_install_target_defaults_to_pull_only() {
    let w = World::new();
    let dev = w.tool_repo("other-cli");
    fs::write(dev.join("Makefile"), "build:\n\tcargo build\n").unwrap();
    git(&dev, &["commit", "--quiet", "-am", "no install target"]);

    let out = w.ok(&dev, &["add", "-y"]);
    assert!(out.contains("none (pull only)"), "{out}");
    assert!(!w.tools_toml().contains("install"));
}

#[test]
fn output_has_no_color_codes_when_piped() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    let add = w.ok(&dev, &["add"]);
    let sync = w.sync();
    let status = w.ok(w.tmp.path(), &["status"]);
    for out in [add, sync, status] {
        assert!(!out.contains('\u{1b}'), "{out:?}");
    }
}

#[test]
fn add_fails_when_remote_has_no_main() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    let bare = w.path("remotes/trunk-only.git");
    fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "--quiet", "--bare", "-b", "trunk"]);
    git(&dev, &["remote", "add", "other", bare.to_str().unwrap()]);
    git(&dev, &["push", "--quiet", "other", "main:trunk"]);

    let out = w.run(&dev, &["add", "--remote", "other"]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("has no `main` branch"),
        "{}",
        text(&out)
    );
    assert!(w.tools_toml().is_empty());
}

#[test]
fn bare_command_runs_sync() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add", "--install"]);
    let v2 = w.push_commit(&dev, "v2");

    let out = w.ok(w.tmp.path(), &[]);
    assert!(out.contains("my-tool installed"), "{out}");
    // Pulled in place: no clone anywhere under the data directory.
    assert_eq!(w.head(&dev), v2);
    assert!(!w.home().join("repos").exists());
}

#[test]
fn pull_only_entries_never_install() {
    let w = World::new();
    let dev = w.tool_repo("other-cli");
    w.ok(&dev, &["add", "--pull-only"]);

    w.sync();
    assert_eq!(w.installs(), 0);
    let line = w.status_line("other-cli");
    assert!(line.contains("synced (pull only)"), "{line}");
    assert!(line.contains("—"), "{line}");
}

#[test]
fn custom_install_command_runs_in_checkout() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(
        &dev,
        &[
            "add",
            "--install",
            "echo custom-$(cat src/nested/lib.txt) >> \"$INSTALL_COUNTER\"",
        ],
    );
    w.sync();
    assert_eq!(fs::read_to_string(w.counter()).unwrap(), "custom-v1\n");
}

#[test]
fn second_sync_without_new_commit_does_not_reinstall() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add", "--install"]);

    w.sync();
    let out = w.sync();
    assert!(out.contains("up to date"), "{out}");
    assert_eq!(w.installs(), 1);

    w.push_commit(&dev, "v2");
    w.sync();
    assert_eq!(w.installs(), 2);
}

#[test]
fn failed_install_keeps_previous_sha_and_retries() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add", "--install"]);
    w.sync();
    let v1 = git(&dev, &["rev-parse", "main"]);

    let v2 = w.push_commit(&dev, "v2");
    let out = w.run_env(w.tmp.path(), &["sync"], &[("FAIL_INSTALL", "1")]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("install failed; will retry"),
        "{}",
        text(&out)
    );

    let line = w.status_line("my-tool");
    let cols: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(&cols[1..4], &[short(&v2), short(&v2), short(&v1)], "{line}");
    assert!(line.contains("install failed; will retry"), "{line}");

    // main has not moved, but the next run retries.
    w.sync();
    assert_eq!(w.installs(), 3);
    let line = w.status_line("my-tool");
    assert!(
        line.contains(&format!("{}  {}", short(&v2), short(&v2))),
        "{line}"
    );
    assert!(line.ends_with("installed"), "{line}");
}

#[test]
fn custom_pull_command_must_land_on_main_at_expected_commit() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add", "--install"]);
    w.sync();
    assert_eq!(w.installs(), 1);

    // Exits zero but leaves the old commit checked out.
    w.push_commit(&dev, "v2");
    w.ok(&dev, &["add", "--pull", "true"]);
    let out = w.fails(&["sync"]);
    assert!(out.contains("sync error"), "{out}");
    assert!(out.contains("expected main at"), "{out}");
    assert_eq!(w.installs(), 1);

    // Exits zero but switches branch.
    w.ok(&dev, &["add", "--pull", "git checkout -q -b other"]);
    let out = w.fails(&["sync"]);
    assert!(out.contains("on other after sync"), "{out}");
    assert_eq!(w.installs(), 1);
    assert!(w.status_line("my-tool").contains("sync error"));

    // The repo is now on `other`, so sync leaves it alone.
    let out = w.sync();
    assert!(out.contains("my-tool skipped: on branch other"), "{out}");

    // A command that really updates main is accepted.
    git(&dev, &["checkout", "--quiet", "main"]);
    w.ok(
        &dev,
        &["add", "--pull", "git pull -q --ff-only origin main"],
    );
    w.sync();
    assert_eq!(w.installs(), 2);
}

#[test]
fn repo_on_another_branch_is_skipped_untouched() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add", "--install"]);
    git(&dev, &["checkout", "--quiet", "-b", "feature"]);
    let before = w.head(&dev);
    w.push_commit(&dev, "v2");

    let out = w.sync();
    assert!(out.contains("my-tool skipped: on branch feature"), "{out}");
    assert_eq!(git(&dev, &["branch", "--show-current"]), "feature");
    assert_eq!(w.head(&dev), before);
    assert_eq!(w.installs(), 0);
    assert!(
        w.status_line("my-tool")
            .contains("skipped: on branch feature")
    );

    // Back on main, the next sync catches up.
    git(&dev, &["checkout", "--quiet", "main"]);
    let out = w.sync();
    assert!(out.contains("my-tool installed"), "{out}");
    assert_eq!(w.installs(), 1);
}

#[test]
fn uncommitted_changes_are_skipped_not_overwritten() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add"]);
    let before = w.head(&dev);

    let file = dev.join("src/nested/lib.txt");
    fs::write(&file, "local edit\n").unwrap();
    w.push_commit(&dev, "v2");

    let out = w.sync();
    assert!(
        out.contains("my-tool skipped: uncommitted changes"),
        "{out}"
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), "local edit\n");
    assert_eq!(w.head(&dev), before);
}

#[test]
fn unpushed_commits_on_main_are_skipped() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add"]);

    fs::write(dev.join("local.txt"), "x").unwrap();
    git(&dev, &["add", "."]);
    git(&dev, &["commit", "--quiet", "-m", "local"]);
    let before = w.head(&dev);

    let out = w.sync();
    assert!(
        out.contains("skipped: main has commits that aren't on origin"),
        "{out}"
    );
    assert_eq!(w.head(&dev), before);
}

#[test]
fn diverged_main_is_an_error() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add"]);

    fs::write(dev.join("local.txt"), "x").unwrap();
    git(&dev, &["add", "."]);
    git(&dev, &["commit", "--quiet", "-m", "local"]);
    w.push_commit(&dev, "v2");

    let out = w.fails(&["sync"]);
    assert!(out.contains("diverged"), "{out}");
}

#[test]
fn moved_repo_is_a_sync_error_until_re_added() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add", "--install"]);
    let moved = w.path("moved");
    fs::rename(&dev, &moved).unwrap();

    let out = w.fails(&["sync"]);
    assert!(out.contains("no Git repo at"), "{out}");

    w.ok(&moved, &["add"]);
    let out = w.sync();
    assert!(out.contains("my-tool installed"), "{out}");
}

#[test]
fn entry_without_path_asks_to_re_add() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add"]);
    let list = w.tools_toml();
    let list: String = list
        .lines()
        .filter(|l| !l.starts_with("path"))
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(w.home().join("tools.toml"), list).unwrap();

    let out = w.fails(&["sync"]);
    assert!(out.contains("run `tsync add` inside it"), "{out}");
}

#[test]
fn offline_sync_reports_refresh_failure_and_keeps_last_known_state() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    let other = w.tool_repo("other-cli");
    w.ok(&dev, &["add", "--install"]);
    w.ok(&other, &["add"]);
    w.sync();
    let v1 = git(&dev, &["rev-parse", "main"]);

    // Simulate losing the network for every remote.
    fs::rename(w.path("remotes"), w.path("remotes-offline")).unwrap();
    let out = w.fails(&["sync"]);
    assert!(out.contains("my-tool couldn't refresh"), "{out}");
    assert!(out.contains("other-cli couldn't refresh"), "{out}");

    let line = w.status_line("my-tool");
    let cols: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(&cols[1..4], &[short(&v1), short(&v1), short(&v1)], "{line}");
    assert!(line.contains("just now"), "{line}");
    assert!(
        line.contains("couldn't refresh; showing last known"),
        "{line}"
    );

    // A second offline status reports the failure once, not twice.
    let status = w.ok(w.tmp.path(), &["status"]);
    assert_eq!(
        status.matches("my-tool: couldn't reach").count(),
        1,
        "{status}"
    );
}

#[test]
fn sync_refuses_to_overlap_with_a_running_sync() {
    let w = World::new();
    fs::create_dir_all(w.home()).unwrap();
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(w.home().join("sync.lock"))
        .unwrap();
    lock.lock().unwrap();

    let out = w.fails(&["sync"]);
    assert!(out.contains("already running"), "{out}");
}

#[test]
fn remove_from_repo_forgets_entry_and_leaves_repo_alone() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add", "--install"]);
    w.sync();
    let head = w.head(&dev);

    let out = w.ok(&dev.join("src"), &["remove"]);
    assert!(out.contains("Removed my-tool"), "{out}");
    assert!(out.contains("does not uninstall"), "{out}");
    assert!(!w.tools_toml().contains("my-tool"));
    assert_eq!(w.head(&dev), head);
    assert!(dev.join("Makefile").exists());
    assert!(
        w.ok(w.tmp.path(), &["status"])
            .contains("No tools registered")
    );

    let out = w.fails(&["remove", "my-tool"]);
    assert!(out.contains("not registered"), "{out}");
}

#[test]
fn hand_removed_entry_shows_in_status_until_cleaned_up() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add"]);
    w.sync();

    fs::write(w.home().join("tools.toml"), "").unwrap();
    let status = w.ok(w.tmp.path(), &["status"]);
    assert!(status.contains("not in tool list"), "{status}");
    assert!(status.contains("tsync remove my-tool"), "{status}");

    w.ok(w.tmp.path(), &["remove", "my-tool"]);
    assert!(dev.exists());
    assert!(!w.ok(w.tmp.path(), &["status"]).contains("my-tool"));
}

/// Install command that marks `me` as started, then waits up to 3s for `other` to start.
fn rendezvous(dir: &Path, me: &str, other: &str) -> String {
    let d = dir.display();
    format!(
        "touch '{d}/{me}'; i=0; while [ ! -f '{d}/{other}' ]; do i=$((i+1)); [ $i -gt 30 ] && exit 1; sleep 0.1; done"
    )
}

#[test]
fn sync_runs_tools_in_parallel() {
    let w = World::new();
    let marks = w.path("marks");
    fs::create_dir_all(&marks).unwrap();
    let a = w.tool_repo("a-tool");
    let b = w.tool_repo("b-tool");
    w.ok(&a, &["add", "--install", &rendezvous(&marks, "a", "b")]);
    w.ok(&b, &["add", "--install", &rendezvous(&marks, "b", "a")]);

    // Each install waits for the other to start, so this only passes when they overlap.
    let out = w.sync();
    assert!(out.contains("a-tool installed"), "{out}");
    assert!(out.contains("b-tool installed"), "{out}");
}

#[test]
fn jobs_limits_concurrency() {
    let w = World::new();
    let marks = w.path("marks");
    fs::create_dir_all(&marks).unwrap();
    let a = w.tool_repo("a-tool");
    let b = w.tool_repo("b-tool");
    w.ok(&a, &["add", "--install", &rendezvous(&marks, "a", "b")]);
    w.ok(&b, &["add", "--install", &rendezvous(&marks, "b", "a")]);

    // One at a time: a-tool gives up waiting, then b-tool finds a-tool's mark.
    let out = w.fails(&["sync", "--jobs", "1"]);
    assert!(out.contains("a-tool install failed"), "{out}");
    assert!(out.contains("b-tool installed"), "{out}");
}

#[test]
fn status_reads_remote_main_live_and_offline_skips_it() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    let other = w.tool_repo("other-cli");
    w.ok(&dev, &["add", "--install"]);
    w.ok(&other, &["add", "--pull-only"]);
    w.sync();
    let v1 = git(&dev, &["rev-parse", "main"]);
    let v2 = w.push_commit(&dev, "v2");

    let offline = w.ok(w.tmp.path(), &["status", "--offline"]);
    let line = offline.lines().find(|l| l.starts_with("my-tool")).unwrap();
    assert!(
        line.contains(short(&v1)) && !line.contains(short(&v2)),
        "{line}"
    );
    assert!(line.ends_with("installed"), "{line}");

    let line = w.status_line("my-tool");
    let cols: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(&cols[1..4], &[short(&v2), short(&v1), short(&v1)], "{line}");
    assert!(line.ends_with("behind main"), "{line}");
    assert!(w.status_line("other-cli").contains("synced (pull only)"));

    // The live result is saved, so --offline now shows it too.
    let offline = w.ok(w.tmp.path(), &["status", "--offline"]);
    assert!(offline.contains("behind main"), "{offline}");
    assert_eq!(w.installs(), 1);
}

#[test]
fn config_shows_paths() {
    let w = World::new();
    let out = w.ok(w.tmp.path(), &["config", "--path"]);
    assert_eq!(
        out.trim(),
        w.home().join("tools.toml").display().to_string()
    );
    let out = w.ok(w.tmp.path(), &["config"]);
    assert!(out.contains("Tool list"), "{out}");
    assert!(out.contains("not created yet"), "{out}");
    assert!(
        out.contains(&w.home().join("logs").display().to_string()),
        "{out}"
    );
}

#[test]
fn pull_flag_skips_install_until_next_plain_sync() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add", "--install"]);
    let v2 = w.push_commit(&dev, "v2");

    for args in [&["--pull"][..], &["sync", "--pull"][..]] {
        let out = w.ok(w.tmp.path(), args);
        assert!(out.contains("(install skipped)"), "{out}");
        assert_eq!(w.head(&dev), v2);
        assert_eq!(w.installs(), 0);
    }
    assert!(w.status_line("my-tool").ends_with("install pending"));

    let out = w.ok(w.tmp.path(), &["-j", "1"]);
    assert!(out.contains("my-tool installed"), "{out}");
    assert_eq!(w.installs(), 1);
}

#[test]
fn status_reads_local_main_from_the_repo() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add", "--install"]);
    w.sync();
    let v1 = w.head(&dev);

    // Commit and push from the registered repo itself, without syncing.
    fs::write(dev.join("src/nested/lib.txt"), "v2\n").unwrap();
    git(&dev, &["commit", "--quiet", "-am", "v2"]);
    git(&dev, &["push", "--quiet", "origin", "main"]);
    let v2 = w.head(&dev);

    for args in [&["status"][..], &["status", "--offline"][..]] {
        let out = w.ok(w.tmp.path(), args);
        let line = out.lines().find(|l| l.starts_with("my-tool")).unwrap();
        let cols: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(cols[2], short(&v2), "{args:?}: {line}");
        assert_eq!(cols[3], short(&v1), "{args:?}: {line}");
        assert!(!line.contains("behind main"), "{args:?}: {line}");
    }
    assert!(w.status_line("my-tool").ends_with("install pending"));
}

#[test]
fn up_to_date_repo_is_not_pulled() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    let pulls = w.path("pulls");
    let cmd = format!(
        "echo pulled >> '{}'; git pull -q --ff-only origin main",
        pulls.display()
    );
    w.ok(&dev, &["add", "--install", "--pull", &cmd]);
    let count = || {
        fs::read_to_string(&pulls)
            .map(|s| s.lines().count())
            .unwrap_or(0)
    };

    // dev already has remote main: install, but no pull.
    let out = w.sync();
    assert!(out.contains("my-tool installed"), "{out}");
    assert_eq!(count(), 0);
    w.sync();
    assert_eq!(count(), 0);

    // A new commit on the remote is pulled once.
    let v2 = w.push_commit(&dev, "v2");
    w.sync();
    assert_eq!(count(), 1);
    assert_eq!(w.head(&dev), v2);
    w.sync();
    assert_eq!(count(), 1);
}

/// Each process launch can cost ~0.25s on machines with endpoint-security
/// agents, so an up-to-date tool must stay at a few git calls.
#[test]
fn up_to_date_sync_runs_few_git_processes() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.ok(&dev, &["add", "-y"]);
    w.sync();

    let real_git = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let shim_dir = w.path("shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let log = w.path("git-calls");
    let shim = shim_dir.join("git");
    fs::write(
        &shim,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real_git.trim()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        shim_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let out = w.run_env(w.tmp.path(), &["sync"], &[("PATH", &path)]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("up to date"), "{}", text(&out));
    let calls = fs::read_to_string(&log).unwrap();
    // ls-remote, remote -v, status.
    assert_eq!(calls.lines().count(), 3, "git calls:\n{calls}");
}
