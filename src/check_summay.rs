use std::collections::HashSet;

use anyhow::{Context, Result, anyhow};
use regex::RegexBuilder;

use crate::calc_summary::SummaryDefinition;

pub fn check_summary_definitions(definitions: &[SummaryDefinition]) -> Result<()> {
    let mut seen_names = HashSet::new();
    let mut seen_regex = HashSet::new();

    for def in definitions {
        let name = def.name.trim();
        let regex_text = def.regex.trim();

        let name_key = name.to_lowercase();
        if !seen_names.insert(name_key) {
            return Err(anyhow!("duplicate summary definition name '{}'", def.name));
        }

        let regex_key = regex_text.to_lowercase();
        if !seen_regex.insert(regex_key) {
            return Err(anyhow!(
                "duplicate summary regex '{}' (summary '{}')",
                def.regex,
                def.name
            ));
        }

        let regex = RegexBuilder::new(regex_text)
            .case_insensitive(true)
            .build()
            .map_err(|err| anyhow!("invalid regex for summary '{}': {}", def.name, err))?;

        if regex.is_match("") {
            return Err(anyhow!(
                "regex for summary '{}' can match empty text; remove empty alternatives like trailing '|'",
                def.name
            ));
        }

        if let Some(color) = &def.color {
            parse_summary_color(color)
                .with_context(|| format!("invalid color for summary '{}'", def.name))?;
        }
    }

    Ok(())
}

pub fn parse_summary_color(color: &str) -> Result<u32> {
    let trimmed = color.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);

    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "invalid color '{}'; expected #RRGGBB or RRGGBB",
            color
        ));
    }

    u32::from_str_radix(hex, 16)
        .map_err(|_| anyhow!("invalid color '{}'; expected #RRGGBB or RRGGBB", color))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str, regex: &str) -> SummaryDefinition {
        SummaryDefinition {
            name: name.to_string(),
            description: String::new(),
            regex: regex.to_string(),
            color: None,
            lock_sign_on_first_match: true,
        }
    }

    #[test]
    fn rejects_duplicate_names() {
        let defs = vec![def("groceries", "shop"), def("Groceries", "food")];
        let err = check_summary_definitions(&defs).unwrap_err().to_string();
        assert!(err.contains("duplicate summary definition name"));
    }

    #[test]
    fn rejects_duplicate_regex() {
        let defs = vec![def("a", "power"), def("b", "POWER")];
        let err = check_summary_definitions(&defs).unwrap_err().to_string();
        assert!(err.contains("duplicate summary regex"));
    }

    #[test]
    fn rejects_empty_matching_regex() {
        let defs = vec![def("groceries", "new world|")];
        let err = check_summary_definitions(&defs).unwrap_err().to_string();
        assert!(err.contains("can match empty text"));
    }

    #[test]
    fn accepts_valid_hex_colors() {
        assert_eq!(parse_summary_color("#E2F0D9").unwrap(), 0xE2F0D9);
        assert_eq!(parse_summary_color("c6efd6").unwrap(), 0xC6EFD6);
    }

    #[test]
    fn rejects_invalid_hex_colors() {
        let err = parse_summary_color("green").unwrap_err().to_string();
        assert!(err.contains("expected #RRGGBB or RRGGBB"));
    }
}
