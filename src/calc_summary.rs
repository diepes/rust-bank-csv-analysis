use std::fs;
use std::path::Path;
use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;

use crate::Transaction;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SummaryDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub regex: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SummaryItem {
    pub name: String,
    pub description: String,
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub items: Vec<SummaryItem>,
}

const NO_MATCH_SUMMARY_NAME: &str = "no_match";
const TOTAL_SUMMARY_NAME: &str = "total";

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
}

struct CompiledSummaryDefinition {
    name: String,
    description: String,
    regex: Regex,
}

pub fn default_summary_definitions() -> Vec<SummaryDefinition> {
    vec![
        SummaryDefinition {
            name: "power_payments_total".to_string(),
            description: "Total power payments".to_string(),
            regex: "power".to_string(),
        },
        SummaryDefinition {
            name: "mortgage_interest_total".to_string(),
            description: "Total mortgage interest".to_string(),
            regex: "mortgage.*interest|interest.*mortgage".to_string(),
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
            })
            .collect(),
    };

    validate_summary_definitions(&definitions)?;
    Ok(definitions)
}

pub fn summarize_for_period(
    transactions: &[Transaction],
    period_start: NaiveDate,
    period_end: NaiveDate,
    definitions: &[SummaryDefinition],
) -> Result<Summary> {
    validate_summary_definitions(definitions)?;
    let compiled = compile_summary_definitions(definitions)?;

    let mut totals: Vec<SummaryItem> = compiled
        .iter()
        .map(|def| SummaryItem {
            name: def.name.clone(),
            description: def.description.clone(),
            total: 0.0,
        })
        .collect();
    let mut no_match_total = 0.0;
    let mut total = 0.0;

    for tx in transactions
        .iter()
        .filter(|tx| tx.date >= period_start && tx.date <= period_end)
    {
        if tx.amount >= 0.0 {
            continue;
        }

        let amount = -tx.amount;
        total += amount;
        let text = searchable_text(tx);
        if let Some((idx, _)) = compiled
            .iter()
            .enumerate()
            .find(|(_, def)| def.regex.is_match(&text))
        {
            totals[idx].total += amount;
        } else {
            no_match_total += amount;
        }
    }

    let classified_total: f64 = totals.iter().map(|item| item.total).sum();
    let expected_total = classified_total + no_match_total;
    let epsilon = 1e-6;
    if (expected_total - total).abs() > epsilon {
        return Err(anyhow!(
            "summary totals do not add up: configured + no_match = {expected_total:.2}, total = {total:.2}"
        ));
    }

    totals.push(SummaryItem {
        name: NO_MATCH_SUMMARY_NAME.to_string(),
        description: "Total unmatched transactions".to_string(),
        total: no_match_total,
    });
    totals.push(SummaryItem {
        name: TOTAL_SUMMARY_NAME.to_string(),
        description: "Total matched and unmatched transactions".to_string(),
        total,
    });

    Ok(Summary { items: totals })
}

fn validate_summary_definitions(definitions: &[SummaryDefinition]) -> Result<()> {
    if definitions.is_empty() {
        return Err(anyhow!("summary definitions cannot be empty"));
    }

    for def in definitions {
        if def.name.trim().is_empty() {
            return Err(anyhow!("summary definition name cannot be empty"));
        }
        if def.name == NO_MATCH_SUMMARY_NAME || def.name == TOTAL_SUMMARY_NAME {
            return Err(anyhow!(
                "summary definition name '{}' is reserved",
                def.name
            ));
        }
        if def.regex.trim().is_empty() {
            return Err(anyhow!(
                "summary definition regex cannot be empty for '{}'",
                def.name
            ));
        }
    }

    Ok(())
}

fn compile_summary_definitions(
    definitions: &[SummaryDefinition],
) -> Result<Vec<CompiledSummaryDefinition>> {
    definitions
        .iter()
        .map(|def| {
            let regex = RegexBuilder::new(&def.regex)
                .case_insensitive(true)
                .build()
                .with_context(|| format!("invalid regex for summary '{}'", def.name))?;

            Ok(CompiledSummaryDefinition {
                name: def.name.clone(),
                description: def.description.clone(),
                regex,
            })
        })
        .collect()
}

fn searchable_text(tx: &Transaction) -> String {
    format!(
        "{} {} {} {} {} {} {} {}",
        tx.transaction_type,
        tx.source,
        tx.other_party,
        tx.particulars,
        tx.reference,
        tx.analysis_code,
        tx.serial_number,
        tx.account_code
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_matching_categories_for_apr_to_may_window() {
        let txs = vec![
            Transaction {
                account_number: "1".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 2).unwrap(),
                amount: -120.0,
                transaction_code: String::new(),
                transaction_type: "AUTOMATIC PAYMENT".into(),
                source: String::new(),
                other_party: "Power Co".into(),
                particulars: String::new(),
                analysis_code: String::new(),
                reference: String::new(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504020001".into(),
            },
            Transaction {
                account_number: "1".into(),
                date: NaiveDate::from_ymd_opt(2025, 5, 15).unwrap(),
                amount: -500.0,
                transaction_code: String::new(),
                transaction_type: "AUTOMATIC PAYMENT".into(),
                source: String::new(),
                other_party: "ABC Mortgage".into(),
                particulars: "mortgage interest".into(),
                analysis_code: String::new(),
                reference: String::new(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202505150001".into(),
            },
            Transaction {
                account_number: "1".into(),
                date: NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
                amount: -220.0,
                transaction_code: String::new(),
                transaction_type: "AUTOMATIC PAYMENT".into(),
                source: String::new(),
                other_party: "Power Co".into(),
                particulars: String::new(),
                analysis_code: String::new(),
                reference: String::new(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202506010001".into(),
            },
        ];

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
        let definitions = default_summary_definitions();
        let summary = summarize_for_period(&txs, start, end, &definitions).unwrap();

        assert_eq!(summary.items[0].name, "power_payments_total");
        assert_eq!(summary.items[0].total, 120.0);
        assert_eq!(summary.items[1].name, "mortgage_interest_total");
        assert_eq!(summary.items[1].total, 500.0);
        assert_eq!(summary.items[2].name, "no_match");
        assert_eq!(summary.items[2].total, 0.0);
        assert_eq!(summary.items[3].name, "total");
        assert_eq!(summary.items[3].total, 620.0);
    }

    #[test]
    fn tracks_unmatched_transactions_in_no_match() {
        let txs = vec![Transaction {
            account_number: "1".into(),
            date: NaiveDate::from_ymd_opt(2025, 4, 10).unwrap(),
            amount: -75.0,
            transaction_code: String::new(),
            transaction_type: "PAYMENT".into(),
            source: String::new(),
            other_party: "Unknown Vendor".into(),
            particulars: "misc expense".into(),
            analysis_code: String::new(),
            reference: String::new(),
            serial_number: String::new(),
            account_code: String::new(),
            unique_id: "202504100001".into(),
        }];

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
        let definitions = default_summary_definitions();
        let summary = summarize_for_period(&txs, start, end, &definitions).unwrap();

        assert_eq!(summary.items[0].total, 0.0);
        assert_eq!(summary.items[1].total, 0.0);
        assert_eq!(summary.items[2].name, "no_match");
        assert_eq!(summary.items[2].total, 75.0);
        assert_eq!(summary.items[3].name, "total");
        assert_eq!(summary.items[3].total, 75.0);
    }

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
                })
                .collect(),
        };

        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "mortgage_interest_total");
        assert_eq!(defs[1].name, "power_payments_total");
    }
}
