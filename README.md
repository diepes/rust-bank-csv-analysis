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

## CSV sign rules

Mixed positive and negative amounts are supported.
Summary totals are calculated from absolute values of all non-zero transactions in the selected period.

## summary.yaml format

You can provide a top-level named map:

```yaml
power_payments_total:
  description: Total power payments
  regex: power
  color: '#E2F0D9'
  lock_sign_on_first_match: true
mortgage_interest_total:
  description: Total mortgage interest
  regex: mortgage.*interest|interest.*mortgage
  colour: C6EFD6
  lock_sign_on_first_match: true
```

You can also provide a top-level list or a `summaries:` wrapper.

```yaml
summaries:
  - name: power_payments_total
    description: Total power payments
    regex: power
    color: '#E2F0D9'
    lock_sign_on_first_match: true
  - name: mortgage_interest_total
    description: Total mortgage interest
    regex: mortgage.*interest|interest.*mortgage
    color: C6EFD6
    lock_sign_on_first_match: true
```

`color` (or `colour`) is optional and controls the Transactions row background color for matched rows in that summary.
Accepted format: `#RRGGBB` or `RRGGBB`.

The generated workbook contains:
- `Transactions`: all input transactions combined.
  - Includes `Summary` column with the first matching summary name for each row (blank if unmatched).
- `Summary`: totals for the period `1 April` to `31 May` of `--tax-year-start` for each configured summary definition.
  - Each transaction is assigned to the first matching regex (in YAML order).
  - Two built-in rows are always added:
    - `no_match`: transactions that matched no configured regex
    - `total`: total of all non-zero transactions in the period (by absolute value)
  - The code validates that configured totals + `no_match` equals `total`.
  - By default each summary locks sign on first match (`lock_sign_on_first_match: true`).
    If a later matched transaction has the opposite sign, processing fails and reports both the first match file/line and offending file/line.

Summary configuration is validated before use and will fail fast for common issues such as:
- duplicate summary names
- duplicate regex definitions
- regexes that can match empty text (for example trailing `|`)
