mod doctor;
mod redo;

use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use chrono::{Duration as ChronoDuration, Local, Utc};
use clap::{Parser, Subcommand};
use mnemonic_core::{
    find_by_id_prefix, parse_since, walk_notes, Config, FindByPrefix, LoadedNote,
};

#[derive(Parser)]
#[command(name = "mnemonic", version, about = "Local voice notes — list, search, manage.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List notes newest-first
    Ls {
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Search notes by case-insensitive substring
    Find {
        query: String,
        #[arg(long)]
        open: bool,
    },
    /// Print a note by id or unambiguous prefix
    Show { id: String },
    /// Health and configuration check
    Doctor,
    /// Re-run the structuring prompt against the saved audio for a note
    Redo { id: String },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Ls { since, tag, limit } => {
            cmd_ls(since.as_deref(), tag.as_deref(), limit)
        }
        Command::Show { id } => cmd_show(&id),
        Command::Find { query, open } => cmd_find(&query, open),
        Command::Doctor => doctor::run(),
        Command::Redo { id } => redo::run(&id),
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

fn cmd_ls(since: Option<&str>, tag: Option<&str>, limit: Option<usize>) -> Result<(), String> {
    let notes_dir = load_notes_dir()?;
    let notes = walk_notes(&notes_dir, |p, e| {
        eprintln!("warning: {}: {e}", p.display())
    });

    let cutoff = match since {
        Some(s) => Some(
            Utc::now()
                - ChronoDuration::from_std(parse_since(s)?)
                    .map_err(|e| format!("duration overflow: {e}"))?,
        ),
        None => None,
    };

    let filtered: Vec<&LoadedNote> = notes
        .iter()
        .filter(|n| {
            if let Some(c) = cutoff {
                if n.created.with_timezone(&Utc) < c {
                    return false;
                }
            }
            if let Some(t) = tag {
                if !n.meta.tags.iter().any(|nt| nt == t) {
                    return false;
                }
            }
            true
        })
        .take(limit.unwrap_or(usize::MAX))
        .collect();

    if filtered.is_empty() {
        println!("(no notes)");
        return Ok(());
    }
    print_table(&filtered);
    Ok(())
}

fn print_table(notes: &[&LoadedNote]) {
    let today = Local::now().date_naive();
    for n in notes {
        let local = n.created.with_timezone(&Local);
        let time = if local.date_naive() == today {
            local.format("%H:%M").to_string()
        } else {
            local.format("%Y-%m-%d %H:%M").to_string()
        };
        let title = title_from_body(&n.body).unwrap_or_else(|| "(untitled)".to_string());
        let tags = n.meta.tags.join(", ");
        println!(
            "{time:<16}  {title:<48}  {tags}",
            title = truncate(&title, 48)
        );
    }
}

fn title_from_body(body: &str) -> Option<String> {
    body.lines()
        .find_map(|line| line.strip_prefix("# ").map(|s| s.to_string()))
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

fn cmd_find(query: &str, open_first: bool) -> Result<(), String> {
    if query.is_empty() {
        return Err("query is empty".into());
    }
    let needle = query.to_lowercase();
    let notes_dir = load_notes_dir()?;
    let notes = walk_notes(&notes_dir, |p, e| {
        eprintln!("warning: {}: {e}", p.display())
    });

    let mut hits: Vec<FindHit> = Vec::new();
    for note in &notes {
        let mut matches = note_matches(note, &needle);
        if !matches.is_empty() {
            hits.append(&mut matches);
        }
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

fn note_matches(note: &LoadedNote, needle: &str) -> Vec<FindHit> {
    let mut frontmatter_haystacks: Vec<String> = Vec::new();
    frontmatter_haystacks.extend(note.meta.tags.iter().cloned());
    frontmatter_haystacks.extend(note.meta.people.iter().cloned());
    frontmatter_haystacks.extend(note.meta.projects.iter().cloned());
    frontmatter_haystacks.extend(note.meta.places.iter().cloned());

    let mut hits = Vec::new();
    let lines: Vec<&str> = note.body.lines().collect();
    let mut emitted_lines: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();

    for (i, line) in lines.iter().enumerate() {
        if line.to_lowercase().contains(needle) && !emitted_lines.contains(&i) {
            emitted_lines.insert(i);
            let mut ctx = Vec::with_capacity(3);
            if i > 0 {
                ctx.push((-1, lines[i - 1].to_string()));
            }
            ctx.push((0, line.to_string()));
            if i + 1 < lines.len() {
                ctx.push((1, lines[i + 1].to_string()));
            }
            hits.push(FindHit {
                path: note.path.clone(),
                line_no: i + 1,
                context: ctx,
            });
        }
    }

    if hits.is_empty()
        && frontmatter_haystacks
            .iter()
            .any(|h| h.to_lowercase().contains(needle))
    {
        hits.push(FindHit {
            path: note.path.clone(),
            line_no: 0,
            context: vec![(0, format!("(matched in frontmatter: {})", note.meta.id))],
        });
    }

    hits
}

fn cmd_show(prefix: &str) -> Result<(), String> {
    let notes_dir = load_notes_dir()?;
    let notes = walk_notes(&notes_dir, |p, e| {
        eprintln!("warning: {}: {e}", p.display())
    });
    let note = match find_by_id_prefix(&notes, prefix) {
        FindByPrefix::One(n) => n,
        FindByPrefix::None => return Err(format!("no note matches {prefix:?}")),
        FindByPrefix::Many(ms) => {
            let ids: Vec<&str> = ms.iter().map(|n| n.meta.id.as_str()).collect();
            return Err(format!(
                "ambiguous prefix {prefix:?}; matches:\n  {}",
                ids.join("\n  ")
            ));
        }
    };

    let bat = ProcessCommand::new("bat")
        .args(["--paging=never", "-l", "markdown"])
        .arg(&note.path)
        .status();
    if matches!(&bat, Ok(s) if s.success()) {
        return Ok(());
    }

    let content = std::fs::read_to_string(&note.path)
        .map_err(|e| format!("read {}: {e}", note.path.display()))?;
    print!("{content}");
    Ok(())
}
