use std::fs;
use std::path::Path;
use std::collections::BTreeMap;
use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;

use crate::Transaction;
use crate::check_summay;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SummaryDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub regex: String,
    #[serde(default, alias = "colour")]
    pub color: Option<String>,
    #[serde(default = "default_lock_sign_on_first_match")]
    pub lock_sign_on_first_match: bool,
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
const LOAN_REPAYMENT_SUMMARY_NAME: &str = "loan_repayment_total";

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

struct CompiledSummaryDefinition {
    name: String,
    description: String,
    regex: Regex,
    lock_sign_on_first_match: bool,
}

#[derive(Debug, Clone)]
pub struct LoanRepaymentFlags {
    pub related: Vec<bool>,
    pub counted: Vec<bool>,
}

#[derive(Debug, Clone)]
struct SignLockState {
    expected_positive: bool,
    first_file: String,
    first_line: usize,
}

fn default_lock_sign_on_first_match() -> bool {
    true
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

pub fn summarize_for_period(
    transactions: &[Transaction],
    period_start: NaiveDate,
    period_end: NaiveDate,
    definitions: &[SummaryDefinition],
) -> Result<Summary> {
    validate_summary_definitions(definitions)?;
    let compiled = compile_summary_definitions(definitions)?;
    let internal_transfer_flags = detect_internal_transfers(transactions);
    let card_payment_flags = detect_card_payments(transactions);
    let loan_repayment_flags = detect_loan_repayments(transactions);

    let mut totals: Vec<SummaryItem> = compiled
        .iter()
        .map(|def| SummaryItem {
            name: def.name.clone(),
            description: def.description.clone(),
            total: 0.0,
        })
        .collect();
    let mut sign_locks: Vec<Option<SignLockState>> = vec![None; compiled.len()];
    let mut loan_repayment_total = 0.0;
    let mut no_match_total = 0.0;
    let mut total = 0.0;

    for (idx, tx) in transactions.iter().enumerate() {
        if tx.date < period_start || tx.date > period_end {
            continue;
        }

        if internal_transfer_flags[idx] || card_payment_flags[idx] {
            continue;
        }

        if loan_repayment_flags.related[idx] && !loan_repayment_flags.counted[idx] {
            continue;
        }

        if tx.amount == 0.0 {
            continue;
        }

        let amount = tx.amount.abs();
        total += amount;

        if loan_repayment_flags.counted[idx] {
            loan_repayment_total += amount;
            continue;
        }

        let text = searchable_text(tx);
        if let Some((idx, _)) = compiled
            .iter()
            .enumerate()
            .find(|(_, def)| def.regex.is_match(&text))
        {
            if compiled[idx].lock_sign_on_first_match {
                let is_positive = tx.amount > 0.0;
                match &sign_locks[idx] {
                    None => {
                        sign_locks[idx] = Some(SignLockState {
                            expected_positive: is_positive,
                            first_file: tx.source_file.clone(),
                            first_line: tx.source_line,
                        });
                    }
                    Some(state) if state.expected_positive != is_positive => {
                        return Err(anyhow!(
                            "sign mismatch for summary '{}': first match was {} at '{}' line {}, but found {} at '{}' line {}",
                            compiled[idx].name,
                            if state.expected_positive {
                                "positive"
                            } else {
                                "negative"
                            },
                            state.first_file,
                            state.first_line,
                            if is_positive { "positive" } else { "negative" },
                            tx.source_file,
                            tx.source_line
                        ));
                    }
                    _ => {}
                }
            }
            totals[idx].total += amount;
        } else {
            no_match_total += amount;
        }
    }

    let classified_total: f64 = totals.iter().map(|item| item.total).sum();
    let expected_total = classified_total + loan_repayment_total + no_match_total;
    let epsilon = 1e-6;
    if (expected_total - total).abs() > epsilon {
        return Err(anyhow!(
            "summary totals do not add up: configured + loan_repayment + no_match = {expected_total:.2}, total = {total:.2}"
        ));
    }

    totals.push(SummaryItem {
        name: LOAN_REPAYMENT_SUMMARY_NAME.to_string(),
        description: "Total negative loan repayments".to_string(),
        total: loan_repayment_total,
    });

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

pub fn matched_transactions_for_period(
    transactions: &[Transaction],
    period_start: NaiveDate,
    period_end: NaiveDate,
    definitions: &[SummaryDefinition],
) -> Result<Vec<bool>> {
    let internal_transfer_flags = detect_internal_transfers(transactions);
    let card_payment_flags = detect_card_payments(transactions);
    let loan_repayment_flags = detect_loan_repayments(transactions);
    let all_matches = matched_transactions(transactions, definitions)?;

    let mut matched = vec![false; transactions.len()];
    for (idx, tx) in transactions.iter().enumerate() {
        if tx.date < period_start
            || tx.date > period_end
            || tx.amount == 0.0
            || internal_transfer_flags[idx]
            || card_payment_flags[idx]
            || (loan_repayment_flags.related[idx] && !loan_repayment_flags.counted[idx])
        {
            continue;
        }
        matched[idx] = all_matches[idx];
    }

    Ok(matched)
}

pub fn matched_transactions(
    transactions: &[Transaction],
    definitions: &[SummaryDefinition],
) -> Result<Vec<bool>> {
    validate_summary_definitions(definitions)?;
    let compiled = compile_summary_definitions(definitions)?;
    let internal_transfer_flags = detect_internal_transfers(transactions);
    let card_payment_flags = detect_card_payments(transactions);
    let loan_repayment_flags = detect_loan_repayments(transactions);

    let mut matched = vec![false; transactions.len()];
    for (idx, tx) in transactions.iter().enumerate() {
        if tx.amount == 0.0
            || internal_transfer_flags[idx]
            || card_payment_flags[idx]
            || (loan_repayment_flags.related[idx] && !loan_repayment_flags.counted[idx])
        {
            continue;
        }

        let text = searchable_text(tx);
        matched[idx] = loan_repayment_flags.counted[idx]
            || compiled.iter().any(|def| def.regex.is_match(&text));
    }

    Ok(matched)
}

pub fn matched_summary_names(
    transactions: &[Transaction],
    definitions: &[SummaryDefinition],
) -> Result<Vec<Option<String>>> {
    validate_summary_definitions(definitions)?;
    let compiled = compile_summary_definitions(definitions)?;
    let internal_transfer_flags = detect_internal_transfers(transactions);
    let card_payment_flags = detect_card_payments(transactions);
    let loan_repayment_flags = detect_loan_repayments(transactions);

    let mut names = vec![None; transactions.len()];
    for (idx, tx) in transactions.iter().enumerate() {
        if tx.amount == 0.0
            || internal_transfer_flags[idx]
            || card_payment_flags[idx]
            || (loan_repayment_flags.related[idx] && !loan_repayment_flags.counted[idx])
        {
            continue;
        }

        if loan_repayment_flags.counted[idx] {
            names[idx] = Some(LOAN_REPAYMENT_SUMMARY_NAME.to_string());
            continue;
        }

        let text = searchable_text(tx);
        names[idx] = compiled
            .iter()
            .find(|def| def.regex.is_match(&text))
            .map(|def| def.name.clone());
    }

    Ok(names)
}

pub fn detect_loan_repayments(transactions: &[Transaction]) -> LoanRepaymentFlags {
    let mut related = vec![false; transactions.len()];
    let mut counted = vec![false; transactions.len()];
    let mut groups: HashMap<(NaiveDate, String), Vec<usize>> = HashMap::new();

    for (idx, tx) in transactions.iter().enumerate() {
        if tx.amount == 0.0 || !looks_like_loan_repayment_candidate(tx) {
            continue;
        }

        groups
            .entry((tx.date, loan_repayment_signature(tx)))
            .or_default()
            .push(idx);
    }

    for indices in groups.values() {
        if indices.is_empty() || indices.len() > 3 {
            continue;
        }

        let has_negative = indices.iter().any(|idx| transactions[*idx].amount < 0.0);
        if !has_negative {
            continue;
        }

        for idx in indices {
            related[*idx] = true;
            if transactions[*idx].amount < 0.0 {
                counted[*idx] = true;
            }
        }
    }

    LoanRepaymentFlags { related, counted }
}

fn looks_like_loan_repayment_candidate(tx: &Transaction) -> bool {
    let fields = [
        tx.transaction_type.as_str(),
        tx.source.as_str(),
        tx.other_party.as_str(),
        tx.particulars.as_str(),
        tx.analysis_code.as_str(),
        tx.reference.as_str(),
    ];

    fields.iter().any(|value| value.to_lowercase().contains("loan repayment"))
}

fn loan_repayment_signature(tx: &Transaction) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        normalize_signature_piece(&tx.transaction_type),
        normalize_signature_piece(&tx.other_party),
        normalize_signature_piece(&tx.particulars),
        normalize_signature_piece(&tx.analysis_code),
        normalize_signature_piece(&tx.reference),
    )
}

