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

## Remaining Candidates

### #3 — Remove dead parameter from `read_transactions_with_summary_definitions`

**Files:** `src/lib.rs`

**Problem:** `read_transactions_with_summary_definitions` accepts
`_summary_definitions: Option<&[SummaryDefinition]>` but immediately ignores it
(note the `_` prefix). `main.rs` passes `Some(&summary_definitions)` but it is silently
dropped. The interface promises a capability it does not deliver.

**Solution:** Remove the `_summary_definitions` parameter (or implement it).
Update the one call site in `main.rs` to call `read_transactions` directly.
Keep the public name as an alias if backward compatibility matters, or delete it.

**Note:** `read_transactions_with_summary_definitions` is still exported via `pub use` —
check whether any external caller uses it before removing.

---

### #4 — Extract shared pair-detection scaffold

**Files:** `src/calc_summary.rs` — `detect_internal_transfers`, `detect_card_payments`

**Problem:** Both functions share an identical structure:
1. Build `HashMap<(NaiveDate, i64), Vec<usize>>` grouping by `(date, abs-amount-cents)`
2. Nested `i×j` pair loop
3. Check accounts differ and signs differ
4. Apply a heuristic predicate

Only the final predicate differs. The scaffold is copy-pasted, so a bug in the pairing
logic (e.g. the cross-account check) would need fixing in two places.

**Solution:** Extract a private function:

```rust
fn candidate_pairs<'a>(transactions: &'a [Transaction]) -> impl Iterator<Item=(usize, usize)> + 'a
```

It should yield all `(i, j)` index pairs where accounts differ, signs differ, same date,
and same absolute amount. Both `detect_internal_transfers` and `detect_card_payments`
become a filter over that iterator with their respective predicates.

---

### #5 — Merge `check_summay` into `calc_summary` (fixes typo too)

**Files:** `src/check_summay.rs` (typo — should be `check_summary`),
`src/calc_summary.rs`, `src/lib.rs`

**Problem:** `check_summay::check_summary_definitions` validates duplicates, regex
safety, and colors. `calc_summary::validate_summary_definitions` calls that and adds:
non-empty list, reserved name checks, non-empty regex. The split is arbitrary — callers
inside `calc_summary.rs` must know to call both. The module also has a typo in its name.
`parse_summary_color` is called directly from `write_xlsx` in `lib.rs`.

**Solution:**
1. Move `check_summary_definitions` and `parse_summary_color` into `calc_summary.rs`.
2. Merge `check_summary_definitions` into `validate_summary_definitions` so there is one
   complete validation function.
3. Re-export `parse_summary_color` from `calc_summary` (or `lib.rs`) so `write_xlsx`
   can still call it.
4. Delete `src/check_summay.rs` and remove `pub mod check_summay` from `lib.rs`.
5. Move the `check_summay` tests into `calc_summary` tests.

**Watch out for:** `pub mod check_summay` makes the module part of the public API. Check
whether any external caller uses `rust_bank_csv_analysis::check_summay::parse_summary_color`
before deleting it.

---

## Suggested order

#3 first (trivial, removes a misleading interface), then #5 (moderate, cleans up a real
naming bug and merges split logic), then #4 (most structural, extracts a reusable scaffold).
