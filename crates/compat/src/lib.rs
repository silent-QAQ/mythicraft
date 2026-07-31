use mythicraft_economy::Economy;
use mythicraft_permission::{Group, PermissionEngine, PermissionNode, User};
use mythicraft_rpg::{
    Condition, Effect, EntityOptions, RpgDocument, RpgEntityDefinition, SkillDefinition,
    TargetSelector, Trigger,
};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::{BTreeMap, HashMap};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceSpan {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub path: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub source: SourceSpan,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ImportStatus {
    Supported,
    Converted,
    Partial,
    Unsupported,
    Invalid,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImportReport {
    pub source_version: String,
    pub status: ImportStatus,
    pub diagnostics: Vec<Diagnostic>,
    pub document: Option<RpgDocument>,
    pub ir_hash: Option<String>,
}
#[derive(Debug, Error)]
pub enum ImportError {
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("top-level YAML must be a mapping")]
    NotMapping,
}

fn strip_utf8_bom(source: &str) -> &str {
    source.strip_prefix('\u{feff}').unwrap_or(source)
}

fn parse_yaml(source: &str) -> Result<Value, ImportError> {
    Ok(serde_yaml::from_str(strip_utf8_bom(source))?)
}

pub fn import_mythicmobs(
    file: &str,
    source: &str,
    dry_run: bool,
) -> Result<ImportReport, ImportError> {
    let source = strip_utf8_bom(source);
    let root = parse_yaml(source)?;
    let mapping = root.as_mapping().ok_or(ImportError::NotMapping)?;
    let mut diagnostics = vec![];
    let mut doc = RpgDocument::default();
    for (key, value) in mapping {
        let Some(name) = key.as_str() else { continue };
        let path = format!("$.{name}");
        if name.eq_ignore_ascii_case("Mobs") || name.eq_ignore_ascii_case("Entities") {
            if let Some(mobs) = value.as_mapping() {
                for (id, raw) in mobs {
                    parse_entity(
                        file,
                        source,
                        &path,
                        id.as_str().unwrap_or(""),
                        raw,
                        &mut doc,
                        &mut diagnostics,
                    );
                }
            } else {
                diagnostics.push(diag(
                    file,
                    source,
                    &path,
                    "MM_INVALID_TYPE",
                    Severity::Error,
                    "Mobs must be a mapping",
                ));
            }
        } else if !name.eq_ignore_ascii_case("Version") {
            diagnostics.push(diag(
                file,
                source,
                &path,
                "MM_UNKNOWN_FIELD",
                Severity::Warning,
                "unknown top-level field; it was not silently discarded",
            ));
        }
    }
    if let Err(errors) = doc.validate() {
        for error in errors {
            diagnostics.push(diag(
                file,
                source,
                "$",
                "RPG_IR_INVALID",
                Severity::Error,
                error.to_string(),
            ));
        }
    }
    let status = if diagnostics
        .iter()
        .any(|d| matches!(d.severity, Severity::Error))
    {
        ImportStatus::Invalid
    } else if diagnostics
        .iter()
        .any(|d| matches!(d.severity, Severity::Warning))
    {
        ImportStatus::Partial
    } else {
        ImportStatus::Converted
    };
    let hash = (!dry_run).then(|| doc.content_hash());
    Ok(ImportReport {
        source_version: "MythicMobs-5.13.0".into(),
        status,
        diagnostics,
        document: Some(doc),
        ir_hash: hash,
    })
}
fn parse_entity(
    file: &str,
    source: &str,
    parent_path: &str,
    id: &str,
    raw: &Value,
    doc: &mut RpgDocument,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let entity_path = format!("{parent_path}.{id}");
    let Some(map) = raw.as_mapping() else {
        diagnostics.push(diag(
            file,
            source,
            &entity_path,
            "MM_INVALID_TYPE",
            Severity::Error,
            "mob definition must be a mapping",
        ));
        return;
    };
    let mut known = BTreeMap::new();
    for (k, v) in map {
        if let Some(k) = k.as_str() {
            known.insert(k.to_ascii_lowercase(), v);
        }
    }
    let entity_type = scalar(&known, "type").unwrap_or_else(|| "ZOMBIE".into());
    let display = scalar(&known, "display").unwrap_or_else(|| id.into());
    let health = number(&known, "health", file, source, id, diagnostics).unwrap_or(20.0);
    let damage = number(&known, "damage", file, source, id, diagnostics).unwrap_or(1.0);
    let equipment = known
        .get("equipment")
        .and_then(|v| v.as_sequence())
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let options = EntityOptions {
        movement_speed: number_value(known.get("options").copied(), "movement-speed"),
        prevent_other_drops: bool_value(known.get("options").copied(), "prevent-other-drops"),
        invincible: bool_value(known.get("options").copied(), "invincible"),
    };
    if let Some(options) = known.get("options") {
        if options.as_mapping().is_none() {
            diagnostics.push(diag(
                file,
                source,
                &format!("{entity_path}.Options"),
                "MM_INVALID_TYPE",
                Severity::Error,
                "Options must be a mapping",
            ));
        }
    }
    for key in known.keys() {
        if !matches!(
            key.as_str(),
            "type"
                | "display"
                | "health"
                | "damage"
                | "equipment"
                | "options"
                | "skills"
                | "drops"
                | "experience"
        ) {
            diagnostics.push(diag(
                file,
                source,
                &format!("{entity_path}.{key}"),
                "MM_UNKNOWN_FIELD",
                Severity::Warning,
                "unknown mob field",
            ));
        }
    }
    if known.contains_key("drops") {
        diagnostics.push(diag(
            file,
            source,
            &format!("{entity_path}.drops"),
            "MM_UNSUPPORTED_FIELD",
            Severity::Warning,
            "drops is recognized but the current RPG IR has no loot mapping",
        ));
    }
    let experience =
        number(&known, "experience", file, source, id, diagnostics).unwrap_or(0.0) as u32;
    let (skills, triggers) = parse_skills(
        file,
        source,
        &entity_path,
        id,
        known.get("skills").copied(),
        diagnostics,
    );
    doc.entities.push(RpgEntityDefinition {
        id: id.into(),
        display,
        entity_type,
        health,
        damage,
        attributes: vec![],
        equipment,
        options,
        triggers,
        skills,
        loot_table: None,
        experience,
    });
}
fn parse_skills(
    file: &str,
    source: &str,
    entity_path: &str,
    entity_id: &str,
    value: Option<&Value>,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Vec<SkillDefinition>, Vec<Trigger>) {
    let mut skills = Vec::new();
    let mut triggers = Vec::new();
    let Some(value) = value else {
        return (skills, triggers);
    };
    let Some(sequence) = value.as_sequence() else {
        diagnostics.push(diag(
            file,
            source,
            &format!("{entity_path}.Skills"),
            "MM_INVALID_TYPE",
            Severity::Error,
            "Skills must be a sequence",
        ));
        return (skills, triggers);
    };
    for (index, raw) in sequence.iter().enumerate() {
        let Some(line) = raw.as_str() else {
            diagnostics.push(diag(
                file,
                source,
                &format!("{entity_path}.Skills[{index}]"),
                "MM_INVALID_TYPE",
                Severity::Error,
                "skill entry must be a string",
            ));
            continue;
        };
        let (body, trigger) = line
            .split_once('~')
            .map(|(body, trigger)| (body.trim(), Some(trigger.trim())))
            .unwrap_or((line.trim(), None));
        if let Some(trigger) = trigger {
            if let Some(parsed) = parse_trigger(trigger) {
                triggers.push(parsed);
            } else {
                diagnostics.push(diag(
                    file,
                    source,
                    &format!("{entity_path}.Skills[{index}]"),
                    "MM_UNKNOWN_TRIGGER",
                    Severity::Warning,
                    format!("unsupported trigger: {trigger}"),
                ));
            }
        }
        let mut conditions = Vec::new();
        let mut effects = Vec::new();
        for (effect_index, part) in body
            .split('|')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .enumerate()
        {
            if part.to_ascii_lowercase().starts_with("condition{") {
                match parse_condition(part) {
                    Ok(condition) => conditions.push(condition),
                    Err((code, message)) => diagnostics.push(diag(
                        file,
                        source,
                        &format!("{entity_path}.Skills[{index}].conditions[{effect_index}]"),
                        code,
                        Severity::Error,
                        message,
                    )),
                }
                continue;
            }
            match parse_effect(part) {
                Ok(effect) => effects.push(effect),
                Err((code, message)) => diagnostics.push(diag(
                    file,
                    source,
                    &format!("{entity_path}.Skills[{index}].mechanics[{effect_index}]"),
                    code,
                    if code == "MM_UNKNOWN_MECHANIC" {
                        Severity::Warning
                    } else {
                        Severity::Error
                    },
                    message,
                )),
            }
        }
        if !effects.is_empty() {
            skills.push(SkillDefinition {
                id: format!("{entity_id}.skill.{index}"),
                conditions,
                effects,
                cooldown_ticks: 0,
            });
        }
    }
    (skills, triggers)
}
fn parse_trigger(raw: &str) -> Option<Trigger> {
    let normalized = raw.to_ascii_lowercase();
    match normalized.as_str() {
        "onspawn" => Some(Trigger::Spawn),
        "ondeath" => Some(Trigger::Death),
        "ondamaged" => Some(Trigger::Damaged),
        "ontarget" | "ontargetacquired" => Some(Trigger::TargetAcquired),
        _ => normalized
            .strip_prefix("ontimer")
            .and_then(|n| n.parse().ok())
            .map(|ticks| Trigger::Timer { ticks }),
    }
}
fn parse_condition(raw: &str) -> Result<Condition, (&'static str, String)> {
    let args = raw
        .split_once('{')
        .map(|(_, value)| value.trim_end_matches('}'))
        .unwrap_or("");
    let values = args
        .split(';')
        .filter_map(|pair| {
            pair.split_once('=')
                .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim()))
        })
        .collect::<BTreeMap<_, _>>();
    let kind = values
        .get("type")
        .or_else(|| values.get("condition"))
        .copied()
        .unwrap_or("always")
        .to_ascii_lowercase();
    match kind.as_str() {
        "always" => Ok(Condition::Always),
        "healthbelow" | "health_below" => values
            .get("value")
            .or_else(|| values.get("threshold"))
            .and_then(|v| v.parse().ok())
            .map(Condition::HealthBelow)
            .ok_or((
                "MM_INVALID_PARAMETER",
                "healthbelow condition requires value".into(),
            )),
        "targetinrange" | "target_in_range" => values
            .get("value")
            .or_else(|| values.get("radius"))
            .and_then(|v| v.parse().ok())
            .map(Condition::TargetInRange)
            .ok_or((
                "MM_INVALID_PARAMETER",
                "targetinrange condition requires value".into(),
            )),
        "permission" | "haspermission" => values
            .get("node")
            .map(|node| Condition::HasPermission((*node).into()))
            .ok_or((
                "MM_INVALID_PARAMETER",
                "permission condition requires node".into(),
            )),
        _ => Err((
            "MM_UNKNOWN_CONDITION",
            format!("unsupported condition: {kind}"),
        )),
    }
}
fn parse_effect(raw: &str) -> Result<Effect, (&'static str, String)> {
    let mut parts = raw.split_whitespace();
    let mechanic = parts.next().unwrap_or("");
    let target = parse_target(parts.next().unwrap_or("@self"));
    let (name, args) = mechanic
        .split_once('{')
        .map(|(n, a)| (n.to_ascii_lowercase(), a.trim_end_matches('}')))
        .unwrap_or((mechanic.to_ascii_lowercase(), ""));
    let values = args
        .split(';')
        .filter_map(|pair| {
            pair.split_once('=')
                .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim()))
        })
        .collect::<BTreeMap<_, _>>();
    let number = |key: &str, default: f64| {
        values
            .get(key)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    match name.as_str() {
        "skill" => values
            .get("s")
            .or_else(|| values.get("skill"))
            .map(|skill_id| {
                Ok(Effect::Skill {
                    skill_id: (*skill_id).into(),
                    target,
                })
            })
            .unwrap_or_else(|| {
                Err((
                    "MM_INVALID_PARAMETER",
                    "skill mechanic requires s=skill-id".into(),
                ))
            }),
        "damage" => Ok(Effect::Damage {
            amount: number("amount", 0.0),
            target,
        }),
        "heal" => Ok(Effect::Heal {
            amount: number("amount", 0.0),
            target,
        }),
        "knockback" => Ok(Effect::Knockback {
            strength: number("strength", number("amount", 0.0)),
            target,
        }),
        "status" | "potion" => Ok(Effect::Status {
            effect: values
                .get("effect")
                .or_else(|| values.get("type"))
                .unwrap_or(&"unknown")
                .to_string(),
            duration_ticks: values
                .get("duration")
                .or_else(|| values.get("duration_ticks"))
                .and_then(|v| v.parse().ok())
                .unwrap_or(20),
            target,
        }),
        "" => Err(("MM_INVALID_MECHANIC", "empty mechanic".into())),
        _ => Err((
            "MM_UNKNOWN_MECHANIC",
            format!("unsupported mechanic: {name}"),
        )),
    }
}
fn parse_target(raw: &str) -> TargetSelector {
    match raw.to_ascii_lowercase().as_str() {
        "@self" | "self" => TargetSelector::SelfEntity,
        "@target" | "target" => TargetSelector::TriggerTarget,
        s if s.starts_with("@nearest{radius=") => TargetSelector::NearestEnemy {
            radius: s
                .trim_start_matches("@nearest{radius=")
                .trim_end_matches('}')
                .parse()
                .unwrap_or(16.0),
        },
        s => TargetSelector::Explicit(s.trim_start_matches('@').into()),
    }
}
fn scalar(map: &BTreeMap<String, &Value>, key: &str) -> Option<String> {
    map.get(key).and_then(|v| {
        v.as_str()
            .map(String::from)
            .or_else(|| v.as_i64().map(|n| n.to_string()))
    })
}
fn number(
    map: &BTreeMap<String, &Value>,
    key: &str,
    file: &str,
    source: &str,
    id: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<f64> {
    match map.get(key) {
        None => None,
        Some(v) => match v.as_f64().or_else(|| v.as_i64().map(|n| n as f64)) {
            Some(n) => Some(n),
            None => {
                diagnostics.push(diag(
                    file,
                    source,
                    &format!("$.Mobs.{id}.{key}"),
                    "MM_INVALID_TYPE",
                    Severity::Error,
                    "field must be numeric",
                ));
                None
            }
        },
    }
}
fn number_value(value: Option<&Value>, key: &str) -> Option<f64> {
    value
        .and_then(|v| v.as_mapping())
        .and_then(|m| {
            m.iter().find(|(k, _)| {
                k.as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(key))
            })
        })
        .and_then(|(_, v)| v.as_f64().or_else(|| v.as_i64().map(|n| n as f64)))
}
fn bool_value(value: Option<&Value>, key: &str) -> bool {
    value
        .and_then(|v| v.as_mapping())
        .and_then(|m| {
            m.iter().find(|(k, _)| {
                k.as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case(key))
            })
        })
        .and_then(|(_, v)| v.as_bool())
        .unwrap_or(false)
}
fn diag(
    file: &str,
    source: &str,
    path: &str,
    code: &str,
    severity: Severity,
    message: impl Into<String>,
) -> Diagnostic {
    let (line, column) = source
        .lines()
        .enumerate()
        .find_map(|(i, line)| {
            path.rsplit(|separator| matches!(separator, '.' | '[' | ']'))
                .filter(|part| !part.is_empty() && part.chars().any(|c| !c.is_ascii_digit()))
                .find_map(|part| line.find(part).map(|column| (i + 1, column + 1)))
        })
        .unwrap_or((1, 1));
    Diagnostic {
        code: code.into(),
        severity,
        message: message.into(),
        source: SourceSpan {
            file: file.into(),
            line,
            column,
            path: path.into(),
        },
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdapterReport {
    pub source_version: String,
    pub status: ImportStatus,
    pub diagnostics: Vec<Diagnostic>,
    pub covered_fields: Vec<String>,
    pub unsupported: Vec<String>,
    pub raw_hash: String,
}

pub fn import_server_properties(file: &str, source: &str, dry_run: bool) -> AdapterReport {
    let supported = [
        "online-mode",
        "server-port",
        "motd",
        "difficulty",
        "gamemode",
        "view-distance",
        "simulation-distance",
        "enable-command-block",
    ];
    let mut diagnostics = Vec::new();
    let mut covered = Vec::new();
    let mut unsupported = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let text = line.trim().trim_start_matches('\u{feff}');
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let Some((key, _value)) = text.split_once('=') else {
            diagnostics.push(diag(
                file,
                source,
                &format!("$.line[{}]", index + 1),
                "PAPER_INVALID_LINE",
                Severity::Error,
                "expected key=value",
            ));
            continue;
        };
        if supported.contains(&key) {
            covered.push(key.into());
        } else {
            unsupported.push(key.into());
            diagnostics.push(diag(
                file,
                source,
                &format!("$.{key}"),
                "PAPER_UNSUPPORTED_FIELD",
                Severity::Warning,
                "server.properties field is outside the supported migration subset",
            ));
        }
    }
    adapter_report(
        "Paper-server.properties",
        covered,
        unsupported,
        diagnostics,
        source,
        dry_run,
    )
}

pub fn import_vault_config(
    file: &str,
    source: &str,
    dry_run: bool,
) -> Result<AdapterReport, ImportError> {
    let source = strip_utf8_bom(source);
    let root = parse_yaml(source)?;
    let Some(map) = root.as_mapping() else {
        return Err(ImportError::NotMapping);
    };
    let supported = ["currency-name", "starting-balance", "fractional-digits"];
    let mut covered = Vec::new();
    let mut unsupported = Vec::new();
    let mut diagnostics = Vec::new();
    let mut currency_name = "coins".to_string();
    let mut starting_balance = 0_i64;
    let mut fractional_digits = 0_u64;
    for (key, _) in map {
        let Some(name) = key.as_str() else { continue };
        if supported
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(name))
        {
            covered.push(name.into());
            let value = value_for(map, name);
            match name.to_ascii_lowercase().as_str() {
                "currency-name" => match value.and_then(Value::as_str) {
                    Some(value) if !value.trim().is_empty() => currency_name = value.into(),
                    _ => diagnostics.push(diag(
                        file,
                        source,
                        &format!("$.{name}"),
                        "VAULT_INVALID_VALUE",
                        Severity::Error,
                        "currency-name must be a non-empty string",
                    )),
                },
                "starting-balance" => match value.and_then(integer_value) {
                    Some(value) if value >= 0 => starting_balance = value,
                    _ => diagnostics.push(diag(
                        file,
                        source,
                        &format!("$.{name}"),
                        "VAULT_INVALID_VALUE",
                        Severity::Error,
                        "starting-balance must be a non-negative integer",
                    )),
                },
                "fractional-digits" => match value.and_then(Value::as_u64) {
                    Some(value) if value <= 18 => fractional_digits = value,
                    _ => diagnostics.push(diag(
                        file,
                        source,
                        &format!("$.{name}"),
                        "VAULT_INVALID_VALUE",
                        Severity::Error,
                        "fractional-digits must be an integer from 0 to 18",
                    )),
                },
                _ => {}
            }
        } else {
            unsupported.push(name.into());
            diagnostics.push(diag(
                file,
                source,
                &format!("$.{name}"),
                "VAULT_UNSUPPORTED_FIELD",
                Severity::Warning,
                "Vault field is outside the native economy subset",
            ));
        }
    }
    diagnostics.push(diag(
        file,
        source,
        "$.currency-name",
        "VAULT_ECONOMY_INIT",
        Severity::Info,
        format!(
            "Economy initialized with currency-name={currency_name}, starting-balance={starting_balance}, fractional-digits={fractional_digits}; starting balance is applied when a new account is created"
        ),
    ));
    Ok(adapter_report(
        "VaultUnlocked-config",
        covered,
        unsupported,
        diagnostics,
        source,
        dry_run,
    ))
}

