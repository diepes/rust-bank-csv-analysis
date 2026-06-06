use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use rust_xlsxwriter::{Color, Format, Workbook};

use crate::{
    DATE_FMT, Transaction,
    calc_summary::{CompiledSummarySet, Summary, TransactionClass},
};

pub fn write_xlsx(
    output_path: impl AsRef<Path>,
    transactions: &[Transaction],
    period_start: NaiveDate,
    period_end: NaiveDate,
    summary_set: &CompiledSummarySet,
    summary: &Summary,
) -> Result<()> {
    let mut workbook = Workbook::new();
    let summary_colors: HashMap<String, u32> = summary_set.color_map().into_iter().collect();
    write_transactions_sheet(
        &mut workbook,
        transactions,
        period_start,
        period_end,
        &summary_colors,
    )?;
    write_summary_sheet(&mut workbook, period_start, period_end, summary)?;
    workbook
        .save(output_path.as_ref())
        .with_context(|| format!("failed writing '{}'", output_path.as_ref().display()))?;
    Ok(())
}

fn write_transactions_sheet(
    workbook: &mut Workbook,
    transactions: &[Transaction],
    period_start: NaiveDate,
    period_end: NaiveDate,
    summary_colors: &HashMap<String, u32>,
) -> Result<()> {
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Transactions")?;

    let fmt_matched_in_period = Format::new().set_background_color(Color::RGB(0xE2F0D9));
    let fmt_matched_outside_period = Format::new().set_background_color(Color::RGB(0xC6EFD6));
    let fmt_loan_repayment = Format::new().set_background_color(Color::RGB(0xD9EAF7));
    let fmt_internal_transfer = Format::new().set_background_color(Color::RGB(0xFFF2CC));
    let fmt_card_payment = Format::new().set_background_color(Color::RGB(0xFFE599));
    let fmt_sign_reversed = Format::new().set_background_color(Color::RGB(0x9DC3E6));

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
        "Summary",
    ];
    let column_widths = [
        18.0, 12.0, 12.0, 18.0, 22.0, 20.0, 24.0, 28.0, 18.0, 16.0, 16.0, 14.0, 18.0, 22.0,
    ];

    for (col, width) in column_widths.iter().enumerate() {
        worksheet.set_column_width(col as u16, *width)?;
    }
    for (col, header) in headers.iter().enumerate() {
        worksheet.write_string(0, col as u16, *header)?;
    }
    worksheet.set_freeze_panes(1, 0)?;

    for (idx, tx) in transactions.iter().enumerate() {
        let row = (idx + 1) as u32;
        let in_period = tx.date >= period_start && tx.date <= period_end;

        // Row background colour
        match tx.class {
            TransactionClass::CardPayment => {
                worksheet.set_row_format(row, &fmt_card_payment)?;
            }
            TransactionClass::InternalTransfer => {
                worksheet.set_row_format(row, &fmt_internal_transfer)?;
            }
            TransactionClass::LoanRepaymentCounted | TransactionClass::LoanRepaymentOnly => {
                worksheet.set_row_format(row, &fmt_loan_repayment)?;
            }
            TransactionClass::Countable => {
                if tx.is_sign_reversed {
                    worksheet.set_row_format(row, &fmt_sign_reversed)?;
                } else if let Some(name) = tx.summary_name.as_deref() {
                    if let Some(rgb) = summary_colors.get(name) {
                        worksheet.set_row_format(
                            row,
                            &Format::new().set_background_color(Color::RGB(*rgb)),
                        )?;
                    } else if in_period {
                        worksheet.set_row_format(row, &fmt_matched_in_period)?;
                    } else {
                        worksheet.set_row_format(row, &fmt_matched_outside_period)?;
                    }
                }
            }
        }

        // Cell data
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

        // Summary label column
        let label = match tx.class {
            TransactionClass::LoanRepaymentCounted => Some("loan_repayment_total"),
            TransactionClass::CardPayment => Some("card_payment"),
            TransactionClass::InternalTransfer => Some("transfer_internal"),
            _ => None,
        };
        if let Some(l) = label {
            worksheet.write_string(row, 13, l)?;
        } else if let Some(name) = &tx.summary_name {
            worksheet.write_string(row, 13, name.as_str())?;
        }
    }

    let last_row = transactions.len() as u32;
    let last_col = (headers.len() - 1) as u16;
    worksheet.autofilter(0, 0, last_row, last_col)?;

    Ok(())
}

fn write_summary_sheet(
    workbook: &mut Workbook,
    period_start: NaiveDate,
    period_end: NaiveDate,
    summary: &Summary,
) -> Result<()> {
    let ws = workbook.add_worksheet();
    ws.set_name("Summary")?;
    ws.write_string(0, 0, "Tax period start")?;
    ws.write_string(0, 1, period_start.format(DATE_FMT).to_string())?;
    ws.write_string(1, 0, "Tax period end")?;
    ws.write_string(1, 1, period_end.format(DATE_FMT).to_string())?;
    ws.write_string(3, 0, "Name")?;
    ws.write_string(3, 1, "Description")?;
    ws.write_string(3, 2, "Total")?;

    for (idx, item) in summary.items.iter().enumerate() {
        let row = (idx + 4) as u32;
        ws.write_string(row, 0, &item.name)?;
        ws.write_string(row, 1, &item.description)?;
        ws.write_number(row, 2, item.total)?;
    }

    Ok(())
}
