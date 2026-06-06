# Architecture Improvement Log — rust-bank-csv-analysis

Domain glossary: [`CONTEXT.md`](../CONTEXT.md)

---

## Completed

### ✅ #1 — Unified Transaction Classification (commit `221d247`)

**Problem:** `detect_internal_transfers`, `detect_card_payments`, and
`detect_loan_repayments` were called independently inside four separate functions
(`summarize_for_period`, `matched_transactions`, `matched_transactions_for_period`,
`matched_summary_names`, `write_xlsx`). The same exclusion guard was duplicated 4× and
classification was recomputed on every call.

**What was done:**
- Added `TransactionClass` enum in `src/calc_summary.rs`:
  `Countable | InternalTransfer | CardPayment | LoanRepaymentOnly | LoanRepaymentCounted`
- Added `classify_transactions(&mut [Transaction])` — runs all three detectors once and
  stamps `tx.class` on each transaction. Priority: `CardPayment` > `InternalTransfer` >
  `LoanRepaymentOnly/Counted` > `Countable`.
- Added `pub class: TransactionClass` to the `Transaction` struct in `src/lib.rs`
  (defaults to `Countable` at CSV parse time).
- `read_transactions_from_paths` calls `classify_transactions` immediately after sort —
  classification happens once, on load.
- All downstream functions now read `tx.class` directly.
- All 23 tests pass.

---

### ✅ #2 — Split `calc_summary` into focused submodules

**Problem:** `src/calc_summary.rs` had grown to ~52 KB mixing detection heuristics,
classification, summary calculation, YAML config parsing, and validation in one file.
Finding a function required skimming the entire file, and tests were collapsed into one
large block making it hard to see which behaviour each test covered.

**What was done:**
- Converted `src/calc_summary.rs` into a module directory `src/calc_summary/` with four
  focused submodules:

  | Submodule         | Responsibility |
  |-------------------|----------------|
  | `types.rs`        | Public data types: `TransactionClass`, `SummaryDefinition`, `SummaryItem`, `Summary`, `LoanRepaymentFlags` |
  | `detection.rs`    | All three detectors (`detect_internal_transfers`, `detect_card_payments`, `detect_loan_repayments`) plus `classify_transactions` and private heuristic helpers |
  | `summary.rs`      | `summarize_for_period`, matching helpers, `validate_summary_definitions`, `parse_summary_color`, and private `compile_summary_definitions` / `searchable_text` |
  | `config.rs`       | YAML loading (`load_summary_definitions`) and built-in defaults (`default_summary_definitions`) |

- `mod.rs` re-exports all public symbols so the external API (via `lib.rs`) is unchanged.
- Tests were distributed to the submodule they cover; integration tests that span multiple
  modules live in `calc_summary::tests` (mod.rs).
- Introduced a `tx()` helper in `summary.rs` tests and a richer one in `mod.rs` tests to
  reduce `Transaction` construction boilerplate.
- All 23 tests pass; no public API changes.

---

### ✅ #3 — Extract shared pair-detection scaffold (detection.rs)

**Problem:** `detect_internal_transfers` and `detect_card_payments` shared an identical
scaffold (group by date+amount-cents, nested i×j pair loop, same-account + same-sign
guards). A bug in the pairing logic would need fixing in two places.

**What was done:**
- Extracted `candidate_pairs(transactions)` — a private function that builds the
  `HashMap<(NaiveDate, i64), Vec<usize>>` once, iterates all `(i, j)` pairs, and
  yields only pairs where accounts differ, signs differ, date matches, and absolute
  amount matches.
- Both `detect_internal_transfers` and `detect_card_payments` are now a `for` loop
  over `candidate_pairs` with their respective heuristic predicates — no duplicated
  grouping or guard logic.
- All 23 tests pass; no public API changes.

---

### ✅ #4 — Extract XLSX writing into `src/xlsx.rs`

**Problem:** `lib.rs` contained a 170-line `write_xlsx` function alongside CSV reading, the
`Transaction` struct, and public re-exports — mixing I/O concerns.  Formatting constants,
colour logic, and sheet layout were buried in the general-purpose library entry point.

**What was done:**
- Created `src/xlsx.rs` with three functions:
  - `pub write_xlsx(...)` — top-level entry point, delegates to the two sheet writers
  - `write_transactions_sheet(...)` (private) — builds the Transactions sheet
    (headers, column widths, row colours by `TransactionClass` and summary match, autofilter)
  - `write_summary_sheet(...)` (private) — builds the Summary sheet (period dates + items table)