fn normalize_signature_piece(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn detect_internal_transfers(transactions: &[Transaction]) -> Vec<bool> {
    let mut flags = vec![false; transactions.len()];
    let mut groups: HashMap<(NaiveDate, i64), Vec<usize>> = HashMap::new();

    for (idx, tx) in transactions.iter().enumerate() {
        if tx.amount == 0.0 {
            continue;
        }

        let amount_cents = (tx.amount.abs() * 100.0).round() as i64;
        let key = (tx.date, amount_cents);
        groups.entry(key).or_default().push(idx);
    }

    for indices in groups.values() {
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a_idx = indices[i];
                let b_idx = indices[j];
                let a = &transactions[a_idx];
                let b = &transactions[b_idx];

                if a.account_number == b.account_number {
                    continue;
                }
                if (a.amount > 0.0) == (b.amount > 0.0) {
                    continue;
                }

                let has_transfer_code = a.analysis_code.eq_ignore_ascii_case("TRANSFER")
                    || b.analysis_code.eq_ignore_ascii_case("TRANSFER");
                let same_reference = !a.reference.trim().is_empty()
                    && a.reference.trim().eq_ignore_ascii_case(b.reference.trim());
                let same_particulars = !a.particulars.trim().is_empty()
                    && a.particulars.trim().eq_ignore_ascii_case(b.particulars.trim());
                let payment_received_card_pair = (looks_like_payment_received(a)
                    && looks_like_card_transfer_outgoing(b))
                    || (looks_like_payment_received(b) && looks_like_card_transfer_outgoing(a));

                let from_to_pair = (looks_like_from_account_counterparty(a)
                    && looks_like_to_account_counterparty(b))
                    || (looks_like_from_account_counterparty(b)
                        && looks_like_to_account_counterparty(a));

                if payment_received_card_pair
                    || ((has_transfer_code || from_to_pair)
                        && (same_reference || same_particulars || from_to_pair))
                {
                    flags[a_idx] = true;
                    flags[b_idx] = true;
                }
            }
        }
    }

    flags
}

