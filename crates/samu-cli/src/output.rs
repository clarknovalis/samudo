// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Data on stdout in the format asked for (ADR 0019 §7).

use std::io::Write;

use anyhow::Result;
use clap::ValueEnum;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Format {
    /// For reading: the columns separated by two spaces.
    Plain,
    /// One JSON array.
    Json,
    /// One JSON object per line.
    Jsonl,
    /// Tab-separated columns, no header.
    Tsv,
}

fn cell(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.replace(['\t', '\n'], " "),
        Some(other) => other.to_string(),
    }
}

/// Pads every column but the last to its widest cell, two spaces apart;
/// a column empty in every row is left out.
fn aligned(rows: &[Vec<String>]) -> Vec<String> {
    let count = rows.first().map_or(0, Vec::len);
    let widths: Vec<usize> = (0..count)
        .map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
        .collect();
    let shown: Vec<usize> = (0..count).filter(|&i| widths[i] > 0).collect();
    rows.iter()
        .map(|row| {
            let mut line = String::new();
            for (n, &i) in shown.iter().enumerate() {
                if n > 0 {
                    line.push_str("  ");
                }
                line.push_str(&row[i]);
                if n + 1 < shown.len() {
                    let pad = widths[i] - row[i].chars().count();
                    line.extend(std::iter::repeat_n(' ', pad));
                }
            }
            line.trim_end().to_string()
        })
        .collect()
}

/// Prints records; `columns` are the fields `plain` and `tsv` show.
pub(crate) fn emit(format: Format, records: &[Value], columns: &[&str]) -> Result<()> {
    let mut out = std::io::stdout().lock();
    match format {
        Format::Json => writeln!(out, "{}", serde_json::to_string_pretty(records)?)?,
        Format::Jsonl => {
            for record in records {
                writeln!(out, "{record}")?;
            }
        }
        Format::Tsv => {
            for record in records {
                let cells: Vec<String> = columns.iter().map(|c| cell(record.get(*c))).collect();
                writeln!(out, "{}", cells.join("\t").trim_end())?;
            }
        }
        Format::Plain => {
            let rows: Vec<Vec<String>> = records
                .iter()
                .map(|record| columns.iter().map(|c| cell(record.get(*c))).collect())
                .collect();
            for line in aligned(&rows) {
                writeln!(out, "{line}")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::aligned;

    #[test]
    fn plain_columns_line_up_and_empty_ones_go() {
        let rows = vec![
            vec![
                "a".to_string(),
                String::new(),
                "Überfällig".to_string(),
                "x".to_string(),
            ],
            vec![
                "bbb".to_string(),
                String::new(),
                "ok".to_string(),
                String::new(),
            ],
        ];
        assert_eq!(aligned(&rows), ["a    Überfällig  x", "bbb  ok"]);
    }
}
