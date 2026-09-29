//! Launches the user's pager, in the same spirit as `git`.
//!
//! `PARQ_PAGER` is used verbatim. Otherwise `PAGER` (default `less`) is used,
//! and when that is `less` it is also told to chop long lines (`-S`) so wide
//! tables scroll sideways, and to pin the column header (`--header`) when the
//! installed `less` is new enough.

use std::env;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use crate::table::HEADER_LINES;

/// `less` gained `--header` in version 600.
const LESS_HEADER_SINCE: u32 = 600;

/// Starts the pager with a piped stdin, or returns `None` if paging is
/// disabled (empty or `cat`) or the pager cannot be started.
pub fn spawn() -> Option<Child> {
    let (cmdline, tune_less) = match env::var("PARQ_PAGER") {
        Ok(p) => (p, false),
        Err(_) => (env::var("PAGER").unwrap_or_else(|_| "less".into()), true),
    };
    let mut words = cmdline.split_whitespace();
    let program = words.next().filter(|p| *p != "cat")?;

    let mut cmd = Command::new(program);
    cmd.args(words).stdin(Stdio::piped());
    if tune_less && is_less(program) {
        if env::var_os("LESS").is_none() {
            // Quit if it fits on one screen; pass colours through.
            cmd.env("LESS", "FR");
        }
        cmd.arg("-S");
        if less_version(program).is_some_and(|v| v >= LESS_HEADER_SINCE) {
            cmd.arg(format!("--header={HEADER_LINES}"));
        }
    }
    let child = cmd.spawn().ok()?;
    ignore_interrupts();
    Some(child)
}

fn is_less(program: &str) -> bool {
    Path::new(program).file_stem().is_some_and(|s| s == "less")
}

fn less_version(program: &str) -> Option<u32> {
    let out = Command::new(program).arg("--version").output().ok()?;
    parse_less_version(&String::from_utf8_lossy(&out.stdout))
}

/// Parses the leading `less 668 (...)` line of `less --version`.
fn parse_less_version(text: &str) -> Option<u32> {
    let rest = text.lines().next()?.strip_prefix("less ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Ctrl-C in the pager is meant for the pager (e.g. to cancel a search). If
/// it also killed us, the shell would reclaim the terminal while the pager is
/// still running, so ignore it and let the pager decide when we're done.
fn ignore_interrupts() {
    #[cfg(unix)]
    // SAFETY: installing SIG_IGN has no preconditions.
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_less_version() {
        assert_eq!(
            parse_less_version("less 668 (POSIX regular expressions)\nCopyright"),
            Some(668)
        );
        assert_eq!(parse_less_version("less 487\n"), Some(487));
        assert_eq!(parse_less_version("more from util-linux 2.39"), None);
    }

    #[test]
    fn recognises_less_by_path() {
        assert!(is_less("less"));
        assert!(is_less("/usr/bin/less"));
        assert!(!is_less("more"));
        assert!(!is_less("lesspipe"));
    }
}
