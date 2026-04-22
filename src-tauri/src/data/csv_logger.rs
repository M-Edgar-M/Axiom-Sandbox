//! CSV logger for trade records with thread-safe file operations.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rust_decimal::Decimal;
use thiserror::Error;

use super::trade_record::{TradePhase, TradeRecord, TradeStatus};

/// Errors that can occur during CSV operations.
#[derive(Debug, Error)]
pub enum CsvError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Trade not found: {id}")]
    TradeNotFound { id: String },

    #[error("Parse error: {message}")]
    ParseError { message: String },

    #[error("Lock error: failed to acquire mutex")]
    LockError,
}

/// Thread-safe CSV logger for trade records.
///
/// Uses `std::fs::OpenOptions` for safe file appending
/// and a `Mutex` for thread-safe access.
pub struct CsvLogger {
    /// Path to the CSV file.
    path: PathBuf,
    /// Mutex for thread-safe file access.
    lock: Mutex<()>,
}

impl CsvLogger {
    /// Creates a new CsvLogger, initializing the CSV file with headers if it doesn't exist.
    ///
    /// # Arguments
    /// * `path` - Path to the CSV file (e.g., "trades_log.csv")
    pub fn new(path: impl AsRef<Path>) -> Result<Self, CsvError> {
        let path = path.as_ref().to_path_buf();

        // Create file with headers if it doesn't exist
        if !path.exists() {
            let mut file = File::create(&path)?;
            writeln!(file, "{}", TradeRecord::csv_header())?;
        }

        Ok(Self {
            path,
            lock: Mutex::new(()),
        })
    }

    /// Creates a CsvLogger with default path (trades_log.csv in project root).
    pub fn default_path() -> Result<Self, CsvError> {
        Self::new("trades_log.csv")
    }

    /// Appends a new trade record to the CSV file.
    ///
    /// Thread-safe via mutex lock.
    pub fn append_record(&self, record: &TradeRecord) -> Result<(), CsvError> {
        let _guard = self.lock.lock().map_err(|_| CsvError::LockError)?;

        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)?;

        writeln!(file, "{}", record.to_csv_row())?;

