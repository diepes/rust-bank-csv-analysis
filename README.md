# rust-bank-csv-analysis
Load and provide summary of bank csv files

## Usage

```bash
cargo run -- \
  --output /tmp/output.xlsx \
  --tax-year-start 2025 \
  /path/to/file1.csv /path/to/file2.csv
```

Optional summary configuration:

```bash
cargo run -- \
  --output /tmp/output.xlsx \
  --summary-config ./summary.yaml \
  /path/to/file1.csv /path/to/file2.csv
```

If `--tax-year-start` is omitted, the CLI defaults to the latest full tax year that starts on `1 April`.
If `--summary-config` is omitted and `./summary.yaml` exists, it is loaded automatically.
If no config file is found, built-in summary definitions are used.

## summary.yaml format

You can provide a top-level named map:

```yaml
power_payments_total:
  description: Total power payments
  regex: power
mortgage_interest_total:
  description: Total mortgage interest
  regex: mortgage.*interest|interest.*mortgage
```

You can also provide a top-level list or a `summaries:` wrapper.

```yaml
summaries:
  - name: power_payments_total
    description: Total power payments
    regex: power
  - name: mortgage_interest_total
    description: Total mortgage interest
    regex: mortgage.*interest|interest.*mortgage
```

The generated workbook contains:
- `Transactions`: all input transactions combined.
- `Summary`: totals for the period `1 April` to `31 May` of `--tax-year-start` for each configured summary definition.
  - Each transaction is assigned to the first matching regex (in YAML order).
  - Two built-in rows are always added:
    - `no_match`: transactions that matched no configured regex
    - `total`: total of all negative transactions in the period
  - The code validates that configured totals + `no_match` equals `total`.
