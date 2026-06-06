# Sign reversal is a warning, not a fatal error

When a summary's `lock_sign_on_first_match` is true and a later transaction has the
opposite sign to the first match (e.g. a Bunnings return credit against an expense
summary), we emit a `SignReversalWarning` and continue rather than aborting. The
reversed transaction is included in the net total. One hard exception remains: if the
very **first** match for an expense summary is positive, that is still a fatal error —
it almost certainly means the regex is misconfigured to match income instead of spending.

## Considered options

- **Keep fatal for all sign mismatches** — rejected: legitimate store returns break the run
  with no recovery path.
- **Exclude reversed transactions silently** — rejected: silently dropping money from a tax
  summary is worse than including it.
- **Print warnings to stderr inside the library** — rejected: `summarize_for_period` is a
  pure function; hiding a side-effect there makes it hard to test and hard for the XLSX
  writer to know which rows to highlight.

## Consequences

- `summarize_for_period` now returns `(Summary, Vec<SignReversalWarning>)` instead of
  `Result<Summary>` (it is still fallible for fatal cases, so the full signature is
  `Result<(Summary, Vec<SignReversalWarning>)>`).
- The XLSX writer uses the warning list to apply a blue row highlight to each reversed
  transaction in the Transactions sheet.
- `main.rs` prints each warning to stderr before saving the file.
- Setting `lock_sign_on_first_match: false` on a definition opts it out entirely — no
  warning, no blue highlight.
