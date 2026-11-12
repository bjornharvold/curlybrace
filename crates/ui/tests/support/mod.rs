//! Shared fixtures for the click-driven UI tests: the REAL app and runtime driven through
//! `egui_kittest`, a real MongoDB reached through the driver for seeding and verification, and a
//! bounded polling helper for the asynchronous runtime.
#![allow(dead_code)]

pub mod actions;
pub mod bulk_shots;
pub mod compare;
pub mod compare_apply_states;
pub mod compare_export_shots;
pub mod compare_export_states;
pub mod compare_field_shots;
pub mod compare_field_states;
pub mod compare_multi_states;
pub mod compare_states;
pub mod copy_shots;
pub mod db;
pub mod export_flow;
pub mod fixture;
pub mod guard_shots;
pub mod indexes_shots;
pub mod production_shots;
pub mod screen;
pub mod shell_shots;
pub mod shots;
pub mod sidebar;
pub mod snapshot_options;
pub mod verdict_shots;
pub mod wait;
