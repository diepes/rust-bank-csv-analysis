use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::Datelike;
use clap::Parser;
use rust_bank_csv_analysis::{
    nz_period_for_year, read_transactions, summarize_for_period, write_xlsx,
};

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Merge bank CSV files and export XLSX summary"
)]
struct Cli {
    #[arg(short, long)]
    output: PathBuf,
    #[arg(long)]
    tax_year_start: Option<i32>,
    #[arg(required = true)]
    csv_files: Vec<PathBuf>,
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let transactions = read_transactions(&cli.csv_files)?;

    let start_year = cli
        .tax_year_start
        .or_else(|| transactions.first().map(|tx| tx.date.year()))
        .context("no transactions found; use --tax-year-start when input is empty")?;
    let (period_start, period_end) = nz_period_for_year(start_year)?;
    let summary = summarize_for_period(&transactions, period_start, period_end);
    write_xlsx(
        &cli.output,
        &transactions,
        period_start,
        period_end,
        summary,
    )?;

    println!("Created: {}", cli.output.display());
    println!(
        "Tax period: {} to {}",
        period_start.format("%Y%m%d"),
        period_end.format("%Y%m%d")
    );
    println!("Total power payments: {:.2}", summary.power_payments_total);
    println!(
        "Total mortgage interest: {:.2}",
        summary.mortgage_interest_total
    );

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
