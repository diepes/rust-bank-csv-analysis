use std::collections::HashMap;

use chrono::NaiveDate;

use crate::Transaction;

use super::types::{LoanRepaymentFlags, TransactionClass};

/// Classifies every transaction by running all detector passes and embedding
/// the result into `tx.class`.  Call this once, immediately after loading and
/// sorting.  Priority: CardPayment > InternalTransfer > loan repayment variants
/// > Countable.
pub fn classify_transactions(transactions: &mut [Transaction]) {
    let internal_flags = detect_internal_transfers(transactions);
    let card_flags = detect_card_payments(transactions);
    let loan_flags = detect_loan_repayments(transactions);

    for (idx, tx) in transactions.iter_mut().enumerate() {
        tx.class = if card_flags[idx] {
            TransactionClass::CardPayment
        } else if internal_flags[idx] {
            TransactionClass::InternalTransfer
        } else if loan_flags.related[idx] && !loan_flags.counted[idx] {
            TransactionClass::LoanRepaymentOnly
        } else if loan_flags.counted[idx] {
            TransactionClass::LoanRepaymentCounted
        } else {
            TransactionClass::Countable
        };
    }
}

/// Detects loan repayment transactions using a two-pass approach:
///
/// Pass 1 — debits: any transaction where `other_party` contains "loan repayment"
///   (case-insensitive) and `amount < 0` is immediately marked as counted
///   (→ `loan_repayment_total`).
///
/// Pass 2 — credits: a positive-amount transaction where `other_party` ~
///   "loan repayment" is marked as related-only (→ `skip_loan_transfer`) only
///   when it can be paired with a same-date, same-absolute-amount debit using
///   the `analysis_code` cross-reference.  Banks encode the counterpart account
///   number in `analysis_code` with a dash separator (e.g. `"790348-91"`).
///   Stripping the dash gives a substring that appears inside the counterpart's
///   full account number (e.g. `"79034891"` ⊆ `"0304060790348091"`).  We check
///   both directions (credit→debit and debit→credit) to handle either leg.
pub fn detect_loan_repayments(transactions: &[Transaction]) -> LoanRepaymentFlags {
    let mut related = vec![false; transactions.len()];
    let mut counted = vec![false; transactions.len()];

    // Pass 1: mark all debits with other_party ~ "loan repayment".
    for (idx, tx) in transactions.iter().enumerate() {
        if tx.amount < 0.0 && is_loan_repayment_other_party(tx) {
            related[idx] = true;
            counted[idx] = true;
        }
    }

    // Pass 2: match credits to debits via analysis_code ↔ account_number.
    // Group candidates by (date, |amount_cents|) for O(n) pairing.
    let mut by_date_amount: HashMap<(NaiveDate, i64), Vec<usize>> = HashMap::new();
    for (idx, tx) in transactions.iter().enumerate() {
        if tx.amount != 0.0 && is_loan_repayment_other_party(tx) {
            let cents = (tx.amount.abs() * 100.0).round() as i64;
            by_date_amount
                .entry((tx.date, cents))
                .or_default()
                .push(idx);
        }
    }

    for indices in by_date_amount.values() {
        let credits: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&i| transactions[i].amount > 0.0)
            .collect();
        if credits.is_empty() {
            continue;
        }
        let debits: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&i| transactions[i].amount < 0.0)
            .collect();

        for credit_idx in credits {
            let credit = &transactions[credit_idx];
            let credit_code = credit.analysis_code.replace('-', "");
            let matched = debits.iter().any(|&debit_idx| {
                let debit = &transactions[debit_idx];
                let debit_code = debit.analysis_code.replace('-', "");
                (credit_code.len() >= 4 && debit.account_number.contains(&credit_code))
                    || (debit_code.len() >= 4 && credit.account_number.contains(&debit_code))
            });
            if matched {
                related[credit_idx] = true;
                // counted stays false → LoanRepaymentOnly (skip_loan_transfer)
            }
        }
    }

    LoanRepaymentFlags { related, counted }
}

fn is_loan_repayment_other_party(tx: &Transaction) -> bool {
    tx.other_party.to_lowercase().contains("loan repayment")
}

