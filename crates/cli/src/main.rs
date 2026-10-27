//! Command-line front end for cleaner-core: disk map, dry-run plan and clean.

use clap::{Parser, Subcommand};
use cleaner_core::delete::{self, Mode};
use cleaner_core::plan::Env;
use cleaner_core::{Config, Plan, PlanItem, build_plan, scan};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "cleaner-cli",
    version,
    about = "Safe disk cleaner (dry-run first)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the biggest children of a folder.
    Scan {
        path: PathBuf,
        #[arg(long, default_value_t = 20)]
        top: usize,
    },
    /// Dry run: show what would be cleaned. Deletes nothing.
    Plan {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Delete planned items (Recycle Bin by default).
    Clean {
        #[arg(long)]
        config: Option<PathBuf>,
        /// Only these categories (repeatable), e.g. --category temp.
        #[arg(long = "category", value_name = "NAME")]
        categories: Vec<String>,
        /// Also include review-level items (package caches, downloads, dev artifacts).
        #[arg(long)]
        include_review: bool,
        /// Delete immediately instead of sending to the Recycle Bin.
        #[arg(long)]
        permanent: bool,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
    /// Write the default config file.
    InitConfig {
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

fn default_config_path() -> PathBuf {
    cleaner_core::config::default_path()
}

fn load_config(path: Option<PathBuf>) -> Result<Config, String> {
    let path = path.unwrap_or_else(default_config_path);
    let mut cfg =
        Config::load(&path).map_err(|e| format!("cannot read config {}: {e}", path.display()))?;
    for w in cfg.sanitize(&cleaner_core::config::home_dir()) {
        eprintln!("config: {w}");
    }
    Ok(cfg)
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn print_items(plan: &Plan, items: &[&PlanItem]) {
    for (cat, group) in plan.by_category() {
        let group: Vec<_> = group
            .into_iter()
            .filter(|i| items.iter().any(|x| std::ptr::eq(*x, *i)))
            .collect();
        if group.is_empty() {
            continue;
        }
        let total: u64 = group.iter().map(|i| i.bytes).sum();
        println!(
            "\n[{cat}] ({}) {} in {} item(s)",
            group[0].risk,
            human(total),
            group.len()
        );
        for i in group {
            let mark = if i.selected_by_default { "x" } else { " " };
            println!("  [{mark}] {:>10}  {}", human(i.bytes), i.path.display());
        }
    }
}

fn cmd_plan(cfg: &Config, json: bool) -> ExitCode {
    let plan = build_plan(cfg, &Env::from_system());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&plan).expect("plan serializes")
        );
        return ExitCode::SUCCESS;
    }
    let all: Vec<&PlanItem> = plan.items.iter().collect();
    print_items(&plan, &all);
    println!("\nReclaimable:");
    println!(
        "  safe (selected by default): {}",
        human(plan.bytes_where(|i| i.selected_by_default))
    );
    println!(
        "  needs review:               {}",
        human(plan.bytes_where(|i| !i.selected_by_default))
    );
    println!("  total:                      {}", human(plan.total_bytes));
    println!("Dry run only; nothing was deleted. Use `clean` to delete.");
    ExitCode::SUCCESS
}

fn confirm(prompt: &str) -> bool {
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).is_ok() && line.trim().eq_ignore_ascii_case("y")
}

fn cmd_clean(
    cfg: &Config,
    categories: &[String],
    include_review: bool,
    permanent: bool,
    yes: bool,
) -> ExitCode {
    let env = Env::from_system();
    let plan = build_plan(cfg, &env);
    let chosen: Vec<&PlanItem> = plan
        .items
        .iter()
        .filter(|i| i.selected_by_default || include_review)
        .filter(|i| {
            categories.is_empty()
                || categories
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(&i.category))
        })
        .collect();
    if chosen.is_empty() {
        println!("Nothing to clean.");
        if !include_review && !categories.is_empty() {
            println!("(Review categories need --include-review.)");
        }
        return ExitCode::SUCCESS;
    }
    print_items(&plan, &chosen);
    let total: u64 = chosen.iter().map(|i| i.bytes).sum();
    let mode = if permanent {
        Mode::Permanent
    } else {
        Mode::Trash
    };
    let how = if permanent {
        "PERMANENTLY delete"
    } else {
        "move to Recycle Bin"
    };
    println!();
    if !yes
        && !confirm(&format!(
            "{how} {} item(s), {}?",
            chosen.len(),
            human(total)
        ))
    {
        println!("Aborted; nothing deleted.");
        return ExitCode::FAILURE;
    }
    let owned: Vec<PlanItem> = chosen.into_iter().cloned().collect();
    let outcomes = delete::execute(&owned, mode, &cfg.protected, &env.user_profile);
    let mut freed = 0;
    let mut failed = 0;
    for o in &outcomes {
        match &o.error {
            None => {
                freed += o.bytes;
                println!("  ok      {:>10}  {}", human(o.bytes), o.path.display());
            }
            Some(e) => {
                failed += 1;
                println!(
                    "  FAILED  {:>10}  {}  ({e})",
                    human(o.bytes),
                    o.path.display()
                );
            }
        }
    }
    let note = if permanent {
        ""
    } else {
        " (empty the Recycle Bin to reclaim space)"
    };
    println!("\nFreed {}{note}; {failed} item(s) failed.", human(freed));
    if failed > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

fn cmd_init(path: Option<PathBuf>) -> Result<(), String> {
    let path = path.unwrap_or_else(default_config_path);
    Config::default().save(&path).map_err(|e| e.to_string())?;
    println!("Wrote default config to {}", path.display());
    Ok(())
}

fn cmd_scan(path: &Path, top: usize) -> ExitCode {
    if !path.is_dir() {
        eprintln!("not a directory: {}", path.display());
        return ExitCode::FAILURE;
    }
    let children = scan::top_children(path, top);
    let total = scan::dir_size(path);
    println!("{}  {}", human(total), path.display());
    for c in children {
        let pct = if total > 0 {
            c.bytes as f64 * 100.0 / total as f64
        } else {
            0.0
        };
        println!("  {:>10}  {pct:5.1}%  {}", human(c.bytes), c.path.display());
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let run = || -> Result<ExitCode, String> {
        Ok(match cli.cmd {
            Cmd::Scan { path, top } => cmd_scan(&path, top),
            Cmd::Plan { config, json } => cmd_plan(&load_config(config)?, json),
            Cmd::Clean {
                config,
                categories,
                include_review,
                permanent,
                yes,
            } => cmd_clean(
                &load_config(config)?,
                &categories,
                include_review,
                permanent,
                yes,
            ),
            Cmd::InitConfig { config } => {
                cmd_init(config)?;
                ExitCode::SUCCESS
            }
        })
    };
    run().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        ExitCode::FAILURE
    })
}

#[cfg(test)]
mod tests {
    use super::human;

    #[test]
    fn human_sizes() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1023), "1023 B");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(5 * 1024 * 1024 * 1024), "5.0 GB");
    }
}