- `lib.rs` now declares `pub mod xlsx` and re-exports `xlsx::write_xlsx`; the old 170-line
  implementation and its imports (`HashMap`, `rust_xlsxwriter::{Color, Format, Workbook}`)
  were removed from `lib.rs`.
- All 23 tests pass; no public API changes.

---

## Planned

### ✅ #5 — Introduce `CompiledSummarySet` to eliminate repeated validate+compile

**Problem:** In a single run, `summarize_for_period`, `matched_summary_names`, and
`matched_transactions_for_period` → `matched_transactions` each independently called
`validate_summary_definitions` + `compile_summary_definitions` on the same definitions
slice — 3× compiles (and 3× regex builds) per run.

**What was done:**
- Added `pub struct CompiledSummarySet` in `summary.rs` with a `compile(definitions)` constructor
  that validates and builds the regex set exactly once. The struct also carries `color` per entry
  (needed by `xlsx.rs`), exposed via a `color_map()` method.
- Moved the bodies of `summarize_for_period`, `matched_transactions`,
  `matched_transactions_for_period`, and `matched_summary_names` onto `CompiledSummarySet`
  as `pub` methods (the `definitions` parameter is gone — `self` owns the compiled set).
  The `matched_*` methods are now infallible (`Vec<…>` not `Result<Vec<…>>`).
- Kept the four free functions as one-liner shims during the transition (API stability gate),
  then removed them along with their `pub use` re-exports from `mod.rs` and `lib.rs`.
- Updated `main.rs` to compile once and call methods directly.
- Updated `xlsx.rs` to accept `&CompiledSummarySet` instead of `&[SummaryDefinition]`.
- Updated all tests in `summary.rs` and `mod.rs` to use `CompiledSummarySet` directly.
- All 23 tests pass; `CompiledSummarySet` is now the sole public entry point for
  summary matching and period totalling.

---

## Remaining Candidates

*(none at this time)*

---

### ✅ #6 — Transaction Enrichment Model (`annotate` step)

**Problem:** `xlsx.rs` re-ran the full regex set on every transaction to decide row colours
and column values. `summarize_for_period` also ran its own regex pass. The same patterns
were compiled and matched 2× per run on the same data.

**What was done:**
- Added `summary_name: Option<String>` and `is_sign_reversed: bool` to the `Transaction`
  struct (both in `Default`; `None`/`false` at CSV parse time).
- Added `SignReversalWarning { summary_name, source_file, source_line }` to `types.rs`.
- Added `CompiledSummarySet::annotate(&self, &mut [Transaction]) -> Result<Vec<SignReversalWarning>>`
  which runs once over all transactions, stamps `tx.summary_name` and `tx.is_sign_reversed`,
  and returns any sign-reversal warnings. Fatal if a sign-locked summary's very first match
  has the wrong sign (likely misconfigured regex).
- Rewrote `summarize_for_period` to read `tx.summary_name` / `tx.is_sign_reversed` — zero
  regex work.
- Rewrote `xlsx.rs` to read the same fields — removed the `matched_names`/`matched_in_period`
  Vec allocations and the second regex pass.
- Updated `main.rs` to call `annotate()` after `compile()` and print warnings to stderr.
- All 25 tests pass; the pipeline now has three clearly-separated enrichment stages:
  (1) CSV parse → raw fields, (2) `classify_transactions` → `tx.class`,
  (3) `annotate` → `tx.summary_name` / `tx.is_sign_reversed`.

---

### ✅ #7 — `income: true` flag for credit-first Summary Definitions

**Problem:** The sign-lock logic assumed all summaries were expenses (first match negative).
Salary and other income categories failed with a fatal error on their first positive credit.

**What was done:**
- Added `income: bool` (default `false`) to `SummaryDefinition` and `SummaryDefinitionBody`
  (YAML-deserialised; also propagated through to `CompiledSummaryDefinition`).
- `annotate()` reads `def.income` to determine `expected_positive`; the fatal / warning
  branches are symmetric for both directions.
- `SummaryDefinitionBody` in `config.rs` now also deserialises `lock_sign_on_first_match`
  directly from YAML (previously it was hardcoded to the default in Named-map parsing).
- Two new tests: `income_summary_accepts_positive_and_warns_on_negative` and
  `income_summary_fatal_on_first_negative`.
- All 25 tests pass.