fn looks_like_to_account_counterparty(tx: &Transaction) -> bool {
    let text = tx.other_party.trim().to_lowercase();
    text.starts_with("to ") && text.contains('-') && text.chars().any(|ch| ch.is_ascii_digit())
}

fn looks_like_from_account_counterparty(tx: &Transaction) -> bool {
    let text = tx.other_party.trim().to_lowercase();
    text.starts_with("from ") && text.contains('-') && text.chars().any(|ch| ch.is_ascii_digit())
}

pub fn detect_card_payments(transactions: &[Transaction]) -> Vec<bool> {
    let mut flags = vec![false; transactions.len()];
    let mut groups: HashMap<(NaiveDate, i64), Vec<usize>> = HashMap::new();

    for (idx, tx) in transactions.iter().enumerate() {
        if tx.amount == 0.0 {
            continue;
        }

        let amount_cents = (tx.amount.abs() * 100.0).round() as i64;
        groups.entry((tx.date, amount_cents)).or_default().push(idx);
    }

    for indices in groups.values() {
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a_idx = indices[i];
                let b_idx = indices[j];
                let a = &transactions[a_idx];
                let b = &transactions[b_idx];

                if a.account_number == b.account_number {
                    continue;
                }
                if (a.amount > 0.0) == (b.amount > 0.0) {
                    continue;
                }


                let looks_like_pair = (looks_like_payment_received(a) && looks_like_card_transfer_outgoing(b))
                    || (looks_like_payment_received(b) && looks_like_card_transfer_outgoing(a));
                if !looks_like_pair {
                    continue;
                }

                flags[a_idx] = true;
                flags[b_idx] = true;
            }
        }
    }

    flags
}

