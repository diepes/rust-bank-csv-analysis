use serde::Deserialize;

/// The role assigned to a transaction after inter-account analysis.
/// See CONTEXT.md for the full definition and priority rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionClass {
    /// Contributes to summary totals.
    Countable,
    /// Money moving between the user's own accounts — excluded from totals.
    InternalTransfer,
    /// Card top-up / card payment pair across accounts — excluded from totals.
    CardPayment,
    /// Non-negative side of a loan repayment pair — excluded from totals.
    LoanRepaymentOnly,
    /// Negative side of a loan repayment pair — counted in `loan_repayment_total`.
    LoanRepaymentCounted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignReversalWarning {
    pub summary_name: String,
    pub source_file: String,
    pub source_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SummaryDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub regex: String,
    #[serde(default, alias = "colour")]
    pub color: Option<String>,
    #[serde(default = "default_lock_sign_on_first_match")]
    pub lock_sign_on_first_match: bool,
    /// Set to `true` for income categories (e.g. salary) where transactions
    /// are credits (positive). Flips the sign-lock: first positive is the
    /// lock, subsequent negatives are sign-reversal warnings.
    #[serde(default)]
    pub income: bool,
}

pub(crate) fn default_lock_sign_on_first_match() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq)]
pub struct SummaryItem {
    pub name: String,
    pub description: String,
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub items: Vec<SummaryItem>,
}

#[derive(Debug, Clone)]
pub struct LoanRepaymentFlags {
    pub related: Vec<bool>,
    pub counted: Vec<bool>,
}
