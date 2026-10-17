//! Shell completion generation for zsh, bash, fish, elvish, and powershell.

use anyhow::{bail, Context, Result};
use clap::CommandFactory;
use clap_complete::{generate, Shell};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::cli::Cli;

/// Parse a shell name string into a `clap_complete::Shell` enum.
pub fn parse_shell(s: &str) -> Result<Shell> {
    match s.trim().to_ascii_lowercase().as_str() {
        "zsh" => Ok(Shell::Zsh),
        "bash" => Ok(Shell::Bash),
        "fish" => Ok(Shell::Fish),
        "elvish" => Ok(Shell::Elvish),
        "powershell" | "pwsh" => Ok(Shell::PowerShell),
        other => bail!(
            "Unsupported shell '{other}'. Supported shells: zsh, bash, fish, elvish, powershell"
        ),
    }
}

/// Auto-detect the user's active shell from the $SHELL environment variable.
pub fn detect_active_shell() -> Option<Shell> {
    if let Ok(shell_path) = std::env::var("SHELL") {
        let path = Path::new(&shell_path);
        if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
            let lower = file_name.to_ascii_lowercase();
            if lower.contains("zsh") {
                return Some(Shell::Zsh);
            } else if lower.contains("bash") {
                return Some(Shell::Bash);
            } else if lower.contains("fish") {
                return Some(Shell::Fish);
            } else if lower.contains("elvish") {
                return Some(Shell::Elvish);
            } else if lower.contains("pwsh") || lower.contains("powershell") {
                return Some(Shell::PowerShell);
            }
        }
    }
    None
}

/// Generate shell completion script bytes for the given shell, including norbctl aliases.
pub fn generate_shell_script(shell: Shell) -> Vec<u8> {
    let mut cmd = Cli::command();
    let mut buf = Vec::new();
    generate(shell, &mut cmd, "norb", &mut buf);

    // Also register completion for norbctl so that both CLI entrypoints benefit
    match shell {
        Shell::Zsh => {
            buf.extend_from_slice(
                b"\n# Enable completions for norbctl as well\nif (( $+functions[compdef] )); then\n    compdef _norb norbctl 2>/dev/null || true\nfi\n",
            );
        }
        Shell::Bash => {
            buf.extend_from_slice(
                b"\n# Enable completions for norbctl as well\nif type complete >/dev/null 2>&1; then\n    complete -F _norb -o bashdefault -o default norbctl 2>/dev/null || true\nfi\n",
            );
        }
        Shell::Fish => {
            buf.extend_from_slice(
                b"\n# Enable completions for norbctl as well\ncomplete -c norbctl -w norb\n",
            );
        }
        _ => {}
    }

    buf
}

/// Install zsh completion to `~/.zfunc/_norb` or `~/.zsh/completion/_norb`.
pub fn install_zsh(home: &Path) -> Result<PathBuf> {
    let script = generate_shell_script(Shell::Zsh);
    let zsh_comp_dir = home.join(".zsh").join("completion");
    let zfunc_dir = home.join(".zfunc");

    let target_dir = if zsh_comp_dir.exists() {
        zsh_comp_dir
    } else {
        zfunc_dir
    };

    fs::create_dir_all(&target_dir)
        .with_context(|| format!("failed to create directory {}", target_dir.display()))?;

    let dest = target_dir.join("_norb");
    fs::write(&dest, &script)
        .with_context(|| format!("failed to write {}", dest.display()))?;

    // If both directories exist, also populate the other for completeness
    let other_dir = if target_dir.starts_with(home.join(".zsh")) {
        home.join(".zfunc")
    } else {
        home.join(".zsh").join("completion")
    };
    if other_dir.exists() {
        let _ = fs::write(other_dir.join("_norb"), &script);
    }

    Ok(dest)
}

/// Print user activation instructions for zsh.
fn print_zsh_instructions(file: &Path) {
    println!("✓ Installed zsh completion to {}", file.display());
    println!();
    println!("To activate completions, add the following to your ~/.zshrc:");
    println!();
    if file.starts_with(dirs::home_dir().unwrap_or_default().join(".zsh")) {
        println!("    fpath=(~/.zsh/completion $fpath)");
    } else {
        println!("    fpath=(~/.zfunc $fpath)");
    }
    println!("    autoload -Uz compinit && compinit");
    println!();
    println!("Then reload your shell:");
    println!("    exec zsh");
}

/// Install bash completion to `~/.bash_completion.d/norb`.
pub fn install_bash(home: &Path) -> Result<PathBuf> {
    let script = generate_shell_script(Shell::Bash);
    let bash_dir = home.join(".bash_completion.d");
    fs::create_dir_all(&bash_dir)
        .with_context(|| format!("failed to create directory {}", bash_dir.display()))?;

    let dest = bash_dir.join("norb");
    fs::write(&dest, &script)
        .with_context(|| format!("failed to write {}", dest.display()))?;

    Ok(dest)
}

/// Print user activation instructions for bash.
fn print_bash_instructions(file: &Path) {
    println!("✓ Installed bash completion to {}", file.display());
    println!();
    println!("To activate completions, add the following to your ~/.bashrc (or ~/.bash_profile):");
    println!();
    println!("    [[ -r ~/.bash_completion.d/norb ]] && source ~/.bash_completion.d/norb");
    println!();
    println!("Then reload your shell:");
    println!("    source ~/.bashrc");
}

