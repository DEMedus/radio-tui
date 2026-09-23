//! Startup check that offers to install mpv when it is missing.

use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn mpv_is_available() -> bool {
    Command::new("mpv")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn ensure_mpv() -> io::Result<()> {
    if mpv_is_available() {
        return Ok(());
    }

    println!("radio-tui needs mpv to play audio, and it isn't installed.");
    println!();

    let Some((program, args)) = install_command() else {
        println!("Install mpv with your package manager, then run radio-tui again.");
        println!("  https://mpv.io/installation/");
        println!();
        wait_to_continue()?;
        return Ok(());
    };

    let pretty = format!("{program} {}", args.join(" "));
    println!("Install it with:");
    println!("  {pretty}");
    println!();

    if !io::stdin().is_terminal() {
        println!("Skipping the install prompt because this isn't an interactive terminal.");
        return Ok(());
    }

    print!("Install now? [Y/n] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let answer = answer.trim();
    if !answer.is_empty()
        && !answer.eq_ignore_ascii_case("y")
        && !answer.eq_ignore_ascii_case("yes")
    {
        println!("Okay. Playback will not work until mpv is installed.");
        println!();
        return Ok(());
    }

    println!();
    let status = Command::new(program).args(args).status()?;
    println!();
    if mpv_is_available() {
        println!("mpv is installed. Starting radio-tui...");
        println!();
        return Ok(());
    }
    if !status.success() {
        println!("Install didn't finish. You can run this yourself later:");
        println!("  {pretty}");
    } else {
        println!("mpv still isn't on PATH. Open a new terminal and try radio-tui again.");
    }
    println!();
    wait_to_continue()?;
    Ok(())
}

fn wait_to_continue() -> io::Result<()> {
    if !io::stdin().is_terminal() {
        return Ok(());
    }
    print!("Press Enter to continue anyway. ");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(())
}

fn install_command() -> Option<(&'static str, &'static [&'static str])> {
    if command_exists("pacman") {
        Some(("sudo", &["pacman", "-S", "mpv"]))
    } else if command_exists("apt") {
        Some(("sudo", &["apt", "install", "mpv"]))
    } else if command_exists("dnf") {
        Some(("sudo", &["dnf", "install", "mpv"]))
    } else if command_exists("zypper") {
        Some(("sudo", &["zypper", "install", "mpv"]))
    } else if command_exists("apk") {
        Some(("sudo", &["apk", "add", "mpv"]))
    } else {
        None
    }
}

fn command_exists(name: &str) -> bool {
    env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|dir| {
            let candidate: PathBuf = dir.join(name);
            candidate.is_file()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_exists_finds_sh() {
        assert!(command_exists("sh"));
        assert!(!command_exists("radio-tui-not-a-real-binary"));
    }

    #[test]
    fn install_command_is_detected_on_this_machine() {
        assert!(install_command().is_some());
    }
}
