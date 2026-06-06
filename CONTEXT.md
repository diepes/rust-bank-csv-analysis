# Context

## Domain Glossary

**Transaction** — a single bank ledger entry parsed from a bank-exported CSV file. Has an account number, date, amount, and descriptive fields (other party, particulars, reference, analysis code, etc.). After loading, each transaction is assigned a **Transaction Class**.

**Transaction Class** — the role assigned to a transaction after inter-account analysis. One of:
- `Countable` — contributes to summary totals.
- `InternalTransfer` — a pair of entries representing money moving between the user's own accounts; excluded from totals.
- `CardPayment` — a card top-up or card payment pair across accounts; excluded from totals.
- `LoanRepaymentOnly` — the non-negative side of a loan repayment pair; excluded from totals.
- `LoanRepaymentCounted` — the negative side of a loan repayment pair; counted in its own `loan_repayment_total` bucket.

Classification priority when a transaction matches multiple detectors: `CardPayment` > `InternalTransfer` > loan repayment variants > `Countable`.

**Tax Period** — the NZ tax year window used for summarising: April 1 of the given start year through March 31 of the following year (e.g. 2025-04-01 – 2026-03-31).

**Summary Definition** — a user-supplied (or built-in) named rule: a regex matched against a transaction's **Searchable Text**. First match wins. Defined in YAML and validated before use.

**Summary** — the aggregate result for a Tax Period: a total per Summary Definition, plus built-in `loan_repayment_total`, `no_match`, and `total` rows. The invariant `configured + loan_repayment + no_match == total` is checked at calculation time.

**Searchable Text** — the concatenation of a transaction's descriptive fields (transaction type, source, other party, particulars, reference, analysis code, serial number, account code) used when matching against Summary Definition regexes.

**Transfer Detection** — the process of identifying pairs of transactions across different accounts with the same date, same absolute amount, and opposite signs, that represent internal account movements. Produces `InternalTransfer` or `CardPayment` classifications. Requires the full sorted transaction list.

**Loan Repayment Detection** — identifies groups of transactions on the same date with a matching loan-repayment signature, where at least one side is negative. The negative side(s) become `LoanRepaymentCounted`; others become `LoanRepaymentOnly`.

**Sign Lock** — a per-**Summary Definition** guard (`lock_sign_on_first_match: true`) that tracks the sign of the first matched transaction and checks all subsequent matches against it. Detects misconfigured regexes that accidentally capture income into an expense category.

**Sign Reversal** — a transaction whose sign (positive/negative) differs from the first-matched transaction in the same **Summary Definition** when that definition has **Sign Lock** enabled. A legitimate store return credit is a sign reversal; so is a misconfigured regex — the two are distinguished by whether the first-ever match was positive (fatal: regex likely wrong) or negative (warning: plausible return/refund).

**Sign Reversal Warning** — a value returned alongside a **Summary** when a **Sign Reversal** is detected. Carries the summary name, file path, and line number of the offending transaction. The XLSX writer uses this list to apply a blue row highlight to each reversed transaction; `main.rs` prints each warning to stderr.



## Code Structure

See `README.md` for the end-to-end pipeline diagram.

```
src/
  main.rs               CLI entry point (arg parsing, orchestration)
  lib.rs                Public API: re-exports, Transaction struct, CSV reading, XLSX writing
  calc_summary/
    mod.rs              Re-exports all public symbols; integration tests
    types.rs            Data types: TransactionClass, SummaryDefinition, SummaryItem, Summary, SignReversalWarning, LoanRepaymentFlags
    detection.rs        detect_internal_transfers, detect_card_payments, detect_loan_repayments, classify_transactions
    summary.rs          summarize_for_period, matched_* helpers, validate_summary_definitions, parse_summary_color
    config.rs           load_summary_definitions, default_summary_definitions, YAML parsing
```
