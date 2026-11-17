//! Pure domain logic of the YSM Summon Editor.
//!
//! Nothing in this crate performs IO. The data layer (`ysm-data`) feeds it
//! snapshots and the UI drives it through [`session::EditSession`].

pub mod commit;
pub mod evolution;
pub mod i18n;
pub mod model;
pub mod random_group;
pub mod review;
pub mod save_plan;
pub mod scaling;
pub mod scaling_edit;
pub mod session;
pub mod sql_script;
pub mod tooltip;
pub mod value;

pub use rust_decimal::Decimal;