pub fn import_luckperms_config(
    file: &str,
    source: &str,
    dry_run: bool,
) -> Result<AdapterReport, ImportError> {
    let source = strip_utf8_bom(source);
    let root = parse_yaml(source)?;
    let Some(map) = root.as_mapping() else {
        return Err(ImportError::NotMapping);
    };
    let supported = ["users", "groups", "tracks", "meta"];
    let mut covered = Vec::new();
    let mut unsupported = Vec::new();
    let mut diagnostics = Vec::new();
    for (key, _) in map {
        let Some(name) = key.as_str() else { continue };
        if supported
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(name))
        {
            covered.push(name.into())
        } else {
            unsupported.push(name.into());
            diagnostics.push(diag(
                file,
                source,
                &format!("$.{name}"),
                "LP_UNSUPPORTED_FIELD",
                Severity::Warning,
                "LuckPerms backend field requires a native adapter",
            ));
        }
    }
    Ok(adapter_report(
        "LuckPerms-config",
        covered,
        unsupported,
        diagnostics,
        source,
        dry_run,
    ))
}

fn adapter_report(
    version: &str,
    covered: Vec<String>,
    unsupported: Vec<String>,
    diagnostics: Vec<Diagnostic>,
    source: &str,
    dry_run: bool,
) -> AdapterReport {
    let raw_hash = if dry_run {
        String::new()
    } else {
        use sha2::{Digest, Sha256};
        Sha256::digest(source.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let status = if diagnostics
        .iter()
        .any(|d| matches!(d.severity, Severity::Error))
    {
        ImportStatus::Invalid
    } else if !unsupported.is_empty() {
        ImportStatus::Partial
    } else {
        ImportStatus::Converted
    };
    AdapterReport {
        source_version: version.into(),
        status,
        diagnostics,
        covered_fields: covered,
        unsupported,
        raw_hash,
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReloadResult {
    pub committed: bool,
    pub active_hash: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Debug, Default)]
pub struct StagedReload<T> {
    pub active: Option<T>,
    pub active_hash: Option<String>,
}
impl<T> StagedReload<T> {
    pub fn reload<E>(
        &mut self,
        hash: String,
        parser: impl FnOnce() -> Result<T, E>,
        diagnostics: Vec<Diagnostic>,
    ) -> ReloadResult {
        match parser() {
            Ok(value) => {
                self.active = Some(value);
                self.active_hash = Some(hash.clone());
                ReloadResult {
                    committed: true,
                    active_hash: Some(hash),
                    diagnostics,
                }
            }
            Err(_) => ReloadResult {
                committed: false,
                active_hash: self.active_hash.clone(),
                diagnostics,
            },
        }
    }
}

pub fn import_vault_economy(
    file: &str,
    source: &str,
) -> Result<(Economy, AdapterReport), ImportError> {
    let report = import_vault_config(file, source, false)?;
    let source = strip_utf8_bom(source);
    let root = parse_yaml(source)?;
    let currency = root
        .as_mapping()
        .and_then(|map| value_for(map, "currency-name"))
        .and_then(Value::as_str)
        .unwrap_or("coins");
    Ok((Economy::new(currency), report))
}

pub fn import_luckperms_engine(
    file: &str,
    source: &str,
) -> Result<(PermissionEngine, AdapterReport), ImportError> {
    let mut report = import_luckperms_config(file, source, false)?;
    let source = strip_utf8_bom(source);
    let root = parse_yaml(source)?;
    let Some(root) = root.as_mapping() else {
        return Err(ImportError::NotMapping);
    };
    let mut engine = PermissionEngine::default();
    if let Some(groups) = map_value(root, "groups") {
        for (name, raw) in groups {
            let Some(name) = name.as_str() else { continue };
            if raw.as_mapping().is_none() {
                report.diagnostics.push(diag(
                    file,
                    source,
                    &format!("$.groups.{name}"),
                    "LP_INVALID_TYPE",
                    Severity::Error,
                    "group definition must be a mapping",
                ));
                continue;
            }
            engine.groups.insert(name.into(), parse_group(name, raw));
        }
    }
    if let Some(users) = map_value(root, "users") {
        for (id, raw) in users {
            let Some(id_text) = id.as_str() else { continue };
            let Some(id) = uuid::Uuid::parse_str(id_text).ok() else {
                report.diagnostics.push(diag(
                    file,
                    source,
                    &format!("$.users.{id_text}"),
                    "LP_INVALID_ID",
                    Severity::Error,
                    "user key must be a UUID",
                ));
                continue;
            };
            if raw.as_mapping().is_none() {
                report.diagnostics.push(diag(
                    file,
                    source,
                    &format!("$.users.{id_text}"),
                    "LP_INVALID_TYPE",
                    Severity::Error,
                    "user definition must be a mapping",
                ));
                continue;
            }
            engine.users.insert(id, parse_user(id, raw));
        }
    }
    if report
        .diagnostics
        .iter()
        .any(|diagnostic| matches!(diagnostic.severity, Severity::Error))
    {
        report.status = ImportStatus::Invalid;
    } else if report
        .diagnostics
        .iter()
        .any(|diagnostic| matches!(diagnostic.severity, Severity::Warning))
    {
        report.status = ImportStatus::Partial;
    }
    Ok((engine, report))
}
fn value_for<'a>(map: &'a serde_yaml::Mapping, key: &str) -> Option<&'a Value> {
    map.iter()
        .find(|(k, _)| {
            k.as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(key))
        })
        .map(|(_, value)| value)
}
fn map_value<'a>(map: &'a serde_yaml::Mapping, key: &str) -> Option<&'a serde_yaml::Mapping> {
    value_for(map, key).and_then(Value::as_mapping)
}
fn parse_group(name: &str, raw: &Value) -> Group {
    let map = raw.as_mapping();
    let options = map
        .and_then(|map| value_for(map, "options"))
        .and_then(Value::as_mapping);
    let weight = map
        .and_then(|map| value_for(map, "weight"))
        .or_else(|| options.and_then(|options| value_for(options, "weight")))
        .and_then(integer_value)
        .unwrap_or(0);
    Group {
        name: name.into(),
        weight: weight as i32,
        permissions: parse_nodes(map.and_then(|m| value_for(m, "permissions"))),
        parents: parse_parents(map),
        prefix: parse_chat_field(map, "prefix", "prefixes"),
        suffix: parse_chat_field(map, "suffix", "suffixes"),
        meta: parse_meta(map.and_then(|m| value_for(m, "meta"))),
    }
}
fn parse_user(id: uuid::Uuid, raw: &Value) -> User {
    let map = raw.as_mapping();
    let mut groups = Vec::new();
    append_unique(
        &mut groups,
        parse_group_names(map.and_then(|m| value_for(m, "groups"))),
    );
    append_unique(
        &mut groups,
        parse_group_names(map.and_then(|m| value_for(m, "parents"))),
    );
    if let Some(primary) = map
        .and_then(|m| value_for(m, "primary-group"))
        .and_then(Value::as_str)
    {
        append_unique(&mut groups, [primary.to_string()]);
    }
    if let Some(group) = map.and_then(|m| value_for(m, "group")) {
        if let Some(primary) = group
            .as_mapping()
            .and_then(|m| value_for(m, "primary"))
            .and_then(Value::as_str)
        {
            append_unique(&mut groups, [primary.to_string()]);
        } else if let Some(group) = group.as_str() {
            append_unique(&mut groups, [group.to_string()]);
        }
    }
    let mut meta = parse_meta(map.and_then(|m| value_for(m, "meta")));
    if let Some(prefix) = parse_chat_field(map, "prefix", "prefixes") {
        meta.entry("prefix".into()).or_insert(prefix);
    }
    if let Some(suffix) = parse_chat_field(map, "suffix", "suffixes") {
        meta.entry("suffix".into()).or_insert(suffix);
    }
    if let Some(weight) = map
        .and_then(|m| value_for(m, "weight"))
        .and_then(integer_value)
    {
        meta.entry("weight".into()).or_insert(weight.to_string());
    }
    User {
        id,
        groups,
        permissions: parse_nodes(map.and_then(|m| value_for(m, "permissions"))),
        meta,
    }
}
fn parse_nodes(value: Option<&Value>) -> Vec<PermissionNode> {
    let mut nodes = Vec::new();
    match value {
        Some(Value::Sequence(values)) => {
            for raw in values {
                nodes.extend(parse_node_entry(raw));
            }
        }
        Some(Value::Mapping(map)) => nodes.extend(parse_node_mapping(map)),
        Some(Value::String(node)) => nodes.push(permission_node(node, None)),
        _ => {}
    }
    nodes
}
fn parse_meta(value: Option<&Value>) -> HashMap<String, String> {
    let mut meta = HashMap::new();
    match value {
        Some(Value::Mapping(map)) => parse_meta_mapping(map, &mut meta),
        Some(Value::Sequence(values)) => {
            for value in values {
                if let Some(map) = value.as_mapping() {
                    parse_meta_mapping(map, &mut meta);
                }
            }
        }
        _ => {}
    }
    meta
}
fn value_string(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(String::from)
        .or_else(|| value.as_bool().map(|v| v.to_string()))
        .or_else(|| value.as_i64().map(|v| v.to_string()))
        .or_else(|| value.as_f64().map(|v| v.to_string()))
}

