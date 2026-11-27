//! Terminal styling. Colors switch off automatically when output is not a terminal.

use std::fmt::Display;

use console::{StyledObject, style};

pub fn name<D: Display>(d: D) -> StyledObject<D> {
    style(d).cyan().bold()
}

pub fn good<D: Display>(d: D) -> StyledObject<D> {
    style(d).green()
}

pub fn warn<D: Display>(d: D) -> StyledObject<D> {
    style(d).yellow()
}

pub fn bad<D: Display>(d: D) -> StyledObject<D> {
    style(d).red()
}

pub fn dim<D: Display>(d: D) -> StyledObject<D> {
    style(d).dim()
}

pub fn bold<D: Display>(d: D) -> StyledObject<D> {
    style(d).bold()
}

pub fn cmd<D: Display>(d: D) -> StyledObject<D> {
    style(d).magenta()
}

pub fn err_name<D: Display>(d: D) -> StyledObject<D> {
    style(d).for_stderr().cyan().bold()
}

pub fn err_bad<D: Display>(d: D) -> StyledObject<D> {
    style(d).for_stderr().red()
}

pub fn err_dim<D: Display>(d: D) -> StyledObject<D> {
    style(d).for_stderr().dim()
}
