//! Shared Ptyktos document parsing/conversion APIs. Parsers return structured data first;
//! product-specific presentation and import policy belong to their callers.

pub mod xlsx;

/// A parsed worksheet. `index` is zero-based in workbook order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worksheet {
    pub index: usize,
    /// Current Webfold-compatible display name (`Sheet N`). The workbook XML's human name
    /// resolution is a follow-up; callers can still select sheets deterministically by index.
    pub name: String,
    pub rows: Vec<Vec<String>>,
}