fn integer_value(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
}

fn bool_value_from(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_bool)
        .or_else(|| {
            value.and_then(Value::as_str).and_then(|value| match value {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            })
        })
        .unwrap_or(true)
}

fn parse_parents(map: Option<&serde_yaml::Mapping>) -> Vec<String> {
    let mut parents = Vec::new();
    for key in ["parents", "inheritance"] {
        if let Some(value) = map.and_then(|map| value_for(map, key)) {
            append_unique(&mut parents, parse_group_names(Some(value)));
        }
    }
    parents
}

fn parse_group_names(value: Option<&Value>) -> Vec<String> {
    let mut groups = Vec::new();
    match value {
        Some(Value::String(name)) => groups.push(name.clone()),
        Some(Value::Sequence(values)) => {
            for value in values {
                match value {
                    Value::String(name) => groups.push(name.clone()),
                    Value::Mapping(map) => {
                        if let Some(name) = value_for(map, "group")
                            .or_else(|| value_for(map, "parent"))
                            .or_else(|| value_for(map, "name"))
                            .and_then(Value::as_str)
                        {
                            groups.push(name.into());
                        } else {
                            groups.extend(parse_group_names(Some(value)));
                        }
                    }
                    _ => {}
                }
            }
        }
        Some(Value::Mapping(map)) => {
            if let Some(name) = value_for(map, "group")
                .or_else(|| value_for(map, "parent"))
                .or_else(|| value_for(map, "name"))
                .and_then(Value::as_str)
            {
                groups.push(name.into());
            } else {
                for (key, value) in map {
                    let Some(name) = key.as_str() else { continue };
                    if !["contexts", "context", "expiry", "value"].contains(&name) {
                        groups.push(name.into());
                    }
                    if let Some(nested) = value.as_mapping() {
                        groups.extend(parse_group_names(Some(&Value::Mapping(nested.clone()))));
                    }
                }
            }
        }
        _ => {}
    }
    groups
}

