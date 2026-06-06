use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use regex::{Regex, RegexBuilder};
use std::collections::HashMap;

use crate::Transaction;

use super::types::{
    SignReversalWarning, Summary, SummaryDefinition, SummaryItem, TransactionClass,
};

pub(crate) const NO_MATCH_SUMMARY_NAME: &str = "no_match";
pub(crate) const TOTAL_SUMMARY_NAME: &str = "total";
pub(crate) const LOAN_REPAYMENT_SUMMARY_NAME: &str = "loan_repayment_total";

struct CompiledSummaryDefinition {
    name: String,
    description: String,
    regex: Regex,
    lock_sign_on_first_match: bool,
    income: bool,
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
                    income: def.income,
                    color: def.color.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { compiled })
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // reserved for future richer sign-reversal error messages
struct SignLockState {
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
    /// Assign `summary_name` and `is_sign_reversed` on every transaction.
    ///
    /// Runs on the full dataset (not just the period) so the Transactions sheet
    /// shows names for all rows. Returns a warning for each sign reversal found.
    /// Returns `Err` if a sign-locked summary's very first match is a credit —
    /// that strongly indicates a misconfigured regex.
    pub fn annotate(&self, transactions: &mut [Transaction]) -> Result<Vec<SignReversalWarning>> {
        let compiled = &self.compiled;
        let mut sign_locks: Vec<Option<SignLockState>> = vec![None; compiled.len()];
        let mut warnings = Vec::new();

        for tx in transactions.iter_mut() {
            if tx.amount == 0.0
                || matches!(
                    tx.class,
                    TransactionClass::InternalTransfer
                        | TransactionClass::CardPayment
                        | TransactionClass::LoanRepaymentOnly
                        | TransactionClass::LoanRepaymentCounted
                )
            {
                continue;
            }

            let text = searchable_text(tx);
            if let Some((idx, def)) = compiled
                .iter()
                .enumerate()
                .find(|(_, def)| def.regex.is_match(&text))
            {
                tx.summary_name = Some(def.name.clone());

                if def.lock_sign_on_first_match {
                    let is_positive = tx.amount > 0.0;
                    // For income summaries the expected sign is positive; for
                    // expense summaries it is negative.
                    let expected_positive = def.income;
                    let is_sign_ok = is_positive == expected_positive;
                    match &sign_locks[idx] {
                        None if !is_sign_ok => {
                            let (expected_label, actual_label) = if expected_positive {
                                ("income", "debit (negative)")
                            } else {
                                ("expense", "credit (positive)")
                            };
                            return Err(anyhow!(
                                "misconfigured summary '{}': first matched transaction is a {} at '{}' line {}; {} summaries expect {} — check your regex or add `income: true` to the definition",
                                def.name,
                                actual_label,
                                tx.source_file,
                                tx.source_line,
                                expected_label,
                                if expected_positive {
                                    "credits"
                                } else {
                                    "debits"
                                },
                            ));
                        }
                        None => {
                            sign_locks[idx] = Some(SignLockState {
                                first_file: tx.source_file.clone(),
                                first_line: tx.source_line,
                            });
                        }
                        Some(_) if !is_sign_ok => {
                            tx.is_sign_reversed = true;
                            warnings.push(SignReversalWarning {
                                summary_name: def.name.clone(),
                                source_file: tx.source_file.clone(),
                                source_line: tx.source_line,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(warnings)
    }

    /// Accumulate totals for transactions inside the period.
    ///
    /// Reads `tx.summary_name` and `tx.is_sign_reversed` set by `annotate`; does
    /// no regex matching of its own. Sign-reversed transactions are subtracted from
    /// their category total (net effect, e.g. a store return reduces hardware spend).
    pub fn summarize_for_period(
        &self,
        transactions: &[Transaction],
        period_start: NaiveDate,
        period_end: NaiveDate,
    ) -> Result<Summary> {
        let compiled = &self.compiled;

        let name_to_idx: HashMap<&str, usize> = compiled
            .iter()
            .enumerate()
            .map(|(i, def)| (def.name.as_str(), i))
            .collect();

        let mut totals: Vec<SummaryItem> = compiled
            .iter()
            .map(|def| SummaryItem {
                name: def.name.clone(),
                description: def.description.clone(),
                total: 0.0,
            })
            .collect();

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

            if tx.class == TransactionClass::LoanRepaymentCounted {
                loan_repayment_total += amount;
                total += amount;
                continue;
            }

            if let Some(name) = &tx.summary_name {
                if let Some(&idx) = name_to_idx.get(name.as_str()) {
                    let effective = if tx.is_sign_reversed { -amount } else { amount };
                    totals[idx].total += effective;
                    total += effective;
                }
            } else {
                no_match_total += amount;
                total += amount;
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
    use chrono::NaiveDate;

    fn def(name: &str, regex: &str) -> SummaryDefinition {
        SummaryDefinition {
            name: name.to_string(),
            description: String::new(),
            regex: regex.to_string(),
            color: None,
            lock_sign_on_first_match: true,
            income: false,
        }
    }

    fn def_income(name: &str, regex: &str) -> SummaryDefinition {
        SummaryDefinition {
            income: true,
            ..def(name, regex)
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
            transaction_type: transaction_type.into(),
            other_party: other_party.into(),
            particulars: particulars.into(),
            unique_id: unique_id.into(),
            source_file: "test.csv".into(),
            source_line: 2,
            ..Default::default()
        }
    }

    #[test]
    fn summarizes_matching_categories_for_apr_to_may_window() {
        let mut txs = vec![
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
        let compiled = CompiledSummarySet::compile(&definitions).unwrap();
        compiled.annotate(&mut txs).unwrap();
        let summary = compiled.summarize_for_period(&txs, start, end).unwrap();

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
    fn annotate_sets_summary_name_on_all_transactions() {
        let mut txs = vec![
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
                "Unknown",
                "",
                "u2",
            ),
        ];
        let compiled = CompiledSummarySet::compile(&default_summary_definitions()).unwrap();
        let warnings = compiled.annotate(&mut txs).unwrap();
        assert_eq!(
            txs[0].summary_name,
            Some("power_payments_total".to_string())
        );
        assert_eq!(txs[1].summary_name, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn tracks_unmatched_transactions_in_no_match() {
        let mut txs = vec![tx(
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
        let compiled = CompiledSummarySet::compile(&definitions).unwrap();
        compiled.annotate(&mut txs).unwrap();
        let summary = compiled.summarize_for_period(&txs, start, end).unwrap();

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
    fn sign_reversal_sets_flag_and_nets_total() {
        let mut t1 = tx(
            "1",
            NaiveDate::from_ymd_opt(2025, 4, 2).unwrap(),
            -150.0,
            "PAYMENT",
            "Bunnings",
            "",
            "u1",
        );
        t1.source_file = "file_a.csv".into();
        t1.source_line = 6;
        let mut t2 = tx(
            "1",
            NaiveDate::from_ymd_opt(2025, 4, 10).unwrap(),
            20.0,
            "CREDIT",
            "Bunnings",
            "",
            "u2",
        );
        t2.source_file = "file_b.csv".into();
        t2.source_line = 112;

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 5, 31).unwrap();
        let definitions = vec![def("hardware_home_total", "bunnings")];
        let compiled = CompiledSummarySet::compile(&definitions).unwrap();
        let mut txs = vec![t1, t2];
        let warnings = compiled.annotate(&mut txs).unwrap();

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].summary_name, "hardware_home_total");
        assert_eq!(warnings[0].source_file, "file_b.csv");
        assert_eq!(warnings[0].source_line, 112);
        assert!(!txs[0].is_sign_reversed);
        assert!(txs[1].is_sign_reversed);

        let summary = compiled.summarize_for_period(&txs, start, end).unwrap();
        assert_eq!(summary.items[0].name, "hardware_home_total");
        assert_eq!(summary.items[0].total, 130.0); // 150 - 20 net
    }

    #[test]
    fn first_positive_match_is_fatal() {
        let mut t1 = tx(
            "1",
            NaiveDate::from_ymd_opt(2025, 4, 2).unwrap(),
            90.0,
            "CREDIT",
            "Power Co",
            "",
            "u1",
        );
        t1.source_file = "file_a.csv".into();
        t1.source_line = 8;

        let definitions = vec![def("power_payments_total", "power")];
        let err = CompiledSummarySet::compile(&definitions)
            .unwrap()
            .annotate(&mut [t1])
            .unwrap_err()
            .to_string();

        assert!(err.contains("misconfigured summary 'power_payments_total'"));
        assert!(err.contains("'file_a.csv' line 8"));
    }

    #[test]
    fn income_summary_accepts_positive_and_warns_on_negative() {
        let mut txs = vec![
            {
                let mut t = tx(
                    "1",
                    NaiveDate::from_ymd_opt(2025, 4, 25).unwrap(),
                    5000.0,
                    "CREDIT",
                    "Employer Ltd",
                    "",
                    "u1",
                );
                t.source_file = "main.csv".into();
                t.source_line = 8;
                t
            },
            {
                let mut t = tx(
                    "1",
                    NaiveDate::from_ymd_opt(2025, 4, 26).unwrap(),
                    -50.0,
                    "DEBIT",
                    "Employer Ltd",
                    "",
                    "u2",
                );
                t.source_file = "main.csv".into();
                t.source_line = 9;
                t
            },
        ];

        let definitions = vec![def_income("salary_total", "employer")];
        let compiled = CompiledSummarySet::compile(&definitions).unwrap();
        let warnings = compiled.annotate(&mut txs).unwrap();

        assert!(!txs[0].is_sign_reversed);
        assert!(txs[1].is_sign_reversed);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].summary_name, "salary_total");
        assert_eq!(warnings[0].source_line, 9);

        let start = NaiveDate::from_ymd_opt(2025, 4, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
        let summary = compiled.summarize_for_period(&txs, start, end).unwrap();
        assert_eq!(summary.items[0].total, 4950.0); // 5000 salary - 50 clawback
    }

    #[test]
    fn income_summary_fatal_on_first_negative() {
        let mut t = tx(
            "1",
            NaiveDate::from_ymd_opt(2025, 4, 2).unwrap(),
            -50.0,
            "DEBIT",
            "Employer Ltd",
            "",
            "u1",
        );
        t.source_file = "main.csv".into();
        t.source_line = 5;

        let definitions = vec![def_income("salary_total", "employer")];
        let err = CompiledSummarySet::compile(&definitions)
            .unwrap()
            .annotate(&mut [t])
            .unwrap_err()
            .to_string();

        assert!(err.contains("misconfigured summary 'salary_total'"));
        assert!(err.contains("'main.csv' line 5"));
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