/// Yields all `(i, j)` index pairs (i < j) where the two transactions are on the
/// same date, have the same absolute amount, belong to different accounts, and
/// have opposite signs.  This is the common scaffolding shared by both transfer
/// detectors; each detector applies its own heuristic predicate on top.
fn candidate_pairs(transactions: &[Transaction]) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut groups: HashMap<(NaiveDate, i64), Vec<usize>> = HashMap::new();

    for (idx, tx) in transactions.iter().enumerate() {
        if tx.amount == 0.0 {
            continue;
        }
        let amount_cents = (tx.amount.abs() * 100.0).round() as i64;
        groups.entry((tx.date, amount_cents)).or_default().push(idx);
    }

    groups.into_values().flat_map(move |indices| {
        let mut pairs = Vec::new();
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a_idx = indices[i];
                let b_idx = indices[j];
                let a = &transactions[a_idx];
                let b = &transactions[b_idx];
                if a.account_number != b.account_number && (a.amount > 0.0) != (b.amount > 0.0) {
                    pairs.push((a_idx, b_idx));
                }
            }
        }
        pairs
    })
}

pub fn detect_internal_transfers(transactions: &[Transaction]) -> Vec<bool> {
    let mut flags = vec![false; transactions.len()];

    for (a_idx, b_idx) in candidate_pairs(transactions) {
        let a = &transactions[a_idx];
        let b = &transactions[b_idx];

        let has_transfer_code = a.analysis_code.eq_ignore_ascii_case("TRANSFER")
            || b.analysis_code.eq_ignore_ascii_case("TRANSFER");
        let same_reference = !a.reference.trim().is_empty()
            && a.reference.trim().eq_ignore_ascii_case(b.reference.trim());
        let same_particulars = !a.particulars.trim().is_empty()
            && a.particulars
                .trim()
                .eq_ignore_ascii_case(b.particulars.trim());
        let payment_received_card_pair = (looks_like_payment_received(a)
            && looks_like_card_transfer_outgoing(b))
            || (looks_like_payment_received(b) && looks_like_card_transfer_outgoing(a));
        let from_to_pair = (looks_like_from_account_counterparty(a)
            && looks_like_to_account_counterparty(b))
            || (looks_like_from_account_counterparty(b) && looks_like_to_account_counterparty(a));

        if payment_received_card_pair
            || ((has_transfer_code || from_to_pair)
                && (same_reference || same_particulars || from_to_pair))
        {
            flags[a_idx] = true;
            flags[b_idx] = true;
        }
    }

    flags
}

fn looks_like_to_account_counterparty(tx: &Transaction) -> bool {
    let text = tx.other_party.trim().to_lowercase();
    text.starts_with("to ") && text.contains('-') && text.chars().any(|ch| ch.is_ascii_digit())
}

fn looks_like_from_account_counterparty(tx: &Transaction) -> bool {
    let text = tx.other_party.trim().to_lowercase();
    text.starts_with("from ") && text.contains('-') && text.chars().any(|ch| ch.is_ascii_digit())
}

pub fn detect_card_payments(transactions: &[Transaction]) -> Vec<bool> {
    let mut flags = vec![false; transactions.len()];

    for (a_idx, b_idx) in candidate_pairs(transactions) {
        let a = &transactions[a_idx];
        let b = &transactions[b_idx];

        if (looks_like_payment_received(a) && looks_like_card_transfer_outgoing(b))
            || (looks_like_payment_received(b) && looks_like_card_transfer_outgoing(a))
        {
            flags[a_idx] = true;
            flags[b_idx] = true;
        }
    }

    flags
}

fn looks_like_payment_received(tx: &Transaction) -> bool {
    tx.other_party.to_lowercase().contains("payment received")
}

fn looks_like_card_transfer_outgoing(tx: &Transaction) -> bool {
    let lower = tx.other_party.to_lowercase();
    lower.starts_with("to ") && tx.other_party.contains("****")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Transaction;
    use chrono::NaiveDate;

    #[test]
    fn detects_internal_transfer_without_transfer_analysis_code() {
        let txs = vec![
            Transaction {
                account_number: "0313950292849000".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 29).unwrap(),
                amount: 150.0,
                transaction_type: "DIRECT CREDIT".into(),
                other_party: "From 0406-0790348-00".into(),
                particulars: "Car pool".into(),
                reference: "07:41-86654".into(),
                unique_id: "202504290001".into(),
                source_file: "a.csv".into(),
                source_line: 2,
                class: TransactionClass::Countable,
                ..Default::default()
            },
            Transaction {
                account_number: "0304060790348000".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 29).unwrap(),
                amount: -150.0,
                transaction_type: "ONLINE BANKING".into(),
                other_party: "To 1395-0292849-00".into(),
                particulars: "Car pool".into(),
                reference: "07:41-86654".into(),
                unique_id: "202504290002".into(),
                source_file: "b.csv".into(),
                source_line: 2,
                class: TransactionClass::Countable,
                ..Default::default()
            },
        ];

        let flags = detect_internal_transfers(&txs);
        assert_eq!(flags, vec![true, true]);
    }
}
