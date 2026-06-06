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

**Tax Period** — the NZ tax year window used for summarising: April 1 – May 31 of the given start year.

**Summary Definition** — a user-supplied (or built-in) named rule: a regex matched against a transaction's **Searchable Text**. First match wins. Defined in YAML and validated before use.

**Summary** — the aggregate result for a Tax Period: a total per Summary Definition, plus built-in `loan_repayment_total`, `no_match`, and `total` rows. The invariant `configured + loan_repayment + no_match == total` is checked at calculation time.

**Searchable Text** — the concatenation of a transaction's descriptive fields (transaction type, source, other party, particulars, reference, analysis code, serial number, account code) used when matching against Summary Definition regexes.

**Transfer Detection** — the process of identifying pairs of transactions across different accounts with the same date, same absolute amount, and opposite signs, that represent internal account movements. Produces `InternalTransfer` or `CardPayment` classifications. Requires the full sorted transaction list.

**Loan Repayment Detection** — identifies groups of transactions on the same date with a matching loan-repayment signature, where at least one side is negative. The negative side(s) become `LoanRepaymentCounted`; others become `LoanRepaymentOnly`.

**Classification** — the process of running Transfer Detection and Loan Repayment Detection across the full loaded and sorted transaction list and embedding the result as a `TransactionClass` on each `Transaction`. Classification happens once, immediately after loading and sorting; downstream modules read `tx.class` rather than re-deriving it.
