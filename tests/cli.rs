//! End-to-end tests for the MVP acceptance criteria, using local bare repos as remotes.

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

impl World {
    fn new() -> Self {
        let w = Self {
            tmp: TempDir::new().unwrap(),
        };
        let manifest = w.path("remotes/manifest.git");
        fs::create_dir_all(&manifest).unwrap();
        git(&manifest, &["init", "--quiet", "--bare", "-b", "main"]);
        w
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.tmp.path().join(rel)
    }

    fn manifest_url(&self) -> String {
        self.path("remotes/manifest.git").display().to_string()
    }

    fn counter(&self) -> PathBuf {
        self.path("install-count")
    }

    fn installs(&self) -> usize {
        fs::read_to_string(self.counter())
            .map(|s| s.lines().count())
            .unwrap_or(0)
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

    /// Push a new commit to remote main from the dev clone; returns its SHA.
    fn push_commit(&self, dev: &Path, content: &str) -> String {
        git(dev, &["checkout", "--quiet", "main"]);
        fs::write(dev.join("src/nested/lib.txt"), content).unwrap();
        git(dev, &["commit", "--quiet", "-am", content]);
        git(dev, &["push", "--quiet", "origin", "main"]);
        git(dev, &["rev-parse", "HEAD"])
    }

    fn home(&self, machine: &str) -> PathBuf {
        self.path(&format!("machines/{machine}"))
    }

    fn run(&self, machine: &str, cwd: &Path, args: &[&str]) -> Output {
        self.run_env(machine, cwd, args, &[])
    }

    fn run_env(&self, machine: &str, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_toolsync"));
        isolate_git(&mut cmd)
            .args(args)
            .current_dir(cwd)
            .env("TOOLSYNC_HOME", self.home(machine))
            .env("INSTALL_COUNTER", self.counter())
            .env_remove("FAIL_INSTALL");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    fn ok(&self, machine: &str, cwd: &Path, args: &[&str]) -> String {
        let out = self.run(machine, cwd, args);
        assert!(out.status.success(), "{args:?} failed:\n{}", text(&out));
        text(&out)
    }

    fn enroll(&self, machine: &str) {
        self.ok(machine, self.tmp.path(), &["init", &self.manifest_url()]);
    }

    /// Enroll `machine`, register `dev` with `add_args`, and publish.
    fn register(&self, machine: &str, dev: &Path, add_args: &[&str]) {
        self.enroll(machine);
        let mut args = vec!["add"];
        args.extend_from_slice(add_args);
        self.ok(machine, dev, &args);
        self.ok(machine, dev, &["publish"]);
    }

    fn status(&self, machine: &str) -> String {
        self.ok(machine, self.tmp.path(), &["status"])
    }

    fn status_line(&self, machine: &str, tool: &str) -> String {
        self.status(machine)
            .lines()
            .find(|l| l.starts_with(&format!("{tool} ")))
            .unwrap_or_else(|| panic!("no status row for {tool}"))
            .to_string()
    }

    fn manifest_text(&self, machine: &str) -> String {
        fs::read_to_string(self.home(machine).join("manifest/toolsync.toml")).unwrap()
    }
}

fn short(sha: &str) -> &str {
    &sha[..7]
}

#[test]
fn add_from_subfolder_on_feature_branch_registers_main_without_touching_checkout() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    git(&dev, &["checkout", "--quiet", "-b", "feature"]);
    fs::write(dev.join("src/nested/lib.txt"), "wip\n").unwrap();
    let head_before = git(&dev, &["rev-parse", "HEAD"]);

    w.enroll("a");
    let out = w.ok("a", &dev.join("src/nested"), &["add", "--install"]);
    assert!(out.contains("Added `my-tool`"), "{out}");

    let manifest = w.manifest_text("a");
    assert!(manifest.contains("[tools.my-tool]"), "{manifest}");
    assert!(manifest.contains("my-tool.git"), "{manifest}");
    assert!(manifest.contains("mode = \"install\""), "{manifest}");
    assert!(!manifest.contains("feature"), "{manifest}");
    assert!(!manifest.contains("dev/my-tool"), "{manifest}");

    assert_eq!(git(&dev, &["branch", "--show-current"]), "feature");
    assert_eq!(git(&dev, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        fs::read_to_string(dev.join("src/nested/lib.txt")).unwrap(),
        "wip\n"
    );
    // Registration does not publish.
    assert_eq!(
        git(&w.home("a").join("manifest"), &["status", "--porcelain"]),
        "?? toolsync.toml"
    );
}

#[test]
fn add_is_idempotent_and_never_silently_disables_install() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.enroll("a");

    w.ok("a", &dev, &["add"]);
    assert!(w.manifest_text("a").contains("mode = \"pull\""));

    let out = w.ok("a", &dev, &["add", "--install"]);
    assert!(out.contains("pull only -> make install-release"), "{out}");

    let out = w.ok("a", &dev, &["add"]);
    assert!(out.contains("already registered"), "{out}");
    assert!(w.manifest_text("a").contains("mode = \"install\""));

    w.ok("a", &dev, &["add", "--pull-only"]);
    assert!(w.manifest_text("a").contains("mode = \"pull\""));
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

    w.enroll("a");
    let out = w.run("a", &dev, &["add", "--remote", "other"]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("has no `main` branch"),
        "{}",
        text(&out)
    );
    assert!(!w.home("a").join("manifest/toolsync.toml").exists());
}

#[test]
fn second_machine_picks_up_published_entry() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.enroll("b"); // enrolled before the entry exists
    w.register("a", &dev, &["--install"]);

