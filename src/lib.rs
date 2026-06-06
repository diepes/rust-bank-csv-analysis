use std::path::Path;

use anyhow::{Context, Result, anyhow};
use chrono::{Datelike, Local, NaiveDate};
use csv::ReaderBuilder;
use serde::Deserialize;

pub mod calc_summary;
pub mod xlsx;

pub use calc_summary::{
    CompiledSummarySet, Summary, SummaryDefinition, TransactionClass, classify_transactions,
    default_summary_definitions, detect_card_payments, detect_internal_transfers,
    detect_loan_repayments, load_summary_definitions, parse_summary_color,
};
pub use xlsx::write_xlsx;

const DATE_FMT: &str = "%Y%m%d";

#[derive(Debug, Clone)]
pub struct Transaction {
    pub account_number: String,
    pub date: NaiveDate,
    pub amount: f64,
    pub transaction_code: String,
    pub transaction_type: String,
    pub source: String,
    pub other_party: String,
    pub particulars: String,
    pub analysis_code: String,
    pub reference: String,
    pub serial_number: String,
    pub account_code: String,
    pub unique_id: String,
    pub source_file: String,
    pub source_line: usize,
    /// Assigned by `classify_transactions` after loading and sorting.
    pub class: TransactionClass,
}

impl Default for Transaction {
    fn default() -> Self {
        Self {
            account_number: String::new(),
            date: NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid epoch date"),
            amount: 0.0,
            transaction_code: String::new(),
            transaction_type: String::new(),
            source: String::new(),
            other_party: String::new(),
            particulars: String::new(),
            analysis_code: String::new(),
            reference: String::new(),
            serial_number: String::new(),
            account_code: String::new(),
            unique_id: String::new(),
            source_file: String::new(),
            source_line: 0,
            class: TransactionClass::Countable,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawTransaction {
    #[serde(rename = "Account Number")]
    account_number: String,
    #[serde(rename = "Date")]
    date: String,
    #[serde(rename = "Amount")]
    amount: String,
    #[serde(rename = "Transaction Code", default)]
    transaction_code: String,
    #[serde(rename = "Transaction Type", default)]
    transaction_type: String,
    #[serde(rename = "Source", default)]
    source: String,
    #[serde(rename = "Other Party", default)]
    other_party: String,
    #[serde(rename = "Particulars", default)]
    particulars: String,
    #[serde(rename = "Analysis (Code)", default)]
    analysis_code: String,
    #[serde(rename = "Reference", default)]
    reference: String,
    #[serde(rename = "Serial Number", default)]
    serial_number: String,
    #[serde(rename = "Account Code", default)]
    account_code: String,
    #[serde(rename = "Unique ID", default)]
    unique_id: String,
}

impl TryFrom<RawTransaction> for Transaction {
    type Error = anyhow::Error;

    fn try_from(value: RawTransaction) -> Result<Self> {
        let date = NaiveDate::parse_from_str(value.date.trim(), DATE_FMT)
            .with_context(|| format!("invalid date '{}'", value.date))?;
        let amount = value
            .amount
            .trim()
            .parse::<f64>()
            .with_context(|| format!("invalid amount '{}'", value.amount))?;

        Ok(Self {
            account_number: value.account_number,
            date,
            amount,
            transaction_code: value.transaction_code,
            transaction_type: value.transaction_type,
            source: value.source,
            other_party: value.other_party,
            particulars: value.particulars,
            analysis_code: value.analysis_code,
            reference: value.reference,
            serial_number: value.serial_number,
            account_code: value.account_code,
            unique_id: value.unique_id,
            source_file: String::new(),
            source_line: 0,
            class: TransactionClass::Countable,
        })
    }
}

pub fn read_transactions(paths: &[impl AsRef<Path>]) -> Result<Vec<Transaction>> {
    let mut all = Vec::new();

    for path in paths {
        let path_ref = path.as_ref();
        let mut reader = ReaderBuilder::new()
            .trim(csv::Trim::All)
            .from_path(path_ref)
            .with_context(|| format!("failed reading CSV '{}'", path_ref.display()))?;

        for (row_idx, row) in reader.records().enumerate() {
            let line_number = row_idx + 2;
            let record = row.with_context(|| {
                format!(
                    "failed parsing row at line {} in CSV '{}'",
                    line_number,
                    path_ref.to_string_lossy()
                )
            })?;
            let raw: RawTransaction = record.deserialize(None).with_context(|| {
                format!(
                    "failed parsing row at line {} in CSV '{}'",
                    line_number,
                    path_ref.to_string_lossy()
                )
            })?;
            let mut tx = Transaction::try_from(raw).with_context(|| {
                format!(
                    "failed parsing row at line {} in CSV '{}'",
                    line_number,
                    path_ref.to_string_lossy()
                )
            })?;
            tx.source_file = path_ref.display().to_string();
            tx.source_line = line_number;

            all.push(tx);
        }
    }

    all.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| a.unique_id.cmp(&b.unique_id))
    });
    classify_transactions(&mut all);
    Ok(all)
}

