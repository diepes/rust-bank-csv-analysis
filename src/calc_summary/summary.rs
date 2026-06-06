use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use regex::{Regex, RegexBuilder};

use crate::Transaction;

use super::types::{Summary, SummaryDefinition, SummaryItem, TransactionClass};

pub(crate) const NO_MATCH_SUMMARY_NAME: &str = "no_match";
pub(crate) const TOTAL_SUMMARY_NAME: &str = "total";
pub(crate) const LOAN_REPAYMENT_SUMMARY_NAME: &str = "loan_repayment_total";

struct CompiledSummaryDefinition {
    name: String,
    description: String,
    regex: Regex,
    lock_sign_on_first_match: bool,
    color: Option<String>,
}

pub struct CompiledSummarySet {
    compiled: Vec<CompiledSummaryDefinition>,
}

impl CompiledSummarySet {
    pub fn compile(definitions: &[SummaryDefinition]) -> Result<Self> {
        validate_summary_definitions(definitions)?;
        let compiled = definitions
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
                    color: def.color.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { compiled })
    }
}

#[derive(Debug, Clone)]
struct SignLockState {
    expected_positive: bool,
    first_file: String,
    first_line: usize,
}

pub(crate) fn validate_summary_definitions(definitions: &[SummaryDefinition]) -> Result<()> {
    use std::collections::HashSet;

    if definitions.is_empty() {
        return Err(anyhow!("summary definitions cannot be empty"));
    }

    let mut seen_names = HashSet::new();
    let mut seen_regex = HashSet::new();

    for def in definitions {
        let name = def.name.trim();
        let regex_text = def.regex.trim();

        if name.is_empty() {
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
        if regex_text.is_empty() {
            return Err(anyhow!(
                "summary definition regex cannot be empty for '{}'",
                def.name
            ));
        }

        if !seen_names.insert(name.to_lowercase()) {
            return Err(anyhow!("duplicate summary definition name '{}'", def.name));
        }
        if !seen_regex.insert(regex_text.to_lowercase()) {
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

impl CompiledSummarySet {
    pub fn summarize_for_period(
        &self,
        transactions: &[Transaction],
        period_start: NaiveDate,
        period_end: NaiveDate,
    ) -> Result<Summary> {
        let compiled = &self.compiled;
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

        for tx in transactions {
            if tx.date < period_start || tx.date > period_end {
                continue;
            }

            if matches!(
                tx.class,
                TransactionClass::InternalTransfer
                    | TransactionClass::CardPayment
                    | TransactionClass::LoanRepaymentOnly
            ) {
                continue;
            }

            if tx.amount == 0.0 {
                continue;
            }

            let amount = tx.amount.abs();
            total += amount;

            if tx.class == TransactionClass::LoanRepaymentCounted {
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

    pub fn matched_transactions(&self, transactions: &[Transaction]) -> Vec<bool> {
        let compiled = &self.compiled;
        let mut matched = vec![false; transactions.len()];
        for (idx, tx) in transactions.iter().enumerate() {
            if tx.amount == 0.0
                || matches!(
                    tx.class,
                    TransactionClass::InternalTransfer
                        | TransactionClass::CardPayment
                        | TransactionClass::LoanRepaymentOnly
                )
            {
                continue;
            }

            let text = searchable_text(tx);
            matched[idx] = tx.class == TransactionClass::LoanRepaymentCounted
                || compiled.iter().any(|def| def.regex.is_match(&text));
        }
        matched
    }

    pub fn matched_transactions_for_period(
        &self,
        transactions: &[Transaction],
        period_start: NaiveDate,
        period_end: NaiveDate,
    ) -> Vec<bool> {
        let all_matches = self.matched_transactions(transactions);
        let mut matched = vec![false; transactions.len()];
        for (idx, tx) in transactions.iter().enumerate() {
            if tx.date < period_start
                || tx.date > period_end
                || tx.amount == 0.0
                || matches!(
                    tx.class,
                    TransactionClass::InternalTransfer
                        | TransactionClass::CardPayment
                        | TransactionClass::LoanRepaymentOnly
                )
            {
                continue;
            }
            matched[idx] = all_matches[idx];
        }
        matched
    }

    pub fn matched_summary_names(&self, transactions: &[Transaction]) -> Vec<Option<String>> {
        let compiled = &self.compiled;
        let mut names = vec![None; transactions.len()];
        for (idx, tx) in transactions.iter().enumerate() {
            if tx.amount == 0.0
                || matches!(
                    tx.class,
                    TransactionClass::InternalTransfer
                        | TransactionClass::CardPayment
                        | TransactionClass::LoanRepaymentOnly
                )
            {
                continue;
            }

            if tx.class == TransactionClass::LoanRepaymentCounted {
                names[idx] = Some(LOAN_REPAYMENT_SUMMARY_NAME.to_string());
                continue;
            }

            let text = searchable_text(tx);
            names[idx] = compiled
                .iter()
                .find(|def| def.regex.is_match(&text))
                .map(|def| def.name.clone());
        }
        names
    }
    pub fn color_map(&self) -> Vec<(String, u32)> {
        self.compiled
            .iter()
            .filter_map(|def| {
                def.color
                    .as_deref()
                    .and_then(|v| parse_summary_color(v).ok())
                    .map(|rgb| (def.name.clone(), rgb))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Transaction;
    use crate::calc_summary::config::default_summary_definitions;
    use crate::calc_summary::types::TransactionClass;
    use chrono::NaiveDate;

    fn def(name: &str, regex: &str) -> SummaryDefinition {
        SummaryDefinition {
            name: name.to_string(),
            description: String::new(),
            regex: regex.to_string(),
            color: None,
            lock_sign_on_first_match: true,
        }
    }

    fn tx(
        account: &str,
        date: NaiveDate,
        amount: f64,
        transaction_type: &str,
        other_party: &str,
        particulars: &str,
        unique_id: &str,
    ) -> Transaction {
        Transaction {
            account_number: account.into(),
            date,
            amount,
            transaction_code: String::new(),
            transaction_type: transaction_type.into(),
            source: String::new(),
            other_party: other_party.into(),
            particulars: particulars.into(),
            analysis_code: String::new(),
            reference: String::new(),
            serial_number: String::new(),
            account_code: String::new(),
            unique_id: unique_id.into(),
            source_file: "test.csv".into(),
            source_line: 2,
            class: TransactionClass::Countable,
        }
    }

    #[test]
    fn summarizes_matching_categories_for_apr_to_may_window() {
        let txs = vec![
            tx(
                "1",
                NaiveDate::from_ymd_opt(2025, 4, 2).unwrap(),
                -120.0,
                "AUTOMATIC PAYMENT",
                "Power Co",
                "",
                "u1",
            ),
            tx(
                "1",
                NaiveDate::from_ymd_opt(2025, 5, 15).unwrap(),
                -500.0,
                "AUTOMATIC PAYMENT",
                "ABC Mortgage",
                "mortgage interest",
                "u2",
            ),
            tx(
                "1",
                NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
                -220.0,
                "AUTOMATIC PAYMENT",
                "Power Co",
                "",
                "u3",
            ),
        ];

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
        let definitions = default_summary_definitions();
        let summary = CompiledSummarySet::compile(&definitions)
            .unwrap()
            .summarize_for_period(&txs, start, end)
            .unwrap();

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
        let txs = vec![tx(
            "1",
            NaiveDate::from_ymd_opt(2025, 4, 10).unwrap(),
            -75.0,
            "PAYMENT",
            "Unknown Vendor",
            "misc expense",
            "u1",
        )];

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
        let definitions = default_summary_definitions();
        let summary = CompiledSummarySet::compile(&definitions)
            .unwrap()
            .summarize_for_period(&txs, start, end)
            .unwrap();

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
    fn marks_only_matching_transactions_for_period() {
        let txs = vec![
            tx(
                "1",
                NaiveDate::from_ymd_opt(2025, 4, 2).unwrap(),
                -120.0,
                "AUTOMATIC PAYMENT",
                "Power Co",
                "",
                "u1",
            ),
            tx(
                "1",
                NaiveDate::from_ymd_opt(2025, 4, 3).unwrap(),
                -60.0,
                "AUTOMATIC PAYMENT",
                "Unknown",
                "",
                "u2",
            ),
            tx(
                "1",
                NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
                -20.0,
                "AUTOMATIC PAYMENT",
                "Power Co",
                "",
                "u3",
            ),
        ];

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
        let definitions = default_summary_definitions();
        let matched = CompiledSummarySet::compile(&definitions)
            .unwrap()
            .matched_transactions_for_period(&txs, start, end);

        assert_eq!(matched, vec![true, false, false]);
    }

    #[test]
    fn matches_transactions_outside_period_when_requested() {
        let txs = vec![
            tx(
                "1",
                NaiveDate::from_ymd_opt(2025, 4, 2).unwrap(),
                -120.0,
                "AUTOMATIC PAYMENT",
                "Power Co",
                "",
                "u1",
            ),
            tx(
                "1",
                NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
                -20.0,
                "AUTOMATIC PAYMENT",
                "Power Co",
                "",
                "u2",
            ),
        ];

        let definitions = default_summary_definitions();
        let matched = CompiledSummarySet::compile(&definitions)
            .unwrap()
            .matched_transactions(&txs);

        assert_eq!(matched, vec![true, true]);
    }

    #[test]
    fn returns_matched_summary_names_in_order() {
        let txs = vec![
            tx(
                "1",
                NaiveDate::from_ymd_opt(2025, 4, 2).unwrap(),
                -120.0,
                "AUTOMATIC PAYMENT",
                "Power Co",
                "",
                "u1",
            ),
            tx(
                "1",
                NaiveDate::from_ymd_opt(2025, 4, 3).unwrap(),
                -60.0,
                "AUTOMATIC PAYMENT",
                "Unknown",
                "",
                "u2",
            ),
        ];

        let definitions = default_summary_definitions();
        let names = CompiledSummarySet::compile(&definitions)
            .unwrap()
            .matched_summary_names(&txs);

        assert_eq!(names[0], Some("power_payments_total".to_string()));
        assert_eq!(names[1], None);
    }

    #[test]
    fn errors_when_summary_matches_mixed_signs() {
        let mut t1 = tx(
            "1",
            NaiveDate::from_ymd_opt(2025, 4, 2).unwrap(),
            -120.0,
            "PAYMENT",
            "Power Co",
            "",
            "u1",
        );
        t1.source_file = "file_a.csv".into();
        t1.source_line = 8;
        let mut t2 = tx(
            "1",
            NaiveDate::from_ymd_opt(2025, 4, 3).unwrap(),
            90.0,
            "PAYMENT",
            "Power Co",
            "",
            "u2",
        );
        t2.source_file = "file_b.csv".into();
        t2.source_line = 14;

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
        let definitions = vec![def("power_payments_total", "power")];
        let err = CompiledSummarySet::compile(&definitions)
            .unwrap()
            .summarize_for_period(&[t1, t2], start, end)
            .unwrap_err()
            .to_string();

        assert!(err.contains("sign mismatch for summary 'power_payments_total'"));
        assert!(err.contains("'file_a.csv' line 8"));
        assert!(err.contains("'file_b.csv' line 14"));
    }

    #[test]
    fn rejects_duplicate_names() {
        let defs = vec![def("groceries", "shop"), def("Groceries", "food")];
        let err = validate_summary_definitions(&defs).unwrap_err().to_string();
        assert!(err.contains("duplicate summary definition name"));
    }

    #[test]
    fn rejects_duplicate_regex() {
        let defs = vec![def("a", "power"), def("b", "POWER")];
        let err = validate_summary_definitions(&defs).unwrap_err().to_string();
        assert!(err.contains("duplicate summary regex"));
    }

    #[test]
    fn rejects_empty_matching_regex() {
        let defs = vec![def("groceries", "new world|")];
        let err = validate_summary_definitions(&defs).unwrap_err().to_string();
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
