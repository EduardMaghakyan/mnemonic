mod doctor;

use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use chrono::{Duration as ChronoDuration, Local, NaiveDate, Utc};
use clap::{Parser, Subcommand};
use mnemonic_core::{count_entries, parse_since, walk_days, Config, DailyFile};

#[derive(Parser)]
#[command(name = "mnemonic", version, about = "Local voice notes — list, search, manage.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List daily notes newest-first
    Ls {
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Search across daily notes by case-insensitive substring
    Find {
        query: String,
        #[arg(long)]
        open: bool,
    },
    /// Print a daily note (defaults to today; accepts YYYY-MM-DD)
    Show { date: Option<String> },
    /// Health and configuration check
    Doctor,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Ls { since, limit } => cmd_ls(since.as_deref(), limit),
        Command::Show { date } => cmd_show(date.as_deref()),
        Command::Find { query, open } => cmd_find(&query, open),
        Command::Doctor => doctor::run(),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn load_notes_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "cannot determine home directory".to_string())?;
    let config_path = Config::default_path(&home);
    let config = Config::load_from(&config_path).unwrap_or_default();
    Ok(Config::expand_home(&config.paths.notes_dir, &home))
}

fn cmd_ls(since: Option<&str>, limit: Option<usize>) -> Result<(), String> {
    let notes_dir = load_notes_dir()?;
    let days = walk_days(&notes_dir, |p, e| {
        eprintln!("warning: {}: {e}", p.display())
    });

    let cutoff = match since {
        Some(s) => Some(
            (Utc::now()
                - ChronoDuration::from_std(parse_since(s)?)
                    .map_err(|e| format!("duration overflow: {e}"))?)
            .date_naive(),
        ),
        None => None,
    };

    let filtered: Vec<&DailyFile> = days
        .iter()
        .filter(|d| cutoff.map_or(true, |c| d.date >= c))
        .take(limit.unwrap_or(usize::MAX))
        .collect();

    if filtered.is_empty() {
        println!("(no notes)");
        return Ok(());
    }
    for d in &filtered {
        let n = count_entries(&d.body);
        let label = if n == 1 { "entry" } else { "entries" };
        println!("{date}  {n:>3} {label}", date = d.date.format("%Y-%m-%d"));
    }
    Ok(())
}

fn cmd_find(query: &str, open_first: bool) -> Result<(), String> {
    if query.is_empty() {
        return Err("query is empty".into());
    }
    let needle = query.to_lowercase();
    let notes_dir = load_notes_dir()?;
    let days = walk_days(&notes_dir, |p, e| {
        eprintln!("warning: {}: {e}", p.display())
    });

    let mut hits: Vec<FindHit> = Vec::new();
    for day in &days {
        hits.extend(day_matches(day, &needle));
    }

    if hits.is_empty() {
        println!("(no matches)");
        return Ok(());
    }

    for hit in &hits {
        println!("{}:{}", hit.path.display(), hit.line_no);
        for (offset, line) in &hit.context {
            let prefix = if *offset == 0 { ">" } else { " " };
            println!("  {prefix} {line}");
        }
        println!();
    }

    if open_first {
        let first = &hits[0];
        if let Err(e) = ProcessCommand::new("open").arg(&first.path).spawn() {
            return Err(format!("open: {e}"));
        }
    }
    Ok(())
}

struct FindHit {
    path: PathBuf,
    line_no: usize,
    context: Vec<(i32, String)>,
}

fn day_matches(day: &DailyFile, needle: &str) -> Vec<FindHit> {
    let mut hits = Vec::new();
    let lines: Vec<&str> = day.body.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.to_lowercase().contains(needle) {
            let mut ctx = Vec::with_capacity(3);
            if i > 0 {
                ctx.push((-1, lines[i - 1].to_string()));
            }
            ctx.push((0, line.to_string()));
            if i + 1 < lines.len() {
                ctx.push((1, lines[i + 1].to_string()));
            }
            hits.push(FindHit {
                path: day.path.clone(),
                line_no: i + 1,
                context: ctx,
            });
        }
    }
    hits
}

fn cmd_show(date: Option<&str>) -> Result<(), String> {
    let notes_dir = load_notes_dir()?;
    let target = match date {
        Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|e| format!("date {s:?} not YYYY-MM-DD: {e}"))?,
        None => Local::now().date_naive(),
    };
    let path = notes_dir.join(format!("{}.md", target.format("%Y-%m-%d")));
    if !path.exists() {
        return Err(format!("no daily note for {}", target.format("%Y-%m-%d")));
    }

    let bat = ProcessCommand::new("bat")
        .args(["--paging=never", "-l", "markdown"])
        .arg(&path)
        .status();
    if matches!(&bat, Ok(s) if s.success()) {
        return Ok(());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    print!("{content}");
    Ok(())
}
