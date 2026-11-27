use std::collections::{BTreeMap, BTreeSet};
use std::thread;

use anyhow::Result;

use crate::git;
use crate::paths::Paths;
use crate::progress::Board;
use crate::state::{ErrorKind, State, ToolError, ToolState, ago, now};
use crate::sync::{acquire_lock, short};
use crate::tools::{Tool, ToolList};
use crate::ui;

/// Print this machine's state. Unless `offline`, first reads remote `main` for
/// every tool in parallel; local files only otherwise.
pub fn run(paths: &Paths, offline: bool) -> Result<()> {
    let list = ToolList::load(paths)?;
    let names: BTreeSet<String> = list.tools.keys().cloned().collect();
    if names.is_empty() && !paths.state_file().exists() {
        println!(
            "No tools registered. Run {} inside a repo.",
            ui::cmd("tsync add")
        );
        return Ok(());
    }

    // Hold the sync lock while refreshing so a running sync's state is never
    // overwritten. If a sync is running, show the live result without saving it.
    let lock = if offline {
        None
    } else {
        acquire_lock(paths).ok()
    };
    let mut state = State::load(paths)?;
    let now = now();
    let mut unreachable = BTreeMap::new();
    let mut live = false;
    if !offline && !list.tools.is_empty() {
        let (board, rows) = Board::new(list.tools.keys());
        live = board.is_live();
        if live {
            board.println(&ui::dim(format!("Checking {} tool(s)", list.tools.len())).to_string());
        }
        let state_ref = &state;
        let results: Vec<_> = thread::scope(|s| {
            let handles: Vec<_> = list
                .tools
                .iter()
                .zip(rows)
                .map(|((name, tool), row)| {
                    s.spawn(move || {
                        row.stage(ui::warn("checking remote main").to_string());
                        let mut st = state_ref.tools.get(name).cloned().unwrap_or_default();
                        let missed = apply(&mut st, check_remote(tool), now);
                        let (label, tone) = describe(Some(tool), &st);
                        row.done(
                            tone.mark(),
                            format!("{} {}", tone.paint(label), ui::dim(shas(&st))),
                        );
                        (name.clone(), st, missed)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("status thread panicked"))
                .collect()
        });
        for (name, st, missed) in results {
            if let Some(message) = missed {
                unreachable.insert(name.clone(), message);
            }
            state.tools.insert(name, st);
        }
        if lock.is_some() {
            state.save(paths)?;
        }
    }
    drop(lock);

    let names: BTreeSet<&String> = list.tools.keys().chain(state.tools.keys()).collect();
    if names.is_empty() {
        println!(
            "No tools registered. Run {} inside a repo.",
            ui::cmd("tsync add")
        );
        return Ok(());
    }

    let empty = ToolState::default();
    let mut rows = vec![[
        "TOOL".to_string(),
        "MAIN".into(),
        "LOCAL".into(),
        "INSTALLED".into(),
        "CHECKED".into(),
        "STATUS".into(),
    ]];
    let mut tones = vec![Tone::Good];
    let mut details = Vec::new();
    for name in names {
        let tool = list.tools.get(name);
        let st = state.tools.get(name).unwrap_or(&empty);
        rows.push([
            name.clone(),
            sha(&st.remote_main),
            sha(&st.checkout),
            sha(&st.installed),
            ago(st.remote_checked_at, now),
            String::new(),
        ]);
        let (label, tone) = describe(tool, st);
        rows.last_mut().expect("just pushed")[5] = label;
        tones.push(tone);
        if tool.is_none() {
            details.push(format!(
                "{}: no longer in the tool list; run {} to forget it",
                ui::name(name),
                ui::cmd(format!("tsync remove {name}"))
            ));
        } else if let Some(err) = &st.error {
            // git's "couldn't reach" output runs over several lines; the first says enough.
            let message = match err.kind {
                ErrorKind::Refresh => first_line(&err.message),
                _ => &err.message,
            };
            details.push(format!("{}: {}", ui::name(name), ui::dim(message)));
        }
        if let Some(message) = unreachable.get(name) {
            details.push(format!(
                "{}: {}",
                ui::name(name),
                ui::dim(first_line(message))
            ));
        }
    }

    if live {
        // The board already showed one finished row per tool.
        if !details.is_empty() {
            println!();
        }
        for d in &details {
            println!("{d}");
        }
        if tones.iter().any(|t| matches!(t, Tone::Warn)) {
            println!("Run {} to pull and install.", ui::cmd("tsync"));
        }
        return Ok(());
    }

    let mut widths = [0; 6];
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    for (r, (row, tone)) in rows.iter().zip(&tones).enumerate() {
        let mut line = String::new();
        for (i, (cell, w)) in row.iter().zip(widths).enumerate() {
            // Pad the plain text first so color codes don't break alignment.
            let padded = if i == row.len() - 1 {
                cell.clone()
            } else {
                format!("{cell:<w$}  ")
            };
            let styled = match (r, i) {
                (0, _) => ui::bold(padded).to_string(),
                (_, 0) => ui::name(padded).to_string(),
                (_, 4) => ui::dim(padded).to_string(),
                (_, 5) => tone.paint(padded),
                _ => padded,
            };
            line.push_str(&styled);
        }
        println!("{line}");
    }

    if !details.is_empty() {
        println!();
        for d in details {
            println!("{d}");
        }
    }
    println!();
    let footer = if offline {
        "CHECKED is when remote main was last read; run `tsync status` for a live result."
    } else {
        "CHECKED is when remote main was last read; run `tsync` to pull and install."
    };
    println!("{}", ui::dim(footer));
    Ok(())
}

/// Read remote `main` for one tool.
fn check_remote(tool: &Tool) -> Result<String, String> {
    match git::remote_main(None, &tool.url) {
        Ok(Some(sha)) => Ok(sha),
        Ok(None) => Err(format!("{} has no `main` branch", tool.url)),
        Err(err) => Err(format!("{err:#}")),
    }
}

/// Record a live check. Returns the error message when it can't be stored
/// because the tool already has a more important error.
fn apply(st: &mut ToolState, result: Result<String, String>, now: u64) -> Option<String> {
    match result {
        Ok(sha) => {
            st.remote_main = Some(sha);
            st.remote_checked_at = Some(now);
            if st
                .error
                .as_ref()
                .is_some_and(|e| e.kind == ErrorKind::Refresh)
            {
                st.error = None;
            }
            None
        }
        Err(message)
            if st
                .error
                .as_ref()
                .is_none_or(|e| e.kind == ErrorKind::Refresh) =>
        {
            st.error = Some(ToolError::new(ErrorKind::Refresh, message));
            None
        }
        Err(message) => Some(message),
    }
}

/// Short SHA context for a finished live row.
fn shas(st: &ToolState) -> String {
    match (&st.checkout, &st.remote_main) {
        (Some(c), Some(m)) if c != m => format!("{} → {}", short(c), short(m)),
        (Some(c), _) => short(c).to_string(),
        (None, Some(m)) => format!("main {}", short(m)),
        (None, None) => String::new(),
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text)
}

fn sha(value: &Option<String>) -> String {
    value.as_deref().map(short).unwrap_or("—").to_string()
}

#[derive(Clone, Copy)]
enum Tone {
    Good,
    Warn,
    Bad,
}

impl Tone {
    fn mark(self) -> String {
        match self {
            Tone::Good => ui::good("✔").to_string(),
            Tone::Warn => ui::warn("●").to_string(),
            Tone::Bad => ui::bad("✖").to_string(),
        }
    }

    fn paint(self, text: String) -> String {
        match self {
            Tone::Good => ui::good(text).to_string(),
            Tone::Warn => ui::warn(text).to_string(),
            Tone::Bad => ui::bad(text).to_string(),
        }
    }
}

fn describe(tool: Option<&Tool>, st: &ToolState) -> (String, Tone) {
    let Some(tool) = tool else {
        return ("not in tool list".into(), Tone::Warn);
    };
    if let Some(err) = &st.error {
        return (err.summary().to_string(), Tone::Bad);
    }
    if let Some(reason) = &st.skipped {
        return (format!("skipped: {reason}"), Tone::Warn);
    }
    let Some(checkout) = &st.checkout else {
        return ("not synced yet".into(), Tone::Warn);
    };
    if st.remote_main.as_ref() != Some(checkout) {
        return ("behind main".into(), Tone::Warn);
    }
    if tool.install.is_none() {
        ("synced (pull only)".into(), Tone::Good)
    } else if st.installed.as_ref() == Some(checkout) {
        ("installed".into(), Tone::Good)
    } else {
        ("install pending".into(), Tone::Warn)
    }
}
