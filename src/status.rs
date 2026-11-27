use std::collections::BTreeSet;

use anyhow::Result;

use crate::paths::Paths;
use crate::state::{State, ToolState, ago, now};
use crate::sync::short;
use crate::tools::{Tool, ToolList};

/// Print this machine's last known state. Reads local files only.
pub fn run(paths: &Paths) -> Result<()> {
    let list = ToolList::load(paths)?;
    let state = State::load(paths)?;
    let now = now();

    let names: BTreeSet<&String> = list.tools.keys().chain(state.tools.keys()).collect();
    if names.is_empty() {
        println!("No tools registered. Run `toolsync add` inside a repo.");
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
            describe(tool, st),
        ]);
        if tool.is_none() {
            details.push(format!(
                "{name}: no longer in the tool list; run `toolsync remove {name}` to delete its local checkout"
            ));
        } else if let Some(err) = &st.error {
            details.push(format!("{name}: {}", err.message));
        }
    }

    let mut widths = [0; 6];
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    for row in &rows {
        let mut line = String::new();
        for (i, (cell, w)) in row.iter().zip(widths).enumerate() {
            if i == row.len() - 1 {
                line.push_str(cell);
            } else {
                let pad = w - cell.chars().count();
                line.push_str(cell);
                line.push_str(&" ".repeat(pad + 2));
            }
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
    println!("CHECKED is when remote main was last read; run `toolsync sync` for a live result.");
    Ok(())
}

fn sha(value: &Option<String>) -> String {
    value.as_deref().map(short).unwrap_or("—").to_string()
}

fn describe(tool: Option<&Tool>, st: &ToolState) -> String {
    let Some(tool) = tool else {
        return "not in tool list".into();
    };
    if let Some(err) = &st.error {
        return err.summary().to_string();
    }
    let Some(checkout) = &st.checkout else {
        return "not synced yet".into();
    };
    if st.remote_main.as_ref() != Some(checkout) {
        return "behind main".into();
    }
    if tool.install.is_none() {
        "synced (pull only)".into()
    } else if st.installed.as_ref() == Some(checkout) {
        "installed".into()
    } else {
        "install pending".into()
    }
}
