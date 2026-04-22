//! ASCII art banner for the Familiar daemon startup screen.

use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::ExecutableCommand;
use std::io::stdout;

const BANNER: &str = r#"
                   ·   ˚    .    ✦    .   ·   ˚
               ✦          .-"""""-.           .
                         /  _   _  \
             .          ;  (o) (o) ;          ✦
                        |     ᵥ     |
              ✦          \   \_/   /          .
                          '._____.'
                    ·       |   |       ·
                            |   |
                       ╱╲   |   |   ╱╲
                      ╱  ╲__|   |__╱  ╲
                     ╱                ╲
                    ╱   F A M I L I A R ╲
                    ╲__________________╱
                        ~    ~    ~
"#;

const TAGLINE: &str = r#"
    ┌─────────────────────────────────────────────────────────────────┐
    │  "Bind a familiar to an issue, and wake to a pull request.     │
    │   It works in the dark so you don't have to."                  │
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_and_tagline_non_empty() {
        assert!(!BANNER.trim().is_empty());
        assert!(!TAGLINE.trim().is_empty());
    }

    #[test]
    fn banner_does_not_reference_old_name() {
        // Guard against accidental reverts to the "Guild" branding.
        let combined = format!("{}{}", BANNER, TAGLINE).to_lowercase();
        assert!(!combined.contains("guild"));
    }

    #[test]
    fn banner_mentions_familiar() {
        let combined = format!("{}{}", BANNER, TAGLINE).to_lowercase();
        assert!(combined.contains("familiar"));
    }
}
