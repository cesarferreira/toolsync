use std::collections::BTreeSet;

use anyhow::Result;

use crate::paths::Paths;
use crate::state::{State, ToolState, ago, now};
use crate::sync::short;
use crate::tools::{Tool, ToolList};
use crate::ui;

/// Print this machine's last known state. Reads local files only.
pub fn run(paths: &Paths) -> Result<()> {
    let list = ToolList::load(paths)?;
    let state = State::load(paths)?;
    let now = now();

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
    println!(
        "{}",
        ui::dim("CHECKED is when remote main was last read; run `tsync` for a live result.")
    );
    Ok(())
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
