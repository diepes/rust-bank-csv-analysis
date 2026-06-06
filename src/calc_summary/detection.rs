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

pub fn detect_loan_repayments(transactions: &[Transaction]) -> LoanRepaymentFlags {
    let mut related = vec![false; transactions.len()];
    let mut counted = vec![false; transactions.len()];
    let mut groups: HashMap<(NaiveDate, String), Vec<usize>> = HashMap::new();

    for (idx, tx) in transactions.iter().enumerate() {
        if tx.amount == 0.0 || !looks_like_loan_repayment_candidate(tx) {
            continue;
        }

        groups
            .entry((tx.date, loan_repayment_signature(tx)))
            .or_default()
            .push(idx);
    }

    for indices in groups.values() {
        if indices.is_empty() || indices.len() > 3 {
            continue;
        }

        let has_negative = indices.iter().any(|idx| transactions[*idx].amount < 0.0);
        if !has_negative {
            continue;
        }

        for idx in indices {
            related[*idx] = true;
            if transactions[*idx].amount < 0.0 {
                counted[*idx] = true;
            }
        }
    }

    LoanRepaymentFlags { related, counted }
}

fn looks_like_loan_repayment_candidate(tx: &Transaction) -> bool {
    let fields = [
        tx.transaction_type.as_str(),
        tx.source.as_str(),
        tx.other_party.as_str(),
        tx.particulars.as_str(),
        tx.analysis_code.as_str(),
        tx.reference.as_str(),
    ];

    fields.iter().any(|value| value.to_lowercase().contains("loan repayment"))
}

fn loan_repayment_signature(tx: &Transaction) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        normalize_signature_piece(&tx.transaction_type),
        normalize_signature_piece(&tx.other_party),
        normalize_signature_piece(&tx.particulars),
        normalize_signature_piece(&tx.analysis_code),
        normalize_signature_piece(&tx.reference),
    )
}

fn normalize_signature_piece(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn detect_internal_transfers(transactions: &[Transaction]) -> Vec<bool> {
    let mut flags = vec![false; transactions.len()];
    let mut groups: HashMap<(NaiveDate, i64), Vec<usize>> = HashMap::new();

    for (idx, tx) in transactions.iter().enumerate() {
        if tx.amount == 0.0 {
            continue;
        }

        let amount_cents = (tx.amount.abs() * 100.0).round() as i64;
        let key = (tx.date, amount_cents);
        groups.entry(key).or_default().push(idx);
    }

    for indices in groups.values() {
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a_idx = indices[i];
                let b_idx = indices[j];
                let a = &transactions[a_idx];
                let b = &transactions[b_idx];

                if a.account_number == b.account_number {
                    continue;
                }
                if (a.amount > 0.0) == (b.amount > 0.0) {
                    continue;
                }

                let has_transfer_code = a.analysis_code.eq_ignore_ascii_case("TRANSFER")
                    || b.analysis_code.eq_ignore_ascii_case("TRANSFER");
                let same_reference = !a.reference.trim().is_empty()
                    && a.reference.trim().eq_ignore_ascii_case(b.reference.trim());
                let same_particulars = !a.particulars.trim().is_empty()
                    && a.particulars.trim().eq_ignore_ascii_case(b.particulars.trim());
                let payment_received_card_pair = (looks_like_payment_received(a)
                    && looks_like_card_transfer_outgoing(b))
                    || (looks_like_payment_received(b) && looks_like_card_transfer_outgoing(a));

                let from_to_pair = (looks_like_from_account_counterparty(a)
                    && looks_like_to_account_counterparty(b))
                    || (looks_like_from_account_counterparty(b)
                        && looks_like_to_account_counterparty(a));

                if payment_received_card_pair
                    || ((has_transfer_code || from_to_pair)
                        && (same_reference || same_particulars || from_to_pair))
                {
                    flags[a_idx] = true;
                    flags[b_idx] = true;
                }
            }
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
    let mut groups: HashMap<(NaiveDate, i64), Vec<usize>> = HashMap::new();

    for (idx, tx) in transactions.iter().enumerate() {
        if tx.amount == 0.0 {
            continue;
        }

        let amount_cents = (tx.amount.abs() * 100.0).round() as i64;
        groups.entry((tx.date, amount_cents)).or_default().push(idx);
    }

    for indices in groups.values() {
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a_idx = indices[i];
                let b_idx = indices[j];
                let a = &transactions[a_idx];
                let b = &transactions[b_idx];

                if a.account_number == b.account_number {
                    continue;
                }
                if (a.amount > 0.0) == (b.amount > 0.0) {
                    continue;
                }

                let looks_like_pair = (looks_like_payment_received(a)
                    && looks_like_card_transfer_outgoing(b))
                    || (looks_like_payment_received(b) && looks_like_card_transfer_outgoing(a));
                if !looks_like_pair {
                    continue;
                }

                flags[a_idx] = true;
                flags[b_idx] = true;
            }
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
    use chrono::NaiveDate;
    use crate::Transaction;

    #[test]
    fn detects_internal_transfer_without_transfer_analysis_code() {
        let txs = vec![
            Transaction {
                account_number: "0313950292849000".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 29).unwrap(),
                amount: 150.0,
                transaction_code: String::new(),
                transaction_type: "DIRECT CREDIT".into(),
                source: String::new(),
                other_party: "From 0406-0790348-00".into(),
                particulars: "Car pool".into(),
                analysis_code: String::new(),
                reference: "07:41-86654".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504290001".into(),
                source_file: "a.csv".into(),
                source_line: 2,
                class: TransactionClass::Countable,
            },
            Transaction {
                account_number: "0304060790348000".into(),
                date: NaiveDate::from_ymd_opt(2025, 4, 29).unwrap(),
                amount: -150.0,
                transaction_code: String::new(),
                transaction_type: "ONLINE BANKING".into(),
                source: String::new(),
                other_party: "To 1395-0292849-00".into(),
                particulars: "Car pool".into(),
                analysis_code: String::new(),
                reference: "07:41-86654".into(),
                serial_number: String::new(),
                account_code: String::new(),
                unique_id: "202504290002".into(),
                source_file: "b.csv".into(),
                source_line: 2,
                class: TransactionClass::Countable,
            },
        ];

        let flags = detect_internal_transfers(&txs);
        assert_eq!(flags, vec![true, true]);
    }
}