fn looks_like_payment_received(tx: &Transaction) -> bool {
    tx.other_party
        .to_lowercase()
        .contains("payment received")
}

fn looks_like_card_transfer_outgoing(tx: &Transaction) -> bool {
    let lower = tx.other_party.to_lowercase();
    lower.starts_with("to ") && tx.other_party.contains("****")
}

fn validate_summary_definitions(definitions: &[SummaryDefinition]) -> Result<()> {
    check_summay::check_summary_definitions(definitions)?;

    if definitions.is_empty() {
        return Err(anyhow!("summary definitions cannot be empty"));
    }

    for def in definitions {
        if def.name.trim().is_empty() {
            return Err(anyhow!("summary definition name cannot be empty"));
        }
        if def.name == NO_MATCH_SUMMARY_NAME
            || def.name == TOTAL_SUMMARY_NAME
            || def.name == LOAN_REPAYMENT_SUMMARY_NAME
        {
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
                lock_sign_on_first_match: def.lock_sign_on_first_match,
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
                source_file: "test.csv".into(),
                source_line: 2,
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
                source_file: "test.csv".into(),
                source_line: 3,
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
                source_file: "test.csv".into(),
                source_line: 4,
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
        assert_eq!(summary.items[2].name, "loan_repayment_total");
        assert_eq!(summary.items[2].total, 0.0);
        assert_eq!(summary.items[3].name, "no_match");
        assert_eq!(summary.items[3].total, 0.0);
        assert_eq!(summary.items[4].name, "total");
        assert_eq!(summary.items[4].total, 620.0);
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
            source_file: "test.csv".into(),
            source_line: 2,
        }];

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
        let definitions = default_summary_definitions();
        let summary = summarize_for_period(&txs, start, end, &definitions).unwrap();

        assert_eq!(summary.items[0].total, 0.0);
        assert_eq!(summary.items[1].total, 0.0);
        assert_eq!(summary.items[2].name, "loan_repayment_total");
        assert_eq!(summary.items[2].total, 0.0);
        assert_eq!(summary.items[3].name, "no_match");
        assert_eq!(summary.items[3].total, 75.0);
        assert_eq!(summary.items[4].name, "total");
        assert_eq!(summary.items[4].total, 75.0);
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

    #[test]
    fn marks_only_matching_transactions_for_period() {
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
                source_file: "test.csv".into(),
                source_line: 2,
            },
            Transaction {
                account_number: "1".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 3).unwrap(),
                amount: -60.0,
                transaction_code: String::new(),
                transaction_type: "AUTOMATIC PAYMENT".into(),
                source: String::new(),
                other_party: "Unknown".into(),
                particulars: String::new(),
                analysis_code: String::new(),
                reference: String::new(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504030001".into(),
                source_file: "test.csv".into(),
                source_line: 3,
            },
            Transaction {
                account_number: "1".into(),
                date: NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
                amount: -20.0,
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
                source_file: "test.csv".into(),
                source_line: 4,
            },
        ];

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
        let definitions = default_summary_definitions();
        let matched = matched_transactions_for_period(&txs, start, end, &definitions).unwrap();

        assert_eq!(matched, vec![true, false, false]);
    }

    #[test]
    fn matches_transactions_outside_period_when_requested() {
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
                source_file: "test.csv".into(),
                source_line: 2,
            },
            Transaction {
                account_number: "1".into(),
                date: NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
                amount: -20.0,
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
                source_file: "test.csv".into(),
                source_line: 3,
            },
        ];

        let definitions = default_summary_definitions();
        let matched = matched_transactions(&txs, &definitions).unwrap();

        assert_eq!(matched, vec![true, true]);
    }

    #[test]
    fn returns_matched_summary_names_in_order() {
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
                source_file: "test.csv".into(),
                source_line: 2,
            },
            Transaction {
                account_number: "1".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 3).unwrap(),
                amount: -60.0,
                transaction_code: String::new(),
                transaction_type: "AUTOMATIC PAYMENT".into(),
                source: String::new(),
                other_party: "Unknown".into(),
                particulars: String::new(),
                analysis_code: String::new(),
                reference: String::new(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504030001".into(),
                source_file: "test.csv".into(),
                source_line: 3,
            },
        ];

        let definitions = default_summary_definitions();
        let names = matched_summary_names(&txs, &definitions).unwrap();

        assert_eq!(names[0], Some("power_payments_total".to_string()));
        assert_eq!(names[1], None);
    }

    #[test]
    fn detects_and_excludes_internal_transfers_from_summary_totals() {
        let txs = vec![
            Transaction {
                account_number: "A".into(),
                date: NaiveDate::from_ymd_opt(2026, 4, 12).unwrap(),
                amount: -50.0,
                transaction_code: String::new(),
                transaction_type: "ONLINE BANKING".into(),
                source: String::new(),
                other_party: "To 1395-0292849-00".into(),
                particulars: "Lunch NewJob".into(),
                analysis_code: "TRANSFER".into(),
                reference: "12:45-753463".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202604120001".into(),
                source_file: "a.csv".into(),
                source_line: 2,
            },
            Transaction {
                account_number: "B".into(),
                date: NaiveDate::from_ymd_opt(2026, 4, 12).unwrap(),
                amount: 50.0,
                transaction_code: String::new(),
                transaction_type: "DIRECT CREDIT".into(),
                source: String::new(),
                other_party: "FRM 0406-0790348-00".into(),
                particulars: "Lunch NewJob".into(),
                analysis_code: "TRANSFER".into(),
                reference: "12:45-753463".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202604120001".into(),
                source_file: "b.csv".into(),
                source_line: 2,
            },
            Transaction {
                account_number: "A".into(),
                date: NaiveDate::from_ymd_opt(2026, 4, 13).unwrap(),
                amount: -120.0,
                transaction_code: String::new(),
                transaction_type: "AUTOMATIC PAYMENT".into(),
                source: String::new(),
                other_party: "Power Co".into(),
                particulars: String::new(),
                analysis_code: String::new(),
                reference: "abc".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202604130001".into(),
                source_file: "a.csv".into(),
                source_line: 3,
            },
        ];

        let flags = detect_internal_transfers(&txs);
        assert_eq!(flags, vec![true, true, false]);

        let start = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();
        let definitions = default_summary_definitions();
        let summary = summarize_for_period(&txs, start, end, &definitions).unwrap();

        assert_eq!(summary.items[0].name, "power_payments_total");
        assert_eq!(summary.items[0].total, 120.0);
        assert_eq!(summary.items.last().unwrap().name, "total");
        assert_eq!(summary.items.last().unwrap().total, 120.0);
    }

    #[test]
    fn detects_internal_transfer_without_transfer_analysis_code() {
        let txs = vec![
            Transaction {
                account_number: "0313950292849000".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 29).unwrap(),
                amount: 150.0,
                transaction_code: String::new(),
                transaction_type: "DIRECT CREDIT".into(),
                source: String::new(),
                other_party: "From 0406-0790348-00".into(),
                particulars: "Car pool".into(),
                analysis_code: String::new(),
                reference: "07:41-86654".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504290001".into(),
                source_file: "a.csv".into(),
                source_line: 2,
            },
            Transaction {
                account_number: "0304060790348000".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 29).unwrap(),
                amount: -150.0,
                transaction_code: String::new(),
                transaction_type: "ONLINE BANKING".into(),
                source: String::new(),
                other_party: "To 1395-0292849-00".into(),
                particulars: "Car pool".into(),
                analysis_code: String::new(),
                reference: "07:41-86654".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504290002".into(),
                source_file: "b.csv".into(),
                source_line: 2,
            },
        ];

        let flags = detect_internal_transfers(&txs);
        assert_eq!(flags, vec![true, true]);
    }

    #[test]
    fn detects_and_excludes_card_payments_from_summary_totals() {
        let txs = vec![
            Transaction {
                account_number: "CARD".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 10).unwrap(),
                amount: 355.0,
                transaction_code: String::new(),
                transaction_type: String::new(),
                source: String::new(),
                other_party: "PAYMENT RECEIVED THANK YOU NZL".into(),
                particulars: String::new(),
                analysis_code: String::new(),
                reference: "20250410".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504100001".into(),
                source_file: "card.csv".into(),
                source_line: 2,
            },
            Transaction {
                account_number: "BANK".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 10).unwrap(),
                amount: -355.0,
                transaction_code: String::new(),
                transaction_type: "ONLINE BANKING".into(),
                source: String::new(),
                other_party: "To ************2640".into(),
                particulars: "WBC Internet".into(),
                analysis_code: "TRANSFER".into(),
                reference: "12:42-04708".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504100002".into(),
                source_file: "bank.csv".into(),
                source_line: 2,
            },
            Transaction {
                account_number: "BANK".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 11).unwrap(),
                amount: -120.0,
                transaction_code: String::new(),
                transaction_type: "AUTOMATIC PAYMENT".into(),
                source: String::new(),
                other_party: "Power Co".into(),
                particulars: String::new(),
                analysis_code: String::new(),
                reference: "abc".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504110001".into(),
                source_file: "bank.csv".into(),
                source_line: 3,
            },
        ];

        let flags = detect_card_payments(&txs);
        assert_eq!(flags, vec![true, true, false]);

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
        let definitions = default_summary_definitions();
        let summary = summarize_for_period(&txs, start, end, &definitions).unwrap();

        assert_eq!(summary.items[0].name, "power_payments_total");
        assert_eq!(summary.items[0].total, 120.0);
        assert_eq!(summary.items.last().unwrap().name, "total");
        assert_eq!(summary.items.last().unwrap().total, 120.0);
    }

    #[test]
    fn detects_loan_repayments_and_counts_only_negative_rows() {
        let txs = vec![
            Transaction {
                account_number: "0304060790348000".into(),
                date: NaiveDate::from_ymd_opt(2025, 6, 21).unwrap(),
                amount: -77.0,
                transaction_code: String::new(),
                transaction_type: "LOAN REPAYMENT".into(),
                source: String::new(),
                other_party: "Loan repayment".into(),
                particulars: "0406 0".into(),
                analysis_code: String::new(),
                reference: String::new(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202506210001".into(),
                source_file: "loan.csv".into(),
                source_line: 2,
            },
            Transaction {
                account_number: "0304060790348091".into(),
                date: NaiveDate::from_ymd_opt(2025, 6, 21).unwrap(),
                amount: 2800.0,
                transaction_code: String::new(),
                transaction_type: "LOAN REPAYMENT".into(),
                source: String::new(),
                other_party: "Loan repayment".into(),
                particulars: "0406 0".into(),
                analysis_code: String::new(),
                reference: String::new(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202506210001".into(),
                source_file: "loan.csv".into(),
                source_line: 3,
            },
            Transaction {
                account_number: "0304060790348000".into(),
                date: NaiveDate::from_ymd_opt(2025, 6, 21).unwrap(),
                amount: -2800.0,
                transaction_code: String::new(),
                transaction_type: "LOAN REPAYMENT".into(),
                source: String::new(),
                other_party: "Loan repayment".into(),
                particulars: "0406 0".into(),
                analysis_code: String::new(),
                reference: String::new(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202506210002".into(),
                source_file: "loan.csv".into(),
                source_line: 4,
            },
        ];

        let flags = detect_loan_repayments(&txs);
        assert_eq!(flags.related, vec![true, true, true]);
        assert_eq!(flags.counted, vec![true, false, true]);

        let defs = default_summary_definitions();
        let names = matched_summary_names(&txs, &defs).unwrap();
        assert_eq!(names, vec![Some("loan_repayment_total".into()), None, Some("loan_repayment_total".into())]);

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 6, 30).unwrap();
        let summary = summarize_for_period(&txs, start, end, &defs).unwrap();

        assert_eq!(summary.items[0].name, "power_payments_total");
        assert_eq!(summary.items[1].name, "mortgage_interest_total");
        assert_eq!(summary.items[2].name, "loan_repayment_total");
        assert_eq!(summary.items[2].total, 2877.0);
        assert_eq!(summary.items[3].name, "no_match");
        assert_eq!(summary.items[3].total, 0.0);
        assert_eq!(summary.items[4].name, "total");
        assert_eq!(summary.items[4].total, 2877.0);
    }

    #[test]
    fn detects_payment_received_card_charge_without_reference_match() {
        let txs = vec![
            Transaction {
                account_number: "0000000003071972735".into(),
                date: NaiveDate::from_ymd_opt(2025, 9, 10).unwrap(),
                amount: 1200.0,
                transaction_code: String::new(),
                transaction_type: String::new(),
                source: String::new(),
                other_party: "PAYMENT RECEIVED       THANK YOU     NZL".into(),
                particulars: String::new(),
                analysis_code: String::new(),
                reference: "20250910".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202509100001".into(),
                source_file: "card.csv".into(),
                source_line: 46,
            },
            Transaction {
                account_number: "0304060790348000".into(),
                date: NaiveDate::from_ymd_opt(2025, 9, 10).unwrap(),
                amount: -1200.0,
                transaction_code: String::new(),
                transaction_type: "ONLINE BANKING".into(),
                source: String::new(),
                other_party: "To ************2640".into(),
                particulars: "Fly Melbourn".into(),
                analysis_code: String::new(),
                reference: "21:23-90724".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202509100002".into(),
                source_file: "bank.csv".into(),
                source_line: 96,
            },
        ];

        let flags = detect_card_payments(&txs);
        assert_eq!(flags, vec![true, true]);

        let defs = default_summary_definitions();
        let names = matched_summary_names(&txs, &defs).unwrap();
        assert_eq!(names, vec![None, None]);
    }

    #[test]
    fn errors_when_summary_matches_mixed_signs() {
        let txs = vec![
            Transaction {
                account_number: "1".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 2).unwrap(),
                amount: -120.0,
                transaction_code: String::new(),
                transaction_type: "PAYMENT".into(),
                source: String::new(),
                other_party: "Power Co".into(),
                particulars: String::new(),
                analysis_code: String::new(),
                reference: String::new(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504020001".into(),
                source_file: "file_a.csv".into(),
                source_line: 8,
            },
            Transaction {
                account_number: "1".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 3).unwrap(),
                amount: 90.0,
                transaction_code: String::new(),
                transaction_type: "PAYMENT".into(),
                source: String::new(),
                other_party: "Power Co".into(),
                particulars: String::new(),
                analysis_code: String::new(),
                reference: String::new(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504030001".into(),
                source_file: "file_b.csv".into(),
                source_line: 14,
            },
        ];

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
        let definitions = vec![SummaryDefinition {
            name: "power_payments_total".into(),
            description: String::new(),
            regex: "power".into(),
            color: None,
            lock_sign_on_first_match: true,
        }];
        let err = summarize_for_period(&txs, start, end, &definitions)
            .unwrap_err()
            .to_string();

        assert!(err.contains("sign mismatch for summary 'power_payments_total'"));
        assert!(err.contains("'file_a.csv' line 8"));
        assert!(err.contains("'file_b.csv' line 14"));
    }
}
