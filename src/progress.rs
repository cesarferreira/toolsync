//! Live per-tool spinners when stderr is a terminal; plain lines otherwise.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::time::Duration;

use console::strip_ansi_codes;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};

use crate::ui;

const TICK: Duration = Duration::from_millis(80);
const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "✔"];

pub struct Board {
    multi: MultiProgress,
    live: bool,
    width: usize,
}

/// One tool's line on the board.
pub struct Row {
    bar: ProgressBar,
    name: String,
    live: bool,
    width: usize,
}

impl Board {
    pub fn new<'a>(names: impl IntoIterator<Item = &'a String>) -> (Self, Vec<Row>) {
        let multi = MultiProgress::with_draw_target(ProgressDrawTarget::stderr());
        let live = !multi.is_hidden();
        let names: Vec<&String> = names.into_iter().collect();
        let width = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
        let style = ProgressStyle::with_template(
            "{spinner:.cyan} {prefix:.bold.cyan}  {t:.dim}  {wide_msg}",
        )
        .expect("valid template")
        .with_key("t", |state: &ProgressState, w: &mut dyn std::fmt::Write| {
            let _ = w.write_str(&elapsed(state.elapsed()));
        })
        .tick_strings(SPINNER);

        let rows = names
            .into_iter()
            .map(|name| {
                let bar = multi.add(ProgressBar::new_spinner());
                bar.set_style(style.clone());
                bar.set_prefix(format!("{name:<width$}"));
                bar.set_message(ui::dim("waiting").to_string());
                if live {
                    bar.enable_steady_tick(TICK);
                }
                Row {
                    bar,
                    name: name.clone(),
                    live,
                    width,
                }
            })
            .collect();
        (Self { multi, live, width }, rows)
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    pub fn width(&self) -> usize {
        self.width
    }

    /// Print above the bars without tearing them.
    pub fn println(&self, line: &str) {
        if self.live {
            let _ = self.multi.println(line);
        } else {
            eprintln!("{line}");
        }
    }
}

impl Row {
    pub fn is_live(&self) -> bool {
        self.live
    }

    /// Show what this tool is doing right now.
    pub fn stage(&self, msg: impl Into<Cow<'static, str>>) {
        self.bar.set_message(msg);
    }

    /// Show the latest line of a running command's output.
    pub fn activity(&self, what: &str, line: &str) {
        let line = strip_ansi_codes(line.trim()).to_string();
        self.stage(format!(
            "{} {}",
            ui::warn(what),
            ui::dim(format!("· {line}"))
        ));
    }

    pub fn success(&self, outcome: &str) {
        if self.live {
            let msg = format!(
                "{} {}  {}  {}",
                ui::good("✔"),
                ui::name(format!("{:<w$}", self.name, w = self.width)),
                ui::dim(elapsed(self.bar.elapsed())),
                outcome
            );
            self.finish(msg);
        } else {
            println!("{} {}", ui::name(&self.name), outcome);
        }
    }

    pub fn skipped(&self, reason: &str) {
        if self.live {
            self.done(ui::warn("●"), ui::warn(format!("skipped: {reason}")));
        } else {
            println!("{} skipped: {reason}", ui::name(&self.name));
        }
    }

    pub fn failure(&self, summary: &str, message: &str) {
        if self.live {
            let msg = format!(
                "{} {}  {}  {}",
                ui::err_bad("✖"),
                ui::err_name(format!("{:<w$}", self.name, w = self.width)),
                ui::err_dim(elapsed(self.bar.elapsed())),
                ui::err_bad(summary)
            );
            self.finish(msg);
        } else {
            eprintln!(
                "{} {} {}",
                ui::err_name(&self.name),
                ui::err_bad(format!("{summary}:")),
                ui::err_dim(message)
            );
        }
    }

    /// Finish a live row with a custom mark. Non-terminal output is left to the caller.
    pub fn done(&self, mark: impl std::fmt::Display, outcome: impl std::fmt::Display) {
        if self.live {
            let msg = format!(
                "{} {}  {}  {}",
                mark,
                ui::name(format!("{:<w$}", self.name, w = self.width)),
                ui::dim(elapsed(self.bar.elapsed())),
                outcome
            );
            self.finish(msg);
        } else {
            self.bar.finish_and_clear();
        }
    }

    fn finish(&self, msg: String) {
        self.bar
            .set_style(ProgressStyle::with_template("{msg}").expect("valid template"));
        self.bar.finish_with_message(msg);
    }
}

/// Fixed-width elapsed time so running and finished rows line up.
fn elapsed(d: Duration) -> String {
    let secs = d.as_secs_f64();
    let mut out = String::new();
    if secs < 60.0 {
        let _ = write!(out, "{secs:>5.1}s");
    } else {
        let s = secs as u64;
        let _ = write!(out, "{:>2}m{:02}s", s / 60, s % 60);
    }
    out
}
