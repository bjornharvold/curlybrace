//! Shared fixtures for the click-driven UI tests: the REAL app and runtime driven through
//! `egui_kittest`, a real MongoDB reached through the driver for seeding and verification, and a
//! bounded polling helper for the asynchronous runtime.
#![allow(dead_code)]

pub mod actions;
pub mod db;
pub mod fixture;
pub mod screen;
pub mod wait;
