//! End-to-end tests that run the compiled `claudesub` binary against an
//! isolated fake `$HOME`, exercising the real symlink + metadata behaviour.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

struct Harness {
    _dir: TempDir,
    home: PathBuf,
    config: PathBuf,
}

impl Harness {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("home");
        let config = dir.path().join("config");
        std::fs::create_dir_all(&home).unwrap();
        Harness {
            _dir: dir,
            home,
            config,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_claudesub"))
            .args(args)
            .env("CLAUDESUB_HOME", &self.home)
            .env("CLAUDESUB_CONFIG_DIR", &self.config)
            .output()
            .expect("failed to run claudesub")
    }

    fn active_link(&self) -> PathBuf {
        self.home.join(".claude-active")
    }

    fn link_target(&self) -> Option<PathBuf> {
        std::fs::read_link(self.active_link()).ok()
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn write_account(dir: &Path, email: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join(".claude.json"),
        format!(r#"{{"oauthAccount":{{"emailAddress":"{email}"}}}}"#),
    )
    .unwrap();
}

#[test]
fn first_add_becomes_active_and_creates_symlink() {
    let h = Harness::new();
    let out = h.run(&["add", "work"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let target = h.link_target().expect("symlink should exist");
    assert_eq!(target, h.home.join(".claude-work"));
    assert!(h.home.join(".claude-work").is_dir());

    let current = h.run(&["current"]);
    assert_eq!(stdout(&current).trim(), "work");
}

#[test]
fn switch_repoints_symlink() {
    let h = Harness::new();
    h.run(&["add", "work"]);
    h.run(&["add", "personal"]);

    assert_eq!(h.link_target().unwrap(), h.home.join(".claude-work"));
    let out = h.run(&["switch", "personal"]);
    assert!(out.status.success());
    assert_eq!(h.link_target().unwrap(), h.home.join(".claude-personal"));
}

#[test]
fn current_shows_detected_email() {
    let h = Harness::new();
    h.run(&["add", "work"]);
    write_account(&h.home.join(".claude-work"), "paul@nhost.io");
    let out = h.run(&["current"]);
    assert_eq!(stdout(&out).trim(), "work (paul@nhost.io)");
}

#[test]
fn rename_moves_directory_and_repoints() {
    let h = Harness::new();
    h.run(&["add", "work"]);
    write_account(&h.home.join(".claude-work"), "paul@nhost.io");

    let out = h.run(&["rename", "work", "client"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    assert!(!h.home.join(".claude-work").exists());
    assert!(h.home.join(".claude-client").is_dir());
    assert_eq!(h.link_target().unwrap(), h.home.join(".claude-client"));
    // Email survives the move because it lives in the moved directory.
    assert_eq!(stdout(&h.run(&["current"])).trim(), "client (paul@nhost.io)");
}

#[test]
fn remove_keeps_directory_and_switches_active() {
    let h = Harness::new();
    h.run(&["add", "work"]);
    h.run(&["add", "personal"]);
    // work is active; remove it.
    let out = h.run(&["remove", "work"]);
    assert!(out.status.success());
    // Directory is kept.
    assert!(h.home.join(".claude-work").is_dir());
    // Active fell back to the remaining profile.
    assert_eq!(h.link_target().unwrap(), h.home.join(".claude-personal"));
    assert_eq!(stdout(&h.run(&["current"])).trim(), "personal");
}

#[test]
fn remove_purge_deletes_directory() {
    let h = Harness::new();
    h.run(&["add", "work"]);
    h.run(&["add", "personal"]);
    let out = h.run(&["remove", "personal", "--purge"]);
    assert!(out.status.success());
    assert!(!h.home.join(".claude-personal").exists());
}

#[test]
fn list_json_is_valid_and_complete() {
    let h = Harness::new();
    h.run(&["add", "work"]);
    write_account(&h.home.join(".claude-work"), "paul@nhost.io");
    let out = h.run(&["list", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "work");
    assert_eq!(arr[0]["email"], "paul@nhost.io");
    assert_eq!(arr[0]["active"], true);
    assert_eq!(arr[0]["authenticated"], true);
}

#[test]
fn duplicate_add_fails() {
    let h = Harness::new();
    assert!(h.run(&["add", "work"]).status.success());
    let out = h.run(&["add", "work"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
}

#[test]
fn invalid_name_is_rejected() {
    let h = Harness::new();
    let out = h.run(&["add", "bad/name"]);
    assert!(!out.status.success());
}

#[test]
fn switch_unknown_profile_fails() {
    let h = Harness::new();
    let out = h.run(&["switch", "ghost"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not exist"));
}

#[test]
fn adopt_registers_existing_dir_in_place() {
    let h = Harness::new();
    // A pre-existing, signed-in config directory.
    write_account(&h.home.join(".claude-work"), "paul@nhost.io");
    let out = h.run(&["adopt", "work", "--path", h.home.join(".claude-work").to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // First adopted profile becomes active and is not copied.
    assert_eq!(h.link_target().unwrap(), h.home.join(".claude-work"));
    assert_eq!(stdout(&h.run(&["current"])).trim(), "work (paul@nhost.io)");
}

#[test]
fn adopt_default_derives_name_and_migrates_state() {
    let h = Harness::new();
    // Mimic the default install: config dir at ~/.claude, state at ~/.claude.json.
    std::fs::create_dir_all(h.home.join(".claude")).unwrap();
    std::fs::write(
        h.home.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"paul@nhost.io"}}"#,
    )
    .unwrap();

    let out = h.run(&["adopt", "--migrate-state"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    // Name derived from ~/.claude -> "default".
    assert_eq!(stdout(&h.run(&["current"])).trim(), "default (paul@nhost.io)");
    // State was copied into the profile dir; original left untouched.
    assert!(h.home.join(".claude/.claude.json").is_file());
    assert!(h.home.join(".claude.json").is_file());
}

#[test]
fn adopt_scan_finds_all_unmanaged_configs() {
    let h = Harness::new();
    write_account(&h.home.join(".claude-work"), "paul@nhost.io");
    write_account(&h.home.join(".claude-client"), "acme@client.com");
    std::fs::create_dir_all(h.home.join(".claudebar")).unwrap(); // not a claude config

    let out = h.run(&["adopt", "--scan"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let listed = stdout(&h.run(&["list"]));
    assert!(listed.contains("work"), "{listed}");
    assert!(listed.contains("client"), "{listed}");
    assert!(!listed.contains("claudebar"), "{listed}");

    // Re-scanning adopts nothing new.
    let again = h.run(&["adopt", "--scan"]);
    assert!(stdout(&again).contains("No un-managed"));
}

#[test]
fn symlink_is_source_of_truth_after_external_change() {
    let h = Harness::new();
    h.run(&["add", "work"]);
    h.run(&["add", "personal"]);
    // Simulate an external tool repointing the symlink directly.
    let link = h.active_link();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(h.home.join(".claude-personal"), &link).unwrap();
    // claudesub should report the profile the symlink points at, not its cache.
    assert_eq!(stdout(&h.run(&["current"])).trim(), "personal");
}