/// Install fish completion to `~/.config/fish/completions/norb.fish`.
pub fn install_fish(home: &Path) -> Result<PathBuf> {
    let script = generate_shell_script(Shell::Fish);
    let fish_dir = home.join(".config").join("fish").join("completions");
    fs::create_dir_all(&fish_dir)
        .with_context(|| format!("failed to create directory {}", fish_dir.display()))?;

    let dest = fish_dir.join("norb.fish");
    fs::write(&dest, &script)
        .with_context(|| format!("failed to write {}", dest.display()))?;

    Ok(dest)
}

/// Print user activation instructions for fish.
fn print_fish_instructions(file: &Path) {
    println!("✓ Installed fish completion to {}", file.display());
    println!();
    println!("Fish automatically loads completions on new sessions.");
    println!("To load in your current session, run:");
    println!();
    println!("    source {}", file.display());
}

/// Generate shell autocompletion script, or install it to the appropriate shell directory.
pub fn generate_completion(shell_str: &str, install: bool) -> Result<()> {
    if !install {
        let target_shell = if shell_str.eq_ignore_ascii_case("auto") {
            detect_active_shell().unwrap_or(Shell::Zsh)
        } else {
            parse_shell(shell_str)?
        };

        let script = generate_shell_script(target_shell);
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&script)?;
        stdout.flush()?;
        return Ok(());
    }

    let home = dirs::home_dir().context("could not determine user home directory")?;

    // Determine if shell argument was explicitly passed or fell back to clap's default "zsh"
    let explicit_shell = std::env::args().skip(2).any(|arg| {
        !arg.starts_with('-')
            && matches!(
                arg.to_ascii_lowercase().as_str(),
                "zsh" | "bash" | "fish" | "elvish" | "powershell" | "pwsh" | "all" | "auto"
            )
    });

    if explicit_shell {
        match shell_str.to_ascii_lowercase().as_str() {
            "zsh" => {
                let path = install_zsh(&home)?;
                print_zsh_instructions(&path);
            }
            "bash" => {
                let path = install_bash(&home)?;
                print_bash_instructions(&path);
            }
            "fish" => {
                let path = install_fish(&home)?;
                print_fish_instructions(&path);
            }
            "all" => {
                let zpath = install_zsh(&home)?;
                let bpath = install_bash(&home)?;
                let fpath = install_fish(&home)?;
                println!("✓ Installed completions for all supported shells:");
                println!("  - Zsh:  {}", zpath.display());
                println!("  - Bash: {}", bpath.display());
                println!("  - Fish: {}", fpath.display());
                println!();
                print_zsh_instructions(&zpath);
                println!();
                print_bash_instructions(&bpath);
            }
            "auto" => {
                let active = detect_active_shell().unwrap_or(Shell::Zsh);
                match active {
                    Shell::Bash => {
                        let path = install_bash(&home)?;
                        print_bash_instructions(&path);
                    }
                    Shell::Fish => {
                        let path = install_fish(&home)?;
                        print_fish_instructions(&path);
                    }
                    _ => {
                        let path = install_zsh(&home)?;
                        print_zsh_instructions(&path);
                    }
                }
            }
            other => {
                bail!(
                    "Direct installation is not supported for '{other}'. Use `norb completion {other}` to output completion script."
                );
            }
        }
    } else {
        // No explicit shell specified on CLI: auto-detect user's active shell or write both zsh and bash completions
        let active = detect_active_shell().unwrap_or(Shell::Zsh);
        match active {
            Shell::Bash => {
                let bpath = install_bash(&home)?;
                print_bash_instructions(&bpath);
            }
            Shell::Fish => {
                let fpath = install_fish(&home)?;
                print_fish_instructions(&fpath);
            }
            _ => {
                // macOS standard / default: install zsh completion and also install bash completion
                let zpath = install_zsh(&home)?;
                let bpath = install_bash(&home)?;
                print_zsh_instructions(&zpath);
                println!();
                print_bash_instructions(&bpath);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_shells() {
        assert_eq!(parse_shell("zsh").unwrap(), Shell::Zsh);
        assert_eq!(parse_shell("bash").unwrap(), Shell::Bash);
        assert_eq!(parse_shell("fish").unwrap(), Shell::Fish);
        assert_eq!(parse_shell("elvish").unwrap(), Shell::Elvish);
        assert_eq!(parse_shell("powershell").unwrap(), Shell::PowerShell);
        assert_eq!(parse_shell("pwsh").unwrap(), Shell::PowerShell);
        assert_eq!(parse_shell("ZSH").unwrap(), Shell::Zsh);
        assert_eq!(parse_shell("  Bash  ").unwrap(), Shell::Bash);
    }

    #[test]
    fn test_parse_invalid_shell() {
        assert!(parse_shell("cmd").is_err());
        assert!(parse_shell("csh").is_err());
    }

    #[test]
    fn test_generate_zsh_script() {
        let script = generate_shell_script(Shell::Zsh);
        let s = String::from_utf8_lossy(&script);
        assert!(s.contains("#compdef norb") || s.contains("_norb"));
        assert!(s.contains("norbctl"));
    }

    #[test]
    fn test_generate_bash_script() {
        let script = generate_shell_script(Shell::Bash);
        let s = String::from_utf8_lossy(&script);
        assert!(s.contains("_norb"));
        assert!(s.contains("norbctl"));
    }

    #[test]
    fn test_generate_fish_script() {
        let script = generate_shell_script(Shell::Fish);
        let s = String::from_utf8_lossy(&script);
        assert!(s.contains("complete -c norb"));
        assert!(s.contains("norbctl"));
    }

    #[test]
    fn test_detect_active_shell() {
        // SHELL is usually set on macOS/Linux
        if std::env::var("SHELL").is_ok() {
            let detected = detect_active_shell();
            assert!(detected.is_some());
        }
    }
}
