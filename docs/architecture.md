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

## Remaining Candidates

### #3 — Extract shared pair-detection scaffold

**Files:** `src/calc_summary/detection.rs`

**Problem:** `detect_internal_transfers` and `detect_card_payments` share an identical
scaffold:
1. Build `HashMap<(NaiveDate, i64), Vec<usize>>` grouping by `(date, abs-amount-cents)`
2. Nested `i×j` pair loop
3. Check accounts differ and signs differ
4. Apply a heuristic predicate

Only the final predicate differs. The scaffold is copy-pasted, so a bug in the pairing
logic (e.g. the cross-account check) would need fixing in two places.

**Solution:** Extract a private function:

```rust
fn candidate_pairs(transactions: &[Transaction]) -> impl Iterator<Item=(usize, usize)> + '_
```

It should yield all `(i, j)` index pairs where accounts differ, signs differ, same date,
and same absolute amount. Both `detect_internal_transfers` and `detect_card_payments`
become a filter over that iterator with their respective predicates.

---

## Suggested order

#3 (moderate, removes copy-pasted pair-detection scaffold).

