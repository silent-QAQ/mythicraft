use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use thiserror::Error;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Error)]
pub enum ObservabilityError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("reporter lock poisoned")]
    LockPoisoned,
    #[error("latency sample set is empty")]
    EmptySamples,
    #[error("invalid performance metadata: {0}")]
    InvalidMetadata(String),
}

pub fn init_json_logging(default_filter: &str) -> Result<(), String> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_filter))
        .map_err(|error| error.to_string())?;
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_current_span(true)
        .with_span_list(true)
        .try_init()
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StageResult {
    pub schema_version: u32,
    pub run_id: String,
    pub sequence: u32,
    pub stage: String,
    pub status: StageStatus,
    pub duration_ms: u64,
    pub message: String,
    #[serde(default)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

impl StageResult {
    pub fn passed(
        run_id: impl Into<String>,
        sequence: u32,
        stage: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            schema_version: 1,
            run_id: run_id.into(),
            sequence,
            stage: stage.into(),
            status: StageStatus::Passed,
            duration_ms,
            message: String::new(),
            fields: serde_json::Map::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ObservabilityError> {
        if self.schema_version != 1 {
            return Err(ObservabilityError::InvalidMetadata(
                "unsupported stage schema".into(),
            ));
        }
        if self.run_id.is_empty() || self.run_id.len() > 128 {
            return Err(ObservabilityError::InvalidMetadata("invalid run_id".into()));
        }
        if self.stage.is_empty() || self.stage.len() > 128 {
            return Err(ObservabilityError::InvalidMetadata(
                "invalid stage name".into(),
            ));
        }
        if self.message.len() > 4096 {
            return Err(ObservabilityError::InvalidMetadata(
                "stage message exceeds 4096 bytes".into(),
            ));
        }
        Ok(())
    }
}

pub struct JsonLineReporter {
    path: PathBuf,
    writer: Mutex<BufWriter<File>>,
}

impl JsonLineReporter {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, ObservabilityError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ObservabilityError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| ObservabilityError::Io {
                path: path.clone(),
                source,
            })?;
        Ok(Self {
            path,
            writer: Mutex::new(BufWriter::new(file)),
        })
    }

    pub fn record(&self, result: &StageResult) -> Result<(), ObservabilityError> {
        result.validate()?;
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| ObservabilityError::LockPoisoned)?;
        serde_json::to_writer(&mut *writer, result)?;
        writer
            .write_all(b"\n")
            .map_err(|source| ObservabilityError::Io {
                path: self.path.clone(),
                source,
            })?;
        writer.flush().map_err(|source| ObservabilityError::Io {
            path: self.path.clone(),
            source,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PerformanceMetadata {
    pub schema_version: u32,
    pub scenario: String,
    pub machine: String,
    pub operating_system: String,
    pub rust_profile: String,
    pub target: String,
    pub minecraft_version: String,
    pub map_hash: String,
    pub config_hash: String,
    pub players: u32,
    pub entities: u32,
    pub skill_events_per_second: u32,
    pub duration_seconds: u64,
}

impl PerformanceMetadata {
    pub fn validate(&self) -> Result<(), ObservabilityError> {
        if self.schema_version != 1 {
            return Err(ObservabilityError::InvalidMetadata(
                "unsupported performance schema".into(),
            ));
        }
        for (name, value) in [
            ("scenario", &self.scenario),
            ("machine", &self.machine),
            ("operating_system", &self.operating_system),
            ("rust_profile", &self.rust_profile),
            ("target", &self.target),
            ("minecraft_version", &self.minecraft_version),
            ("map_hash", &self.map_hash),
            ("config_hash", &self.config_hash),
        ] {
            if value.is_empty() || value.len() > 256 {
                return Err(ObservabilityError::InvalidMetadata(format!(
                    "invalid {name}"
                )));
            }
        }
        if self.duration_seconds == 0 {
            return Err(ObservabilityError::InvalidMetadata(
                "duration_seconds must be positive".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LatencySummary {
    pub samples: usize,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl LatencySummary {
    pub fn from_milliseconds(samples: &[f64]) -> Result<Self, ObservabilityError> {
        if samples.is_empty() {
            return Err(ObservabilityError::EmptySamples);
        }
        if samples
            .iter()
            .any(|sample| !sample.is_finite() || *sample < 0.0)
        {
            return Err(ObservabilityError::InvalidMetadata(
                "latency samples must be finite and non-negative".into(),
            ));
        }
        let mut sorted = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        Ok(Self {
            samples: sorted.len(),
            min_ms: sorted[0],
            p50_ms: percentile(&sorted, 0.50),
            p95_ms: percentile(&sorted, 0.95),
            p99_ms: percentile(&sorted, 0.99),
            max_ms: sorted[sorted.len() - 1],
        })
    }

    pub fn meets_tick_gate(&self) -> bool {
        self.p95_ms <= 35.0 && self.p99_ms <= 50.0
    }

    pub fn validate(&self) -> Result<(), ObservabilityError> {
        let ordered = self.samples > 0
            && self.min_ms.is_finite()
            && self.p50_ms.is_finite()
            && self.p95_ms.is_finite()
            && self.p99_ms.is_finite()
            && self.max_ms.is_finite()
            && self.min_ms >= 0.0
            && self.min_ms <= self.p50_ms
            && self.p50_ms <= self.p95_ms
            && self.p95_ms <= self.p99_ms
            && self.p99_ms <= self.max_ms;
        if ordered {
            Ok(())
        } else {
            Err(ObservabilityError::InvalidMetadata(
                "latency summary is invalid or unordered".into(),
            ))
        }
    }
}

fn percentile(sorted: &[f64], percentile: f64) -> f64 {
    let rank = (percentile * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PerformanceReport {
    pub metadata: PerformanceMetadata,
    pub tick_latency: LatencySummary,
    pub memory_peak_bytes: u64,
    pub network_bytes_per_second: u64,
    pub blocking_tick_io_events: u64,
}

impl PerformanceReport {
    pub fn validate(&self) -> Result<(), ObservabilityError> {
        self.metadata.validate()?;
        self.tick_latency.validate()
    }

    pub fn meets_development_gate(&self) -> bool {
        self.tick_latency.meets_tick_gate() && self.blocking_tick_io_events == 0
    }

    pub fn write_json(&self, path: &Path) -> Result<(), ObservabilityError> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ObservabilityError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let file = File::create(path).map_err(|source| ObservabilityError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_summary_uses_nearest_rank_and_checks_gate() {
        let samples: Vec<f64> = (1..=100).map(f64::from).collect();
        let summary = LatencySummary::from_milliseconds(&samples).expect("summary");
        assert_eq!(summary.p50_ms, 50.0);
        assert_eq!(summary.p95_ms, 95.0);
        assert_eq!(summary.p99_ms, 99.0);
        assert!(!summary.meets_tick_gate());
        summary.validate().expect("valid summary");
    }

    #[test]
    fn performance_gate_rejects_blocking_tick_io() {
        let report = PerformanceReport {
            metadata: PerformanceMetadata {
                schema_version: 1,
                scenario: "static_player_movement".into(),
                machine: "test".into(),
                operating_system: "test".into(),
                rust_profile: "release".into(),
                target: "test-target".into(),
                minecraft_version: "test-version".into(),
                map_hash: "00".into(),
                config_hash: "00".into(),
                players: 1,
                entities: 1,
                skill_events_per_second: 0,
                duration_seconds: 1,
            },
            tick_latency: LatencySummary::from_milliseconds(&[1.0, 2.0, 3.0]).expect("summary"),
            memory_peak_bytes: 1,
            network_bytes_per_second: 1,
            blocking_tick_io_events: 1,
        };
        assert!(!report.meets_development_gate());
    }

    #[test]
    fn stage_results_are_json_lines() {
        let path =
            std::env::temp_dir().join(format!("mythicraft-report-{}.jsonl", std::process::id()));
        let reporter = JsonLineReporter::open(&path).expect("open reporter");
        reporter
            .record(&StageResult::passed("run-1", 1, "startup", 4))
            .expect("record");
        let contents = std::fs::read_to_string(&path).expect("read report");
        let decoded: StageResult = serde_json::from_str(contents.trim()).expect("decode result");
        assert_eq!(decoded.stage, "startup");
        std::fs::remove_file(path).expect("cleanup");
    }
}
