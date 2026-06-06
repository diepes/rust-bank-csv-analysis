# rust-bank-csv-analysis
Load and provide summary of bank csv files

## Usage

```bash
cargo run -- \
  --output /tmp/output.xlsx \
  --tax-year-start 2025 \
  /path/to/file1.csv /path/to/file2.csv
```

The generated workbook contains:
- `Transactions`: all input transactions combined.
- `Summary`: totals for the period `1 April` to `31 May` of `--tax-year-start`:
  - Total power payments
  - Total mortgage interest
