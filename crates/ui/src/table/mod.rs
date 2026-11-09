//! Tables: the column layout model here, the virtualised table element beside it.

mod element;
pub mod layout;

pub use element::{Pending, Table, TableCell, TableState, Toggle};
pub use layout::{ColumnSpec, Layout, Sort, Sorting, TextAlign, Width, rank};
