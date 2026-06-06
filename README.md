# rust-bank-csv-analysis
Load and provide summary of bank csv files

## Pipeline

```
 CSV files (one per account)     summary.yaml (or built-in defaults)
          │                                    │
          ▼                                    ▼
 read_transactions()              resolve_summary_definitions()
 • parse rows                     • load YAML config
 • merge and sort by date         • validate definitions
 • detect and mark:               • compile regexes once
     InternalTransfer                          │
     CardPayment                    CompiledSummarySet
     LoanRepayment*                            │
 • tx.class set on each row                   │
          │                                   │
          └──────────────┬────────────────────┘
                         │
                Vec<Transaction> + CompiledSummarySet
                         │
                         ▼
          summarize_for_period(period_start, period_end)
          • skip non-Countable transactions
          • regex-match each tx to a summary category
          • accumulate totals; detect sign reversals
                         │
              Summary + Vec<SignReversalWarning>
                         │
                         ▼
                    write_xlsx()
          • Transactions sheet: all rows with
            class / summary colour highlights
          • Summary sheet: period totals table
                         │
                         ▼
                    output.xlsx
```

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
  - Three built-in rows are always added:
    - `loan_repayment_total`: negative loan repayment rows that were detected as related activity
    - `no_match`: transactions that matched no configured regex
    - `total`: total of all non-zero transactions in the period (by absolute value)
  - The code validates that configured totals + `loan_repayment_total` + `no_match` equals `total`.
  - By default each summary locks sign on first match (`lock_sign_on_first_match: true`).
    If the first matched transaction is **positive** (credit), processing fails immediately — expense
    summaries expect debits. If a later matched transaction has the opposite sign (e.g. a store return),
    a **Sign Reversal Warning** is emitted and the transaction is included in the net total; the row is
    highlighted blue in the Transactions sheet.

## Transfer Heuristics

Rows that look like transfers between accounts are ignored from all summary calculations and are highlighted in yellow in the `Transactions` sheet.

`transfer_internal` is detected when a row pair matches all of these conditions:
- different accounts
- same date
- same absolute amount
- opposite signs
- and either:
  - one side has `Analysis (Code) = TRANSFER`, or
  - the counterparties look like `From ...` and `To ...` account transfers
- plus an additional relationship signal, such as matching `Reference`, matching `Particulars`, or a clear `From`/`To` pair

`card_payment` is detected when a pair looks like a card top-up / card payment transfer:
- different accounts
- same date
- same absolute amount
- opposite signs
- one side looks like `PAYMENT RECEIVED ...`
- the other side looks like `To ************....`

Matched `transfer_internal` rows use a lighter yellow, while `card_payment` rows use a stronger yellow.

## Loan Repayments

Rows that look like loan repayment activity are highlighted in light blue in the `Transactions` sheet.

`loan_repayment_total` is detected when a small related group of rows matches the loan repayment pattern:
- the transaction text includes `loan repayment`
- rows share the same date
- related rows usually come in groups of 2, and can be up to 3 rows
- only negative rows are counted in the `loan_repayment_total` summary line

Positive companion rows in the same loan repayment group are shown in the workbook but are not counted toward the summary total.

Summary configuration is validated before use and will fail fast for common issues such as:
- duplicate summary names
- duplicate regex definitions
- regexes that can match empty text (for example trailing `|`)