fn append_unique<I>(target: &mut Vec<String>, values: I)
where
    I: IntoIterator<Item = String>,
{
    for value in values {
        if !target.iter().any(|existing| existing == &value) {
            target.push(value);
        }
    }
}

fn parse_node_entry(value: &Value) -> Vec<PermissionNode> {
    match value {
        Value::String(node) => vec![permission_node(node, None)],
        Value::Mapping(map) => parse_node_mapping(map),
        _ => Vec::new(),
    }
}

fn parse_node_mapping(map: &serde_yaml::Mapping) -> Vec<PermissionNode> {
    if let Some(node) = value_for(map, "node")
        .or_else(|| value_for(map, "permission"))
        .and_then(Value::as_str)
    {
        return vec![permission_node(node, Some(map))];
    }
    map.iter()
        .filter_map(|(key, value)| {
            let node = key.as_str()?;
            let mut parsed = permission_node(node, value.as_mapping());
            if value.as_mapping().is_none() {
                parsed.value = bool_value_from(Some(value));
            }
            Some(parsed)
        })
        .collect()
}

fn permission_node(node: &str, metadata: Option<&serde_yaml::Mapping>) -> PermissionNode {
    let value = metadata
        .and_then(|map| value_for(map, "value"))
        .map(|value| bool_value_from(Some(value)))
        .unwrap_or(true);
    let expiry_tick = metadata
        .and_then(|map| value_for(map, "expiry_tick").or_else(|| value_for(map, "expiry")))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| integer_value(value).and_then(|v| u64::try_from(v).ok()))
        });
    let contexts = metadata
        .and_then(|map| value_for(map, "contexts").or_else(|| value_for(map, "context")))
        .map(parse_contexts)
        .unwrap_or_default();
    PermissionNode {
        node: node.into(),
        value,
        expiry_tick,
        contexts,
    }
}

