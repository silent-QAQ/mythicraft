//! Black-box contract harness for assembling Window 1/2/3 adapters.

use mythicraft_observability::{JsonLineReporter, StageResult, StageStatus};
use std::fmt::Display;
use std::path::Path;
use std::time::Instant;

pub const REQUIRED_STAGES: &[&str] = &[
    "startup",
    "world_load",
    "client_connect",
    "capability_negotiation",
    "player_move",
    "rpg_spawn",
    "skill_damage",
    "loot_economy",
    "ui_audio",
    "disconnect",
    "reconnect",
    "save_restore",
];

pub struct IntegrationRunner {
    run_id: String,
    sequence: u32,
    reporter: JsonLineReporter,
}

impl IntegrationRunner {
    pub fn open(path: &Path, run_id: impl Into<String>) -> Result<Self, String> {
        Ok(Self {
            run_id: run_id.into(),
            sequence: 0,
            reporter: JsonLineReporter::open(path).map_err(|error| error.to_string())?,
        })
    }

    pub fn stage<T, E>(
        &mut self,
        name: &str,
        operation: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, String>
    where
        E: Display,
    {
        self.sequence += 1;
        let started = Instant::now();
        match operation() {
            Ok(value) => {
                self.reporter
                    .record(&StageResult::passed(
                        &self.run_id,
                        self.sequence,
                        name,
                        started.elapsed().as_millis() as u64,
                    ))
                    .map_err(|error| error.to_string())?;
                Ok(value)
            }
            Err(error) => {
                let message = error.to_string();
                self.reporter
                    .record(&StageResult {
                        schema_version: 1,
                        run_id: self.run_id.clone(),
                        sequence: self.sequence,
                        stage: name.into(),
                        status: StageStatus::Failed,
                        duration_ms: started.elapsed().as_millis() as u64,
                        message: message.clone(),
                        fields: serde_json::Map::new(),
                    })
                    .map_err(|report_error| {
                        format!("{message}; failed to record stage result: {report_error}")
                    })?;
                Err(message)
            }
        }
    }

    pub fn skip(&mut self, name: &str, reason: impl Into<String>) -> Result<(), String> {
        self.sequence += 1;
        self.reporter
            .record(&StageResult {
                schema_version: 1,
                run_id: self.run_id.clone(),
                sequence: self.sequence,
                stage: name.into(),
                status: StageStatus::Skipped,
                duration_ms: 0,
                message: reason.into(),
                fields: serde_json::Map::new(),
            })
            .map_err(|error| error.to_string())
    }
}
