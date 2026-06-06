use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use super::summary::validate_summary_definitions;
use super::types::{default_lock_sign_on_first_match, SummaryDefinition};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SummaryConfigFile {
    List(Vec<SummaryDefinition>),
    Wrapped { summaries: Vec<SummaryDefinition> },
    Named(BTreeMap<String, SummaryDefinitionBody>),
}

#[derive(Debug, Deserialize)]
struct SummaryDefinitionBody {
    #[serde(default)]
    description: String,
    regex: String,
    #[serde(default, alias = "colour")]
    color: Option<String>,
}

pub fn default_summary_definitions() -> Vec<SummaryDefinition> {
    vec![
        SummaryDefinition {
            name: "power_payments_total".to_string(),
            description: "Total power payments".to_string(),
            regex: "power".to_string(),
            color: None,
            lock_sign_on_first_match: true,
        },
        SummaryDefinition {
            name: "mortgage_interest_total".to_string(),
            description: "Total mortgage interest".to_string(),
            regex: "mortgage.*interest|interest.*mortgage".to_string(),
            color: None,
            lock_sign_on_first_match: true,
        },
    ]
}

pub fn load_summary_definitions(path: impl AsRef<Path>) -> Result<Vec<SummaryDefinition>> {
    let path_ref = path.as_ref();
    let yaml = fs::read_to_string(path_ref)
        .with_context(|| format!("failed reading summary config '{}'", path_ref.display()))?;

    let parsed: SummaryConfigFile = serde_yaml::from_str(&yaml)
        .with_context(|| format!("invalid YAML in '{}'", path_ref.display()))?;

    let definitions = match parsed {
        SummaryConfigFile::List(defs) => defs,
        SummaryConfigFile::Wrapped { summaries } => summaries,
        SummaryConfigFile::Named(named) => named
            .into_iter()
            .map(|(name, body)| SummaryDefinition {
                name,
                description: body.description,
                regex: body.regex,
                color: body.color,
                lock_sign_on_first_match: default_lock_sign_on_first_match(),
            })
            .collect(),
    };

    validate_summary_definitions(&definitions)?;
    Ok(definitions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_summary_definitions_from_wrapped_yaml() {
        let yaml = "summaries:\n  - name: groceries\n    description: Grocery shops\n    regex: 'new world|pak n save'\n";
        let parsed: SummaryConfigFile = serde_yaml::from_str(yaml).unwrap();

        let defs = match parsed {
            SummaryConfigFile::List(defs) => defs,
            SummaryConfigFile::Wrapped { summaries } => summaries,
            SummaryConfigFile::Named(named) => named
                .into_iter()
                .map(|(name, body)| SummaryDefinition {
                    name,
                    description: body.description,
                    regex: body.regex,
                    color: body.color,
                    lock_sign_on_first_match: default_lock_sign_on_first_match(),
                })
                .collect(),
        };

        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "groceries");
        assert_eq!(defs[0].description, "Grocery shops");
        assert_eq!(defs[0].regex, "new world|pak n save");
    }

    #[test]
    fn loads_summary_definitions_from_named_yaml() {
        let yaml = "power_payments_total:\n  description: Total power payments\n  regex: power\nmortgage_interest_total:\n  description: Total mortgage interest\n  regex: mortgage.*interest|interest.*mortgage\n";
        let parsed: SummaryConfigFile = serde_yaml::from_str(yaml).unwrap();

        let defs = match parsed {
            SummaryConfigFile::List(defs) => defs,
            SummaryConfigFile::Wrapped { summaries } => summaries,
            SummaryConfigFile::Named(named) => named
                .into_iter()
                .map(|(name, body)| SummaryDefinition {
                    name,
                    description: body.description,
                    regex: body.regex,
                    color: body.color,
                    lock_sign_on_first_match: default_lock_sign_on_first_match(),
                })
                .collect(),
        };

        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "mortgage_interest_total");
        assert_eq!(defs[1].name, "power_payments_total");
    }
}