pub fn nz_period_for_year(start_year: i32) -> Result<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::from_ymd_opt(start_year, 4, 1)
        .ok_or_else(|| anyhow!("invalid start year {start_year}"))?;
    let end =
        NaiveDate::from_ymd_opt(start_year, 5, 31).ok_or_else(|| anyhow!("invalid end date"))?;
    Ok((start, end))
}

pub fn latest_full_tax_year_start() -> i32 {
    latest_full_tax_year_start_for_date(Local::now().date_naive())
}

pub fn latest_full_tax_year_start_for_date(today: NaiveDate) -> i32 {
    let current_year_apr_1 =
        NaiveDate::from_ymd_opt(today.year(), 4, 1).expect("valid date for 1 April");

    if today >= current_year_apr_1 {
        today.year() - 1
    } else {
        today.year() - 2
    }
}

pub fn resolve_summary_definitions(
    summary_config_path: Option<impl AsRef<Path>>,
) -> Result<Vec<SummaryDefinition>> {
    if let Some(path) = summary_config_path {
        return load_summary_definitions(path);
    }

    let default_path = Path::new("summary.yaml");
    if default_path.exists() {
        return load_summary_definitions(default_path);
    }

    Ok(default_summary_definitions())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn latest_full_tax_year_start_after_april_first() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 6).unwrap();
        assert_eq!(latest_full_tax_year_start_for_date(date), 2025);
    }

    #[test]
    fn latest_full_tax_year_start_before_april_first() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
        assert_eq!(latest_full_tax_year_start_for_date(date), 2024);
    }

    #[test]
    fn latest_full_tax_year_start_on_april_first() {
        let date = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        assert_eq!(latest_full_tax_year_start_for_date(date), 2025);
    }

    #[test]
    fn read_transactions_allows_mixed_signs_per_file() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mixed-signs-{unique}.csv"));
        let csv = "Account Number,Date,Amount,Transaction Code,Transaction Type,Source,Other Party,Particulars,Analysis (Code),Reference,Serial Number,Account Code,Unique ID\n1,20250401,-10.00,,,SRC,Vendor A,,,,,,u1\n1,20250402,20.00,,,SRC,Vendor B,,,,,,u2\n";

        fs::write(&path, csv).unwrap();
        let txs = read_transactions(&[path.as_path()]).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(txs.len(), 2);
    }

    #[test]
    fn read_transactions_includes_all_rows_from_all_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path_a = std::env::temp_dir().join(format!("all-files-a-{unique}.csv"));
        let path_b = std::env::temp_dir().join(format!("all-files-b-{unique}.csv"));

        let csv_a = "Account Number,Date,Amount,Transaction Code,Transaction Type,Source,Other Party,Particulars,Analysis (Code),Reference,Serial Number,Account Code,Unique ID\n1,20250401,-10.00,,,SRC,Vendor A,,,,,,a1\n1,20250402,20.00,,,SRC,Vendor B,,,,,,a2\n";
        let csv_b = "Account Number,Date,Amount,Transaction Code,Transaction Type,Source,Other Party,Particulars,Analysis (Code),Reference,Serial Number,Account Code,Unique ID\n2,20250403,-30.00,,,SRC,Vendor C,,,,,,b1\n2,20250404,40.00,,,SRC,Vendor D,,,,,,b2\n";

        fs::write(&path_a, csv_a).unwrap();
        fs::write(&path_b, csv_b).unwrap();
        let txs = read_transactions(&[path_a.as_path(), path_b.as_path()]).unwrap();
        fs::remove_file(&path_a).unwrap();
        fs::remove_file(&path_b).unwrap();

        assert_eq!(txs.len(), 4);
        assert!(txs.iter().any(|tx| tx.unique_id == "a1"));
        assert!(txs.iter().any(|tx| tx.unique_id == "a2"));
        assert!(txs.iter().any(|tx| tx.unique_id == "b1"));
        assert!(txs.iter().any(|tx| tx.unique_id == "b2"));
        assert!(
            txs.iter()
                .all(|tx| tx.source_line == 2 || tx.source_line == 3)
        );
    }
}
