//! Bash source fragments that compose without quoting hazards.
//!
//! `func` and `case` render multi-line bodies, so a definition may contain
//! comments, `case`/`esac`, and newlines — none of which survive the
//! `name() { body; }` one-liner form.

use std::fmt;

const INDENT: &str = "    ";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BashSrc(String);

impl BashSrc {
    pub fn empty() -> Self {
        Self(String::new())
    }

    pub fn raw(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn seq(parts: impl IntoIterator<Item = BashSrc>) -> Self {
        Self(
            parts
                .into_iter()
                .filter(|part| !part.is_empty())
                .map(|part| part.0)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    pub fn func(name: &str, body: BashSrc) -> Self {
        Self(format!("{name}() {{\n{}\n}}", indent(&body.0, 1)))
    }

    pub fn case(subject: &str, arms: impl IntoIterator<Item = (String, BashSrc)>) -> Self {
        let mut out = format!("case {subject} in\n");
        for (pattern, body) in arms {
            out.push_str(&format!("{INDENT}{pattern})\n"));
            out.push_str(&indent(&body.0, 2));
            out.push_str(&format!("\n{INDENT}{INDENT};;\n"));
        }
        out.push_str("esac");
        Self(out)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Display for BashSrc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn indent(text: &str, levels: usize) -> String {
    let pad = INDENT.repeat(levels);
    text.lines()
        .map(|line| {
            if line.trim().is_empty() {
                String::new()
            } else {
                format!("{pad}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Multi-line bodies with comments survive `func`; `seq` drops empties;
    /// `case` nests its arms. None of this holds for a one-liner rendering.
    #[test]
    fn composition_renders_readable_bash() {
        let body = BashSrc::seq([
            BashSrc::raw("# a comment"),
            BashSrc::empty(),
            BashSrc::raw("local value=1"),
        ]);
        assert_eq!(
            BashSrc::func("demo", body).as_str(),
            "demo() {\n    # a comment\n    local value=1\n}"
        );
        assert_eq!(
            BashSrc::case("\"$1\"", [("'a'".to_string(), BashSrc::raw("echo a"))]).as_str(),
            "case \"$1\" in\n    'a')\n        echo a\n        ;;\nesac"
        );
    }
}
