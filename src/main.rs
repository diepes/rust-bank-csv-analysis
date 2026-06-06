use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use rust_bank_csv_analysis::{
    latest_full_tax_year_start, nz_period_for_year, read_transactions,
    resolve_summary_definitions, summarize_for_period, write_xlsx,
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
    #[arg(long)]
    summary_config: Option<PathBuf>,
    #[arg(required = true)]
    csv_files: Vec<PathBuf>,
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let transactions = read_transactions(&cli.csv_files)?;

    let start_year = cli
        .tax_year_start
        .unwrap_or_else(latest_full_tax_year_start);
    let (period_start, period_end) = nz_period_for_year(start_year)?;
    let summary_definitions = resolve_summary_definitions(cli.summary_config.as_ref())?;
    let summary = summarize_for_period(
        &transactions,
        period_start,
        period_end,
        &summary_definitions,
    )?;
    write_xlsx(
        &cli.output,
        &transactions,
        period_start,
        period_end,
        &summary,
    )?;

    println!("Created: {}", cli.output.display());
    println!(
        "Tax period: {} to {}",
        period_start.format("%Y%m%d"),
        period_end.format("%Y%m%d")
    );
    for item in &summary.items {
        if item.description.is_empty() {
            println!("{}: {:.2}", item.name, item.total);
        } else {
            println!("{} ({}): {:.2}", item.name, item.description, item.total);
        }
    }

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