    let out = w.ok("b", w.tmp.path(), &["apply"]);
    assert!(out.contains("my-tool: installed"), "{out}");
    let main = git(&dev, &["rev-parse", "main"]);
    assert_eq!(
        git(&w.home("b").join("repos/my-tool"), &["rev-parse", "HEAD"]),
        main
    );
    assert!(w.status_line("b", "my-tool").contains("installed"));
}

#[test]
fn pull_only_entries_never_run_make() {
    let w = World::new();
    let dev = w.tool_repo("other-cli");
    w.register("a", &dev, &[]);

    w.ok("a", w.tmp.path(), &["apply"]);
    assert_eq!(w.installs(), 0);
    let line = w.status_line("a", "other-cli");
    assert!(line.contains("synced (pull only)"), "{line}");
    assert!(line.contains("—"), "{line}");
}

#[test]
fn second_apply_without_new_commit_does_not_reinstall() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.register("a", &dev, &["--install"]);

    w.ok("a", w.tmp.path(), &["apply"]);
    let out = w.ok("a", w.tmp.path(), &["apply"]);
    assert!(out.contains("up to date"), "{out}");
    assert_eq!(w.installs(), 1);

    w.push_commit(&dev, "v2");
    w.ok("a", w.tmp.path(), &["apply"]);
    assert_eq!(w.installs(), 2);
}

#[test]
fn failed_install_keeps_previous_sha_and_retries() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.register("a", &dev, &["--install"]);
    w.ok("a", w.tmp.path(), &["apply"]);
    let v1 = git(&dev, &["rev-parse", "main"]);

    let v2 = w.push_commit(&dev, "v2");
    let out = w.run_env("a", w.tmp.path(), &["apply"], &[("FAIL_INSTALL", "1")]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("install failed; will retry"),
        "{}",
        text(&out)
    );

    let line = w.status_line("a", "my-tool");
    let cols: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(&cols[1..4], &[short(&v2), short(&v2), short(&v1)], "{line}");
    assert!(line.contains("install failed; will retry"), "{line}");

    // main has not moved, but the next run retries.
    w.ok("a", w.tmp.path(), &["apply"]);
    assert_eq!(w.installs(), 3);
    let line = w.status_line("a", "my-tool");
    assert!(
        line.contains(&format!("{}  {}", short(&v2), short(&v2))),
        "{line}"
    );
    assert!(line.ends_with("installed"), "{line}");
}

#[test]
fn custom_sync_command_must_land_on_main_at_expected_commit() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.register("a", &dev, &["--install"]);
    w.ok("a", w.tmp.path(), &["apply"]);
    assert_eq!(w.installs(), 1);

    // Exits zero but leaves the old commit checked out.
    w.push_commit(&dev, "v2");
    fs::write(w.home("a").join("config.toml"), "sync_command = \"true\"\n").unwrap();
    let out = w.run("a", w.tmp.path(), &["apply"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("sync error"), "{}", text(&out));
    assert!(text(&out).contains("expected main at"), "{}", text(&out));
    assert_eq!(w.installs(), 1);

    // Exits zero but switches branch.
    fs::write(
        w.home("a").join("config.toml"),
        "[tools.my-tool]\nsync_command = \"git checkout -q -b other\"\n",
    )
    .unwrap();
    let out = w.run("a", w.tmp.path(), &["apply"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("on other after sync"), "{}", text(&out));
    assert_eq!(w.installs(), 1);
    assert!(w.status_line("a", "my-tool").contains("sync error"));

    // A command that really syncs main is accepted.
    let checkout = w.home("a").join("repos/my-tool");
    git(&checkout, &["checkout", "--quiet", "main"]);
    fs::write(
        w.home("a").join("config.toml"),
        "sync_command = \"git pull -q --ff-only origin main\"\n",
    )
    .unwrap();
    w.ok("a", w.tmp.path(), &["apply"]);
    assert_eq!(w.installs(), 2);
}

#[test]
fn dirty_managed_checkout_is_an_error_not_overwritten() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.register("a", &dev, &[]);
    w.ok("a", w.tmp.path(), &["apply"]);

    let file = w.home("a").join("repos/my-tool/src/nested/lib.txt");
    fs::write(&file, "local edit\n").unwrap();
    w.push_commit(&dev, "v2");

    let out = w.run("a", w.tmp.path(), &["apply"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("uncommitted changes"), "{}", text(&out));
    assert_eq!(fs::read_to_string(&file).unwrap(), "local edit\n");
}

#[test]
fn diverged_managed_checkout_is_an_error() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.register("a", &dev, &[]);
    w.ok("a", w.tmp.path(), &["apply"]);

    let checkout = w.home("a").join("repos/my-tool");
    fs::write(checkout.join("local.txt"), "x").unwrap();
    git(&checkout, &["add", "."]);
    git(&checkout, &["commit", "--quiet", "-m", "local"]);
    w.push_commit(&dev, "v2");

    let out = w.run("a", w.tmp.path(), &["apply"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("diverged"), "{}", text(&out));
}

#[test]
fn offline_run_reports_refresh_failure_and_keeps_last_known_state() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    let other = w.tool_repo("other-cli");
    w.register("a", &dev, &["--install"]);
    w.ok("a", &other, &["add"]);
    w.ok("a", &other, &["publish"]);
    w.ok("a", w.tmp.path(), &["apply"]);
    let v1 = git(&dev, &["rev-parse", "main"]);

    // Simulate losing the network for every remote.
    fs::rename(w.path("remotes"), w.path("remotes-offline")).unwrap();
    let out = w.run("a", w.tmp.path(), &["apply"]);
    assert!(!out.status.success());
    let out = text(&out);
    assert!(out.contains("manifest: couldn't refresh"), "{out}");
    assert!(out.contains("my-tool: couldn't refresh"), "{out}");
    assert!(out.contains("other-cli: couldn't refresh"), "{out}");

    let line = w.status_line("a", "my-tool");
    let cols: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(&cols[1..4], &[short(&v1), short(&v1), short(&v1)], "{line}");
    assert!(line.contains("just now"), "{line}");
    assert!(
        line.contains("couldn't refresh; showing last known"),
        "{line}"
    );
}

