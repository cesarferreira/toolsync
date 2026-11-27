use std::collections::{BTreeMap, BTreeSet};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::git;
use crate::paths::Paths;
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
    if !offline {
        for (name, result) in check_remotes(&list) {
            let st = state.tools.entry(name.clone()).or_default();
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
                }
                Err(message) => {
                    if st.error.is_none() {
                        st.error = Some(ToolError::new(ErrorKind::Refresh, message));
                    } else {
                        unreachable.insert(name, message);
                    }
                }
            }
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
        "CHECKOUT".into(),
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
                "{}: no longer in the tool list; run {} to delete its local checkout",
                ui::name(name),
                ui::cmd(format!("tsync remove {name}"))
            ));
        } else if let Some(err) = &st.error {
            details.push(format!("{}: {}", ui::name(name), ui::dim(&err.message)));
        }
        if let Some(message) = unreachable.get(name) {
            details.push(format!("{}: {}", ui::name(name), ui::dim(message)));
        }
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

/// Read remote `main` for every tool at once, with a spinner on a terminal.
fn check_remotes(list: &ToolList) -> Vec<(String, Result<String, String>)> {
    if list.tools.is_empty() {
        return Vec::new();
    }
    let spinner = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr());
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .expect("valid template")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "]),
    );
    spinner.set_message(
        ui::dim(format!(
            "Checking remote main for {} tool(s)",
            list.tools.len()
        ))
        .to_string(),
    );
    spinner.enable_steady_tick(Duration::from_millis(80));

    let results = thread::scope(|s| {
        let handles: Vec<_> = list
            .tools
            .iter()
            .map(|(name, tool)| {
                s.spawn(move || {
                    let result = match git::remote_main(None, &tool.url) {
                        Ok(Some(sha)) => Ok(sha),
                        Ok(None) => Err(format!("{} has no `main` branch", tool.url)),
                        Err(err) => Err(format!("{err:#}")),
                    };
                    (name.clone(), result)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("remote check thread panicked"))
            .collect()
    });
    spinner.finish_and_clear();
    results
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
