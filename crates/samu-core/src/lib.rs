// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Data model, availability, repetition, HLC and queries for samu.
//!
//! Rules that hold here, as in catastr-core:
//!
//! - **No I/O.** No file access, no network, no database.
//! - **No async runtime.** Nothing in here may need tokio to run.
//! - **Availability is computed here and only here** (ADR 0006). No client
//!   computes it on its own; the Android app reaches this crate through
//!   samu-mobile.
//!
//! The reason is portability: this crate binds to Android via uniffi and has
//! to be testable without a runtime.

pub use samu_api as api;

pub mod availability;
pub mod blockers;
pub mod blocks;
pub mod calendar;
pub mod capture;
pub mod clock;
pub mod clocking;
pub mod close;
pub mod convert;
pub mod delete;
pub mod forecast;
pub mod grant;
pub mod habit;
pub mod hook;
pub mod overlay;
pub mod pairing;
pub mod properties;
pub mod query;
pub mod reminders;
pub mod render;
pub mod repeat;
pub mod report;
pub mod scope;
pub mod share;
pub mod table;
pub mod tagfields;
pub mod views;
pub mod voice;
pub mod workflow;