#[test]
fn apply_refuses_to_overlap_with_a_running_apply() {
    let w = World::new();
    w.enroll("a");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(w.home("a").join("apply.lock"))
        .unwrap();
    lock.lock().unwrap();

    let out = w.run("a", w.tmp.path(), &["apply"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("already running"), "{}", text(&out));
}

#[test]
fn commands_require_enrollment() {
    let w = World::new();
    let out = w.run("a", w.tmp.path(), &["status"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("toolsync init"), "{}", text(&out));
}

#[test]
fn remove_unregisters_and_purge_cleans_up_other_machines() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.register("a", &dev, &["--install"]);
    w.enroll("b");
    w.ok("b", w.tmp.path(), &["apply"]);
    assert_eq!(w.installs(), 1);

    // Remove from inside the repo, without naming it.
    let out = w.ok("a", &dev.join("src"), &["remove"]);
    assert!(out.contains("Removed `my-tool`"), "{out}");
    assert!(out.contains("does not uninstall"), "{out}");
    assert!(!w.manifest_text("a").contains("my-tool"));
    w.ok("a", &dev, &["publish"]);

    // Machine b stops syncing it but keeps its checkout until purged.
    w.push_commit(&dev, "v2");
    w.ok("b", w.tmp.path(), &["apply"]);
    assert_eq!(w.installs(), 1);
    let status = w.status("b");
    assert!(status.contains("not in manifest"), "{status}");
    assert!(
        status.contains("toolsync remove my-tool --purge"),
        "{status}"
    );
    assert!(w.home("b").join("repos/my-tool").exists());

    w.ok("b", w.tmp.path(), &["remove", "my-tool", "--purge"]);
    assert!(!w.home("b").join("repos/my-tool").exists());
    assert!(!w.status("b").contains("my-tool"));

    let out = w.run("b", w.tmp.path(), &["remove", "my-tool", "--purge"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("nothing to remove"), "{}", text(&out));
}

#[test]
fn remove_purge_refuses_local_work_and_changes_nothing() {
    let w = World::new();
    let dev = w.tool_repo("my-tool");
    w.register("a", &dev, &[]);
    w.ok("a", w.tmp.path(), &["apply"]);
    let checkout = w.home("a").join("repos/my-tool");

    fs::write(checkout.join("src/nested/lib.txt"), "edit\n").unwrap();
    let out = w.run("a", w.tmp.path(), &["remove", "my-tool", "--purge"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("uncommitted changes"), "{}", text(&out));

    git(&checkout, &["commit", "--quiet", "-am", "local"]);
    let out = w.run("a", w.tmp.path(), &["remove", "my-tool", "--purge"]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("aren't on its remote"),
        "{}",
        text(&out)
    );

    assert!(checkout.exists());
    assert!(w.manifest_text("a").contains("[tools.my-tool]"));
}

#[test]
fn remove_unknown_tool_fails() {
    let w = World::new();
    w.enroll("a");
    let out = w.run("a", w.tmp.path(), &["remove", "nope"]);
    assert!(!out.status.success());
    assert!(text(&out).contains("not in the manifest"), "{}", text(&out));
}
