use std::env;
use std::process::Command;

/// Compiles the checkout's branch into a debug build as `SONORA_BRANCH`, where the title bar's dev
/// badge shows it. The branch comes from `SONORA_BRANCH` set for the build, else from
/// `RBUILD_BRANCH`, which rbuild sets on the buildbox where the copy has no `.git`, else from git.
/// Empty values are skipped. A release build, a detached head or a missing git leave it unset.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SONORA_BRANCH");
    println!("cargo:rerun-if-env-changed=RBUILD_BRANCH");
    if env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_none() {
        return;
    }

    // HEAD is rewritten when another branch is checked out, in a worktree as in the main checkout.
    if let Some(head) = git(&["rev-parse", "--path-format=absolute", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    let branch = variable("SONORA_BRANCH")
        .or_else(|| variable("RBUILD_BRANCH"))
        .or_else(|| git(&["symbolic-ref", "--quiet", "--short", "HEAD"]));
    if let Some(branch) = branch {
        println!("cargo:rustc-env=SONORA_BRANCH={branch}");
    }
}

/// Reads an environment variable the build was given, or `None` when it is unset or empty.
fn variable(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

/// Runs git in the crate's folder and returns its trimmed output, or `None` when git is missing,
/// the source is not a checkout or the command fails.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(env::var_os("CARGO_MANIFEST_DIR")?)
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (output.status.success() && !text.is_empty()).then(|| text.to_owned())
}
