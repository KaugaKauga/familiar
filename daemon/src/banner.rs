//! ASCII art banner for the Familiar daemon startup screen.

use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::ExecutableCommand;
use std::io::stdout;

const BANNER: &str = r#"
                          /\_/\
                     ____/ o o \
                   /~____  =ω=  /
                  (______)__m_m_/
                   |     /  \/
                   |    /  __\
                   |   / /'  `\
                   |  / /      \
              .-~~~|_/ /~~~~~~~~`-.
             /  ✦  .  .  ✦  .  ✦  \
            :  .  ✦  .  ✦  .  ✦  . :
             \ ✦  .  ✦  .  ✦  .  ✦/
              `-.____.~~~~.____.-'
                   ╱  FAMILIAR  ╲
"#;

const TAGLINE: &str = r#"
    ┌─────────────────────────────────────────────────────────────────┐
    │  Summoned by a label, your familiar toils through the night —  │
    │  from issue to pull request, while you rest.                   │
    └─────────────────────────────────────────────────────────────────┘
"#;

/// Print the familiar banner with colors to stdout.
pub fn print_banner() {
    let mut out = stdout();

    let _ = out.execute(SetForegroundColor(Color::Magenta));
    let _ = out.execute(Print(BANNER));
    let _ = out.execute(SetForegroundColor(Color::Cyan));
    let _ = out.execute(Print(TAGLINE));
    let _ = out.execute(ResetColor);
    let _ = out.execute(Print("\n"));
}