        Ok(())
    }

    /// Updates an existing open trade record.
    ///
    /// Finds the trade by ID, updates exit price, PnL, and status.
    /// This rewrites the entire file (necessary for CSV updates).
    pub fn update_record(
        &self,
        trade_id: &str,
        exit_price: Option<Decimal>,
        realized_pnl: Option<Decimal>,
        new_status: Option<TradeStatus>,
        new_phase: Option<TradePhase>,
        new_stop_loss: Option<Decimal>,
        new_position_size: Option<Decimal>,
    ) -> Result<(), CsvError> {
        let _guard = self.lock.lock().map_err(|_| CsvError::LockError)?;

        // Read all lines
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;

        if lines.is_empty() {
            return Err(CsvError::TradeNotFound {
                id: trade_id.to_string(),
            });
        }

        // Find and update the matching trade
        let mut found = false;
        for line in lines.iter_mut().skip(1) {
            // Skip header
            if line.starts_with(trade_id) {
                *line = self.update_csv_line(
                    line,
                    exit_price,
                    realized_pnl,
                    new_status,
                    new_phase,
                    new_stop_loss,
                    new_position_size,
                )?;
                found = true;
                break;
            }
        }

        if !found {
            return Err(CsvError::TradeNotFound {
                id: trade_id.to_string(),
            });
        }

        // Write back all lines
        let mut file = File::create(&self.path)?;
        for line in lines {
            writeln!(file, "{}", line)?;
        }

        Ok(())
    }

    /// Updates fields in a CSV line.
    fn update_csv_line(
        &self,
        line: &str,
        exit_price: Option<Decimal>,
        realized_pnl: Option<Decimal>,
        new_status: Option<TradeStatus>,
        new_phase: Option<TradePhase>,
        new_stop_loss: Option<Decimal>,
        new_position_size: Option<Decimal>,
    ) -> Result<String, CsvError> {
        let parts: Vec<&str> = line.split(',').collect();

        if parts.len() < 18 {
            return Err(CsvError::ParseError {
                message: "Invalid CSV line format: missing columns".to_string(),
            });
        }

        // CSV columns:
        // 0: id, 1: symbol, 2: direction, 3: entry_time, 4: entry_price,
        // 5: stop_loss, 6: take_profit, 7: position_size, 8: status,
        // 9: exit_time, 10: exit_price, 11: realized_pnl, 12: phase,
        // 13: atr_value, 14: risk_per_unit, 15: entry_rsi, 16: entry_adx, 17: trend_condition

        let status = new_status.map_or(parts[8].to_string(), |s| s.to_string());
        let exit_time = if new_status == Some(TradeStatus::Closed) && parts[9].is_empty() {
            chrono::Utc::now().to_rfc3339()
        } else {
            parts[9].to_string()
        };
        let exit_price_str = exit_price.map_or(parts[10].to_string(), |p| p.to_string());
        let pnl_str = realized_pnl.map_or(parts[11].to_string(), |p| p.to_string());
        let phase = new_phase.map_or(parts[12].to_string(), |p| p.to_string());
        let stop_loss = new_stop_loss.map_or(parts[5].to_string(), |s| s.to_string());
        let position_size = new_position_size.map_or(parts[7].to_string(), |s| s.to_string());

        Ok(format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            parts[0],       // id
            parts[1],       // symbol
            parts[2],       // direction
            parts[3],       // entry_time
            parts[4],       // entry_price
            stop_loss,      // stop_loss (may be updated)
            parts[6],       // take_profit
            position_size,  // position_size (may be updated)
            status,         // status
            exit_time,      // exit_time
            exit_price_str, // exit_price
            pnl_str,        // realized_pnl
            phase,          // phase
            parts[13],      // atr_value
            parts[14],      // risk_per_unit
            parts[15],      // entry_rsi
            parts[16],      // entry_adx
            parts[17],      // trend_condition
        ))
    }

    /// Reads all trade records from the CSV file.
    pub fn read_all_records(&self) -> Result<Vec<TradeRecord>, CsvError> {
        let _guard = self.lock.lock().map_err(|_| CsvError::LockError)?;

        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();

        for (i, line) in reader.lines().enumerate() {
            if i == 0 {
                continue; // Skip header
            }
            let line = line?;
            if let Some(record) = self.parse_csv_line(&line) {
                records.push(record);
            }
        }

        Ok(records)
    }

    /// Finds all open trades.
    pub fn find_open_trades(&self) -> Result<Vec<TradeRecord>, CsvError> {
        let records = self.read_all_records()?;
        Ok(records.into_iter().filter(|r| r.is_open()).collect())
    }

    /// Parses a CSV line into a TradeRecord.
    fn parse_csv_line(&self, line: &str) -> Option<TradeRecord> {
        let parts: Vec<&str> = line.split(',').collect();

        if parts.len() < 18 {
            return None;
        }

        use super::trade_record::TradeDirection;
        use chrono::DateTime;
        use rust_decimal::prelude::FromStr;

        Some(TradeRecord {
            id: parts[0].to_string(),
            symbol: parts[1].to_string(),
            direction: TradeDirection::from_str(parts[2])?,
            entry_time: DateTime::parse_from_rfc3339(parts[3])
                .ok()?
                .with_timezone(&chrono::Utc),
            entry_price: Decimal::from_str(parts[4]).ok()?,
            stop_loss: Decimal::from_str(parts[5]).ok()?,
            take_profit: Decimal::from_str(parts[6]).ok()?,
            position_size: Decimal::from_str(parts[7]).ok()?,
            status: TradeStatus::from_str(parts[8])?,
            exit_time: if parts[9].is_empty() {
                None
            } else {
                Some(
                    DateTime::parse_from_rfc3339(parts[9])
                        .ok()?
                        .with_timezone(&chrono::Utc),
                )
            },
            exit_price: if parts[10].is_empty() {
                None
            } else {
                Some(Decimal::from_str(parts[10]).ok()?)
            },
            realized_pnl: Decimal::from_str(parts[11]).ok()?,
            phase: TradePhase::from_str(parts[12])?,
            atr_value: Decimal::from_str(parts[13]).ok()?,
            risk_per_unit: Decimal::from_str(parts[14]).ok()?,
            entry_rsi: Decimal::from_str(parts[15]).ok()?,
            entry_adx: Decimal::from_str(parts[16]).ok()?,
            trend_condition: parts[17].to_string(),
        })
    }

    /// Returns the path to the CSV file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use std::fs;
    use tempfile::tempdir;

    use super::super::trade_record::TradeDirection;

    fn create_test_record(id: &str) -> TradeRecord {
        TradeRecord::new(
            id,
            "BTCUSDT",
            TradeDirection::Long,
            dec!(50000),
            dec!(49000),
            dec!(53000),
            dec!(1),
            dec!(500),
            dec!(60),
            dec!(25),
            "StrongUptrend".to_string(),
        )
    }

    #[test]
    fn test_csv_logger_creation() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_trades.csv");

        let _logger = CsvLogger::new(&path).unwrap();
        assert!(path.exists());

        // Check header
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("id,symbol,direction"));
    }

    #[test]
    fn test_append_record() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_trades.csv");
        let logger = CsvLogger::new(&path).unwrap();

        let record = create_test_record("trade-001");
        logger.append_record(&record).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("trade-001"));
        assert!(content.contains("BTCUSDT"));
        assert!(content.contains("Long"));
    }

    #[test]
    fn test_read_all_records() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_trades.csv");
        let logger = CsvLogger::new(&path).unwrap();

        logger
            .append_record(&create_test_record("trade-001"))
            .unwrap();
        logger
            .append_record(&create_test_record("trade-002"))
            .unwrap();

        let records = logger.read_all_records().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].id, "trade-001");
        assert_eq!(records[1].id, "trade-002");
    }

    #[test]
    fn test_update_record() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_trades.csv");
        let logger = CsvLogger::new(&path).unwrap();

        logger
            .append_record(&create_test_record("trade-001"))
            .unwrap();

        // Update to closed with exit price and PnL
        logger
            .update_record(
                "trade-001",
                Some(dec!(51500)),
                Some(dec!(1500)),
                Some(TradeStatus::Closed),
                None,
                None,
                None,
            )
            .unwrap();

        let records = logger.read_all_records().unwrap();
        assert_eq!(records[0].status, TradeStatus::Closed);
        assert_eq!(records[0].exit_price, Some(dec!(51500)));
        assert_eq!(records[0].realized_pnl, dec!(1500));
    }

    #[test]
    fn test_update_phase_and_stop_loss() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_trades.csv");
        let logger = CsvLogger::new(&path).unwrap();

        logger
            .append_record(&create_test_record("trade-001"))
            .unwrap();

        // Update to Phase2 with new stop loss at breakeven
        logger
            .update_record(
                "trade-001",
                None,
                None,
                None,
                Some(TradePhase::Phase2),
                Some(dec!(50000)), // Breakeven
                Some(dec!(0.67)),  // 33% closed
            )
            .unwrap();

        let records = logger.read_all_records().unwrap();
        assert_eq!(records[0].phase, TradePhase::Phase2);
        assert_eq!(records[0].stop_loss, dec!(50000));
        assert_eq!(records[0].position_size, dec!(0.67));
    }

    #[test]
    fn test_find_open_trades() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_trades.csv");
        let logger = CsvLogger::new(&path).unwrap();

        logger
            .append_record(&create_test_record("trade-001"))
            .unwrap();
        logger
            .append_record(&create_test_record("trade-002"))
            .unwrap();

        // Close one trade
        logger
            .update_record(
                "trade-001",
                Some(dec!(51500)),
                Some(dec!(1500)),
                Some(TradeStatus::Closed),
                None,
                None,
                None,
            )
            .unwrap();

        let open_trades = logger.find_open_trades().unwrap();
        assert_eq!(open_trades.len(), 1);
        assert_eq!(open_trades[0].id, "trade-002");
    }
}