fn parse_contexts(value: &Value) -> HashMap<String, String> {
    value
        .as_mapping()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| Some((key.as_str()?.into(), value_string(value)?)))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_meta_mapping(map: &serde_yaml::Mapping, target: &mut HashMap<String, String>) {
    if let (Some(key), Some(value)) = (
        value_for(map, "key").and_then(Value::as_str),
        value_for(map, "value").and_then(value_string),
    ) {
        target.insert(key.into(), value);
        return;
    }
    for (key, value) in map {
        let Some(key) = key.as_str() else { continue };
        let value = value
            .as_mapping()
            .and_then(|map| value_for(map, "value"))
            .and_then(value_string)
            .or_else(|| value_string(value));
        if let Some(value) = value {
            target.insert(key.into(), value);
        }
    }
}

fn parse_chat_field(
    map: Option<&serde_yaml::Mapping>,
    singular: &str,
    plural: &str,
) -> Option<String> {
    let direct = map.and_then(|map| value_for(map, singular));
    let options = map
        .and_then(|map| value_for(map, "options"))
        .and_then(Value::as_mapping)
        .and_then(|options| value_for(options, singular));
    direct.or(options).and_then(chat_value).or_else(|| {
        map.and_then(|map| value_for(map, plural))
            .and_then(chat_value)
    })
}

