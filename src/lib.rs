use std::path::Path;

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use csv::ReaderBuilder;
use rust_xlsxwriter::Workbook;
use serde::Deserialize;

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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub power_payments_total: f64,
    pub mortgage_interest_total: f64,
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

        for row in reader.deserialize::<RawTransaction>() {
            let raw = row.with_context(|| {
                format!("failed parsing row in CSV '{}'", path_ref.to_string_lossy())
            })?;
            all.push(Transaction::try_from(raw)?);
        }
    }

    all.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| a.unique_id.cmp(&b.unique_id))
    });
    Ok(all)
}

pub fn summarize_for_period(
    transactions: &[Transaction],
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Summary {
    let mut summary = Summary {
        power_payments_total: 0.0,
        mortgage_interest_total: 0.0,
    };

    for tx in transactions
        .iter()
        .filter(|tx| tx.date >= period_start && tx.date <= period_end)
    {
        if tx.amount >= 0.0 {
            continue;
        }
        if is_power_payment(tx) {
            summary.power_payments_total += -tx.amount;
        }
        if is_mortgage_interest(tx) {
            summary.mortgage_interest_total += -tx.amount;
        }
    }

    summary
}

pub fn nz_period_for_year(start_year: i32) -> Result<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::from_ymd_opt(start_year, 4, 1)
        .ok_or_else(|| anyhow!("invalid start year {start_year}"))?;
    let end =
        NaiveDate::from_ymd_opt(start_year, 5, 31).ok_or_else(|| anyhow!("invalid end date"))?;
    Ok((start, end))
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
    .to_lowercase()
}

fn is_power_payment(tx: &Transaction) -> bool {
    searchable_text(tx).contains("power")
}

fn is_mortgage_interest(tx: &Transaction) -> bool {
    let txt = searchable_text(tx);
    txt.contains("mortgage interest") || (txt.contains("mortgage") && txt.contains("interest"))
}

pub fn write_xlsx(
    output_path: impl AsRef<Path>,
    transactions: &[Transaction],
    period_start: NaiveDate,
    period_end: NaiveDate,
    summary: Summary,
) -> Result<()> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Transactions")?;

    let headers = [
        "Account Number",
        "Date",
        "Amount",
        "Transaction Code",
        "Transaction Type",
        "Source",
        "Other Party",
        "Particulars",
        "Analysis (Code)",
        "Reference",
        "Serial Number",
        "Account Code",
        "Unique ID",
    ];

    for (col, header) in headers.iter().enumerate() {
        worksheet.write_string(0, col as u16, *header)?;
    }

    for (idx, tx) in transactions.iter().enumerate() {
        let row = (idx + 1) as u32;
        worksheet.write_string(row, 0, &tx.account_number)?;
        worksheet.write_string(row, 1, tx.date.format(DATE_FMT).to_string())?;
        worksheet.write_number(row, 2, tx.amount)?;
        worksheet.write_string(row, 3, &tx.transaction_code)?;
        worksheet.write_string(row, 4, &tx.transaction_type)?;
        worksheet.write_string(row, 5, &tx.source)?;
        worksheet.write_string(row, 6, &tx.other_party)?;
        worksheet.write_string(row, 7, &tx.particulars)?;
        worksheet.write_string(row, 8, &tx.analysis_code)?;
        worksheet.write_string(row, 9, &tx.reference)?;
        worksheet.write_string(row, 10, &tx.serial_number)?;
        worksheet.write_string(row, 11, &tx.account_code)?;
        worksheet.write_string(row, 12, &tx.unique_id)?;
    }

    let summary_ws = workbook.add_worksheet();
    summary_ws.set_name("Summary")?;
    summary_ws.write_string(0, 0, "Tax period start")?;
    summary_ws.write_string(0, 1, period_start.format(DATE_FMT).to_string())?;
    summary_ws.write_string(1, 0, "Tax period end")?;
    summary_ws.write_string(1, 1, period_end.format(DATE_FMT).to_string())?;
    summary_ws.write_string(2, 0, "Total power payments")?;
    summary_ws.write_number(2, 1, summary.power_payments_total)?;
    summary_ws.write_string(3, 0, "Total mortgage interest")?;
    summary_ws.write_number(3, 1, summary.mortgage_interest_total)?;

    workbook
        .save(output_path.as_ref())
        .with_context(|| format!("failed writing '{}'", output_path.as_ref().display()))?;
    Ok(())
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

        let (start, end) = nz_period_for_year(2025).unwrap();
        let summary = summarize_for_period(&txs, start, end);

        assert_eq!(
            summary,
            Summary {
                power_payments_total: 120.0,
                mortgage_interest_total: 500.0
            }
        );
    }
}
