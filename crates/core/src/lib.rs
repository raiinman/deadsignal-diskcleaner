//! Core engine for the disk cleaner: config, safety rules, scanning,
//! cleanup planning and deletion. No UI code lives here.

pub mod config;
pub mod delete;
pub mod disk;
pub mod plan;
pub mod safety;
pub mod scan;

pub use config::Config;
pub use plan::{CategoryGroup, Plan, PlanItem, Risk, build_plan};