fn chat_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Sequence(values) => values
            .iter()
            .filter_map(chat_candidate)
            .max_by_key(|(_, priority)| *priority)
            .map(|(value, _)| value),
        Value::Mapping(map) => {
            if let Some(value) = value_for(map, "value").and_then(value_string) {
                return Some(value);
            }
            map.iter()
                .filter_map(|(key, value)| {
                    let key = key.as_str()?.to_string();
                    let priority = value
                        .as_mapping()
                        .and_then(|map| value_for(map, "priority"))
                        .and_then(integer_value)
                        .unwrap_or(0);
                    Some((key, priority))
                })
                .max_by_key(|(_, priority)| *priority)
                .map(|(value, _)| value)
        }
        _ => value_string(value),
    }
}

fn chat_candidate(value: &Value) -> Option<(String, i64)> {
    match value {
        Value::String(value) => Some((value.clone(), 0)),
        Value::Mapping(map) => map
            .iter()
            .filter_map(|(key, raw)| {
                let value = key.as_str()?.to_string();
                let priority = raw
                    .as_mapping()
                    .and_then(|map| value_for(map, "priority"))
                    .and_then(integer_value)
                    .unwrap_or(0);
                Some((value, priority))
            })
            .max_by_key(|(_, priority)| *priority),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn staged_reload_commits_success_and_preserves_previous_on_failure() {
        let mut staged = StagedReload::<String>::default();
        let first = staged.reload("hash-a".into(), || Ok::<_, ()>("active-a".into()), vec![]);
        assert!(first.committed);
        assert_eq!(staged.active.as_deref(), Some("active-a"));
        let second = staged.reload("hash-b".into(), || Err::<String, _>(()), vec![]);
        assert!(!second.committed);
        assert_eq!(second.active_hash.as_deref(), Some("hash-a"));
        assert_eq!(staged.active.as_deref(), Some("active-a"));
    }
    #[test]
    fn imports_and_reports_unknown_fields() {
        let yaml = "Mobs:\n  Goblin:\n    Type: ZOMBIE\n    Health: 20\n    Damage: 3\n    Mystery: true\n";
        let report = import_mythicmobs("fixtures/compat/mythicmobs/basic.yml", yaml, true).unwrap();
        assert_eq!(report.status, ImportStatus::Partial);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "MM_UNKNOWN_FIELD"));
        assert_eq!(report.ir_hash, None);
        assert_eq!(report.document.unwrap().entities[0].id, "Goblin");
    }
    #[test]
    fn adapter_reports_coverage_and_unsupported_fields() {
        let paper = import_server_properties(
            "server.properties",
            "motd=Mythicraft\nonline-mode=true\nunknown-paper-option=true\n",
            false,
        );
        assert_eq!(paper.status, ImportStatus::Partial);
        assert!(paper.covered_fields.contains(&"motd".to_string()));
        assert!(paper
            .unsupported
            .contains(&"unknown-paper-option".to_string()));
        assert!(!paper.raw_hash.is_empty());
        let vault = import_vault_config(
            "config.yml",
            "currency-name: gold\nstarting-balance: 10\nlegacy: true\n",
            true,
        )
        .unwrap();
        assert_eq!(vault.status, ImportStatus::Partial);
        assert!(vault.raw_hash.is_empty());
        let luckperms = import_luckperms_config(
            "config.yml",
            "users: {}\ngroups: {}\nbackend: mysql\n",
            false,
        )
        .unwrap();
        assert_eq!(luckperms.status, ImportStatus::Partial);
        assert!(luckperms
            .diagnostics
            .iter()
            .any(|d| d.code == "LP_UNSUPPORTED_FIELD"));
    }
    #[test]
    fn compiles_skill_mechanics_and_triggers() {
        let yaml = "Mobs:\n  Mage:\n    Type: ZOMBIE\n    Health: 30\n    Skills:\n      - \"condition{type=healthbelow;value=25} | damage{amount=7} @target ~onAttack\"\n      - \"heal{amount=3} @self ~onTimer20\"\n      - \"mystic{foo=1} @target ~onSpawn\"\n";
        let report = import_mythicmobs("skills.yml", yaml, false).unwrap();
        let entity = &report.document.unwrap().entities[0];
        assert_eq!(entity.skills.len(), 2);
        assert_eq!(entity.skills[0].conditions.len(), 1);
        assert_eq!(entity.skills[0].effects.len(), 1);
        assert!(entity
            .triggers
            .iter()
            .any(|t| matches!(t, Trigger::Timer { ticks: 20 })));
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "MM_UNKNOWN_MECHANIC"));
        assert!(report.ir_hash.is_some());
    }

    #[test]
    fn yaml_imports_share_bom_normalization_and_vault_init_diagnostic() {
        let vault = import_vault_config(
            "vault.yml",
            "\u{feff}currency-name: gold\nstarting-balance: 100\nfractional-digits: 2\n",
            true,
        )
        .unwrap();
        assert_eq!(vault.status, ImportStatus::Converted);
        assert!(vault
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "VAULT_ECONOMY_INIT"));

        let mythicmobs = import_mythicmobs(
            "mobs.yml",
            "\u{feff}Mobs:\n  Goblin:\n    Type: ZOMBIE\n",
            true,
        )
        .unwrap();
        assert_eq!(mythicmobs.status, ImportStatus::Converted);
    }

    #[test]
    fn imports_common_luckperms_yaml_shapes_into_existing_engine() {
        let source = "\u{feff}groups:\n  default:\n    permissions:\n      - base.node\n  vip:\n    inheritance:\n      - group: default\n    options:\n      prefix: '[VIP]'\n      suffix: '!'\n      weight: 20\n    meta:\n      - color:\n          value: gold\n    permissions:\n      - special.node:\n          value: true\n          expiry: 1200\n          context:\n            world: arena\nusers:\n  00000000-0000-0000-0000-000000000001:\n    primary-group: vip\n    parents:\n      - vip\n    meta:\n      - title:\n          value: Hero\n";
        let (engine, report) = import_luckperms_engine("users.yml", source).unwrap();
        let id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let group = engine.groups.get("vip").unwrap();
        assert_eq!(group.parents, vec!["default"]);
        assert_eq!(group.prefix.as_deref(), Some("[VIP]"));
        assert_eq!(group.suffix.as_deref(), Some("!"));
        assert_eq!(group.weight, 20);
        assert_eq!(group.meta.get("color").map(String::as_str), Some("gold"));
        assert_eq!(group.permissions[0].expiry_tick, Some(1200));
        assert_eq!(
            group.permissions[0]
                .contexts
                .get("world")
                .map(String::as_str),
            Some("arena")
        );
        assert_eq!(engine.users.get(&id).unwrap().groups, vec!["vip"]);
        assert_eq!(engine.meta(id, "title").as_deref(), Some("Hero"));
        assert_eq!(report.status, ImportStatus::Converted);
    }
}
