pub(crate) mod config;
pub(crate) mod detection;
pub(crate) mod summary;
pub(crate) mod types;

pub use config::{default_summary_definitions, load_summary_definitions};
pub use detection::{
    classify_transactions, detect_card_payments, detect_internal_transfers, detect_loan_repayments,
};
pub use summary::{CompiledSummarySet, parse_summary_color};
pub use types::{
    LoanRepaymentFlags, SignReversalWarning, Summary, SummaryDefinition, SummaryItem,
    TransactionClass,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Transaction;
    use chrono::NaiveDate;

    fn tx(
        account: &str,
        date: NaiveDate,
        amount: f64,
        transaction_type: &str,
        other_party: &str,
        particulars: &str,
        analysis_code: &str,
        reference: &str,
        unique_id: &str,
        source_file: &str,
        source_line: usize,
    ) -> Transaction {
        Transaction {
            account_number: account.into(),
            date,
            amount,
            transaction_type: transaction_type.into(),
            other_party: other_party.into(),
            particulars: particulars.into(),
            analysis_code: analysis_code.into(),
            reference: reference.into(),
            unique_id: unique_id.into(),
            source_file: source_file.into(),
            source_line,
            ..Default::default()
        }
    }

    #[test]
    fn detects_and_excludes_internal_transfers_from_summary_totals() {
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let mut txs = vec![
            tx(
                "A",
                d(2026, 4, 12),
                -50.0,
                "ONLINE BANKING",
                "To 1395-0292849-00",
                "Lunch NewJob",
                "TRANSFER",
                "12:45-753463",
                "u1",
                "a.csv",
                2,
            ),
            tx(
                "B",
                d(2026, 4, 12),
                50.0,
                "DIRECT CREDIT",
                "FRM 0406-0790348-00",
                "Lunch NewJob",
                "TRANSFER",
                "12:45-753463",
                "u2",
                "b.csv",
                2,
            ),
            tx(
                "A",
                d(2026, 4, 13),
                -120.0,
                "AUTOMATIC PAYMENT",
                "Power Co",
                "",
                "",
                "abc",
                "u3",
                "a.csv",
                3,
            ),
        ];

        let flags = detect_internal_transfers(&txs);
        assert_eq!(flags, vec![true, true, false]);

        classify_transactions(&mut txs);
        let start = d(2026, 4, 1);
        let end = d(2026, 5, 31);
        let definitions = default_summary_definitions();
        let compiled = CompiledSummarySet::compile(&definitions).unwrap();
        compiled.annotate(&mut txs).unwrap();
        let summary = compiled.summarize_for_period(&txs, start, end).unwrap();

        assert_eq!(summary.items[0].name, "power_payments_total");
        assert_eq!(summary.items[0].total, 120.0);
        assert_eq!(summary.items.last().unwrap().name, "total");
        assert_eq!(summary.items.last().unwrap().total, 120.0);
    }

    #[test]
    fn detects_and_excludes_card_payments_from_summary_totals() {
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let mut txs = vec![
            tx(
                "CARD",
                d(2025, 4, 10),
                355.0,
                "",
                "PAYMENT RECEIVED THANK YOU NZL",
                "",
                "",
                "20250410",
                "u1",
                "card.csv",
                2,
            ),
            tx(
                "BANK",
                d(2025, 4, 10),
                -355.0,
                "ONLINE BANKING",
                "To ************2640",
                "WBC Internet",
                "TRANSFER",
                "12:42-04708",
                "u2",
                "bank.csv",
                2,
            ),
            tx(
                "BANK",
                d(2025, 4, 11),
                -120.0,
                "AUTOMATIC PAYMENT",
                "Power Co",
                "",
                "",
                "abc",
                "u3",
                "bank.csv",
                3,
            ),
        ];

        let flags = detect_card_payments(&txs);
        assert_eq!(flags, vec![true, true, false]);

        classify_transactions(&mut txs);
        let start = d(2025, 4, 1);
        let end = d(2025, 5, 31);
        let definitions = default_summary_definitions();
        let compiled = CompiledSummarySet::compile(&definitions).unwrap();
        compiled.annotate(&mut txs).unwrap();
        let summary = compiled.summarize_for_period(&txs, start, end).unwrap();

        assert_eq!(summary.items[0].name, "power_payments_total");
        assert_eq!(summary.items[0].total, 120.0);
        assert_eq!(summary.items.last().unwrap().name, "total");
        assert_eq!(summary.items.last().unwrap().total, 120.0);
    }

    #[test]
    fn detects_loan_repayments_and_counts_only_negative_rows() {
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        // Real-world scenario: main account has two debits on the same day —
        // -2800 (principal transfer) and -77 (loan fee) — plus the +2800 credit
        // at the loan account. All share "LOAN REPAYMENT" text so they form one
        // group. The two negatives are counted; the positive is only.
        let mut txs = vec![
            tx(
                "0304060790348000",
                d(2025, 6, 21),
                -77.0,
                "LOAN REPAYMENT",
                "Loan repayment",
                "0406 0",
                "790348-92",
                "",
                "u1",
                "main.csv",
                2,
            ),
            tx(
                "0304060790348091",
                d(2025, 6, 21),
                2800.0,
                "LOAN REPAYMENT",
                "Loan repayment",
                "0406 0",
                "790348-00",
                "",
                "u2",
                "loan.csv",
                2,
            ),
            tx(
                "0304060790348000",
                d(2025, 6, 21),
                -2800.0,
                "LOAN REPAYMENT",
                "Loan repayment",
                "0406 0",
                "790348-91",
                "",
                "u3",
                "main.csv",
                3,
            ),
        ];

        let flags = detect_loan_repayments(&txs);
        // Debits with other_party="Loan repayment" → counted immediately.
        // The +2800 credit matches -2800 debit via analysis_code cross-reference
        // ("79034800" ⊆ "0304060790348000") → related-only (skip_loan_transfer).
        assert_eq!(flags.related, vec![true, true, true]);
        assert_eq!(flags.counted, vec![true, false, true]);

        classify_transactions(&mut txs);
        assert_eq!(txs[0].class, crate::TransactionClass::LoanRepaymentCounted);
        assert_eq!(txs[1].class, crate::TransactionClass::LoanRepaymentOnly);
        assert_eq!(txs[2].class, crate::TransactionClass::LoanRepaymentCounted);

        let defs = default_summary_definitions();
        let set = CompiledSummarySet::compile(&defs).unwrap();
        set.annotate(&mut txs).unwrap();

        let start = d(2025, 4, 1);
        let end = d(2025, 6, 30);
        let summary = set.summarize_for_period(&txs, start, end).unwrap();

        assert_eq!(summary.items[2].name, "loan_repayment_total");
        assert_eq!(summary.items[2].total, 2877.0); // 2800 + 77 fee
        assert_eq!(summary.items[3].name, "no_match");
        assert_eq!(summary.items[3].total, 0.0);
        assert_eq!(summary.items[4].name, "total");
        assert_eq!(summary.items[4].total, 2877.0);
    }

    #[test]
    fn detects_payment_received_card_charge_without_reference_match() {
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let mut txs = vec![
            tx(
                "0000000003071972735",
                d(2025, 9, 10),
                1200.0,
                "",
                "PAYMENT RECEIVED       THANK YOU     NZL",
                "",
                "",
                "20250910",
                "u1",
                "card.csv",
                46,
            ),
            tx(
                "0304060790348000",
                d(2025, 9, 10),
                -1200.0,
                "ONLINE BANKING",
                "To ************2640",
                "Fly Melbourn",
                "",
                "21:23-90724",
                "u2",
                "bank.csv",
                96,
            ),
        ];

        let flags = detect_card_payments(&txs);
        assert_eq!(flags, vec![true, true]);

        classify_transactions(&mut txs);
        let defs = default_summary_definitions();
        let compiled = CompiledSummarySet::compile(&defs).unwrap();
        compiled.annotate(&mut txs).unwrap();
        assert_eq!(txs[0].summary_name, None);
        assert_eq!(txs[1].summary_name, None);
    }
}
