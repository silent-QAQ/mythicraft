use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use thiserror::Error;

use crate::model::{
    ActionDefinition, ActionType, ArcartxDocument, Control, DocumentKind, PageConfig, ResourceRef,
    UiSettings, UiTask,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFormat {
    Json,
    Yaml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseReport {
    pub document: ArcartxDocument,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("JSON parse failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("YAML parse failed: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("configuration root must be an object")]
    RootNotObject,
    #[error("page_id is missing; pass the ArcartX filename as source_id")]
    MissingPageId,
    #[error("{path}: expected {expected}")]
    InvalidType {
        path: String,
        expected: &'static str,
    },
}

pub fn parse_json(input: &str, source_id: Option<&str>) -> Result<ParseReport, ParseError> {
    let value = serde_json::from_str(input)?;
    parse_value(value, source_id, InputFormat::Json)
}

pub fn parse_yaml(input: &str, source_id: Option<&str>) -> Result<ParseReport, ParseError> {
    let value = serde_yaml::from_str::<Value>(input)?;
    parse_value(value, source_id, InputFormat::Yaml)
}

pub fn parse_auto(input: &str, source_id: Option<&str>) -> Result<ParseReport, ParseError> {
    let first = input.trim_start().chars().next();
    match first {
        Some('{') | Some('[') => parse_json(input, source_id),
        _ => parse_yaml(input, source_id),
    }
}

fn parse_value(
    value: Value,
    source_id: Option<&str>,
    _format: InputFormat,
) -> Result<ParseReport, ParseError> {
    let root = value.as_object().ok_or(ParseError::RootNotObject)?;
    let mut diagnostics = Vec::new();
    check_unknown(
        root,
        &[
            "kind",
            "format_version",
            "id",
            "page_id",
            "version",
            "page_version",
            "nonce",
            "permission",
            "permissions",
            "required_permissions",
            "required_capabilities",
            "capabilities",
            "ui",
            "controls",
            "components",
            "template",
            "tasks",
            "root_control",
            "tip",
            "match",
            "hide",
            "resources",
            "resource",
            "actions",
            "action",
        ],
        "root",
        &mut diagnostics,
    );

    let page_id = first_string(root, &["page_id", "id"])
        .or_else(|| source_id.map(clean_source_id))
        .filter(|value| !value.is_empty())
        .ok_or(ParseError::MissingPageId)?;
    let version = match first_u64(
        root,
        &["page_version", "version"],
        "root.version",
        &mut diagnostics,
    )? {
        Some(version) => version,
        None => {
            diagnostics.push(warning(
                "defaulted_page_version",
                "root.version",
                "ArcartX 配置没有页面版本；兼容层默认使用 1，生产接入应在发布时递增。",
            ));
            1
        }
    };
    let nonce = optional_string(root, "nonce", &mut diagnostics)?;
    let permissions = read_string_list_aliases(
        root,
        &["permissions", "required_permissions", "permission"],
        "root.permissions",
        &mut diagnostics,
    )?;
    let required_capabilities = read_string_list_aliases(
        root,
        &["required_capabilities", "capabilities"],
        "root.required_capabilities",
        &mut diagnostics,
    )?;
    let kind = parse_kind(root.get("kind"), &mut diagnostics);
    let ui = parse_ui_settings(root.get("ui"), &mut diagnostics)?;
    let controls = parse_control_map(
        root.get("controls").or_else(|| root.get("components")),
        "root.controls",
        &mut diagnostics,
    )?;
    let template = parse_control_map(root.get("template"), "root.template", &mut diagnostics)?;
    let tasks = parse_task_map(root.get("tasks"), "root.tasks", &mut diagnostics)?;
    let root_control = root
        .get("root_control")
        .map(|value| parse_control(value, "root_control", &mut diagnostics))
        .transpose()?;
    let (mut match_rules, mut hide_rules) = parse_match_rules(root, &mut diagnostics)?;
    if let Some(value) = ui.values.get("match") {
        match_rules.extend(read_string_list_value(
            value,
            "root.ui.match",
            &mut diagnostics,
        )?);
    }
    if let Some(value) = ui.values.get("hide") {
        hide_rules.extend(read_string_list_value(
            value,
            "root.ui.hide",
            &mut diagnostics,
        )?);
    }
    match_rules.sort();
    match_rules.dedup();
    hide_rules.sort();
    hide_rules.dedup();

    let mut resources = parse_resources(
        root.get("resources").or_else(|| root.get("resource")),
        "root.resources",
        &mut diagnostics,
    )?;
    collect_tilde_resources(&value, &mut resources);

    let mut actions = parse_action_section(
        root.get("actions").or_else(|| root.get("action")),
        &page_id,
        "root.actions",
        &mut diagnostics,
    )?;
    collect_control_actions(&controls, &mut actions);
    collect_control_actions(&template, &mut actions);
    if let Some(control) = &root_control {
        collect_one_control_actions(control, &mut actions);
    }
    collect_ui_actions(&ui, &page_id, &mut actions, &mut diagnostics);
    dedupe_actions(&mut actions, &mut diagnostics);

    Ok(ParseReport {
        document: ArcartxDocument {
            kind,
            page_id,
            version,
            nonce,
            permissions,
            required_capabilities,
            page: PageConfig {
                ui,
                controls,
                template,
                tasks,
                root_control,
                match_rules,
                hide_rules,
            },
            raw_model: value,
            resources,
            actions,
        },
        diagnostics,
    })
}

fn parse_kind(value: Option<&Value>, diagnostics: &mut Vec<Diagnostic>) -> DocumentKind {
    let Some(value) = value else {
        return DocumentKind::Page;
    };
    let Some(kind) = value.as_str() else {
        diagnostics.push(warning(
            "invalid_kind",
            "root.kind",
            "kind 必须是 page、tooltip、resource 或 actions；已按 page 处理。",
        ));
        return DocumentKind::Page;
    };
    match kind.to_ascii_lowercase().as_str() {
        "page" | "ui" => DocumentKind::Page,
        "tooltip" | "tip" => DocumentKind::Tooltip,
        "resource" | "resources" => DocumentKind::Resource,
        "actions" | "action" => DocumentKind::Actions,
        _ => {
            diagnostics.push(warning(
                "unknown_kind",
                "root.kind",
                format!("未知 kind {}；已按 page 处理。", kind),
            ));
            DocumentKind::Page
        }
    }
}

fn parse_ui_settings(
    value: Option<&Value>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<UiSettings, ParseError> {
    let Some(value) = value else {
        return Ok(UiSettings::default());
    };
    let object = expect_object(value, "root.ui")?;
    check_unknown(
        object,
        &[
            "match",
            "hide",
            "itemSize",
            "through",
            "escClose",
            "background",
            "closeDied",
            "show",
            "jei",
            "level",
            "screenScale",
            "action",
            "packetHandler",
            "isHud",
            "defaultOpen",
        ],
        "root.ui",
        diagnostics,
    );
    Ok(UiSettings {
        values: object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    })
}

fn parse_control_map(
    value: Option<&Value>,
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<BTreeMap<String, Control>, ParseError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let object = expect_object(value, path)?;
    let mut result = BTreeMap::new();
    for (id, control) in object {
        result.insert(
            id.clone(),
            parse_control(control, &format!("{}.{}", path, id), diagnostics)?,
        );
    }
    Ok(result)
}

fn parse_control(
    value: &Value,
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Control, ParseError> {
    let object = expect_object(value, path)?;
    check_unknown(
        object,
        &[
            "id",
            "type",
            "val",
            "value",
            "attribute",
            "attributes",
            "effect",
            "effects",
            "action",
            "actions",
            "children",
            "permission",
            "permissions",
        ],
        path,
        diagnostics,
    );
    let id = first_string(object, &["id"])
        .unwrap_or_else(|| path.rsplit('.').next().unwrap_or("control").to_owned());
    let control_type = first_string(object, &["type"]).unwrap_or_else(|| "none".to_owned());
    let value_name = first_string(object, &["val", "value"]);
    let attributes = read_open_map(
        object,
        &["attribute", "attributes"],
        &format!("{}.attribute", path),
    )?;
    let effects = read_open_map(object, &["effect", "effects"], &format!("{}.effect", path))?;
    let actions = read_action_map(
        object,
        &["action", "actions"],
        &format!("{}.action", path),
        diagnostics,
    )?;
    let permissions = read_string_list_aliases(
        object,
        &["permissions", "permission"],
        &format!("{}.permissions", path),
        diagnostics,
    )?;
    let children = parse_control_map(
        object.get("children"),
        &format!("{}.children", path),
        diagnostics,
    )?;
    Ok(Control {
        id,
        control_type,
        value: value_name,
        attributes,
        effects,
        actions,
        children,
        permissions,
    })
}

fn parse_task_map(
    value: Option<&Value>,
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<BTreeMap<String, UiTask>, ParseError> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let object = expect_object(value, path)?;
    let mut result = BTreeMap::new();
    for (id, value) in object {
        let task_path = format!("{}.{}", path, id);
        let task = expect_object(value, &task_path)?;
        check_unknown(
            task,
            &["type", "time", "cycle", "run"],
            &task_path,
            diagnostics,
        );
        result.insert(
            id.clone(),
            UiTask {
                task_type: optional_string(task, "type", diagnostics)?,
                time: task.get("time").cloned(),
                cycle: task.get("cycle").cloned(),
                run: optional_string(task, "run", diagnostics)?,
            },
        );
    }
    Ok(result)
}

fn parse_match_rules(
    root: &Map<String, Value>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(Vec<String>, Vec<String>), ParseError> {
    let mut matches = read_string_list_aliases(root, &["match"], "root.match", diagnostics)?;
    let mut hide = read_string_list_aliases(root, &["hide"], "root.hide", diagnostics)?;
    if let Some(tip) = root.get("tip") {
        let tip = expect_object(tip, "root.tip")?;
        check_unknown(tip, &["match"], "root.tip", diagnostics);
        matches.extend(read_string_list_aliases(
            tip,
            &["match"],
            "root.tip.match",
            diagnostics,
        )?);
    }
    matches.sort();
    matches.dedup();
    hide.sort();
    hide.dedup();
    Ok((matches, hide))
}

fn parse_resources(
    value: Option<&Value>,
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<ResourceRef>, ParseError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                parse_resource(item, None, &format!("{}[{}]", path, index), diagnostics)
            })
            .collect(),
        Value::Object(object) => {
            if object.contains_key("path")
                || object.contains_key("file")
                || object.contains_key("filename")
            {
                return Ok(vec![parse_resource(value, None, path, diagnostics)?]);
            }
            object
                .iter()
                .map(|(id, item)| {
                    parse_resource(item, Some(id), &format!("{}.{}", path, id), diagnostics)
                })
                .collect()
        }
        _ => Err(ParseError::InvalidType {
            path: path.to_owned(),
            expected: "resource object or array",
        }),
    }
}

fn parse_resource(
    value: &Value,
    id_hint: Option<&str>,
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<ResourceRef, ParseError> {
    if let Some(resource_path) = value.as_str() {
        return Ok(ResourceRef {
            id: id_hint.map(str::to_owned),
            path: resource_path.trim_start_matches('~').to_owned(),
            ..ResourceRef::default()
        });
    }
    let object = expect_object(value, path)?;
    check_unknown(
        object,
        &[
            "id",
            "path",
            "file",
            "filename",
            "name",
            "kind",
            "type",
            "hash",
            "sha256",
            "crc64",
            "permission",
            "permissions",
            "metadata",
        ],
        path,
        diagnostics,
    );
    let resource_path = first_string(object, &["path", "file", "filename", "name"])
        .map(|path| path.trim_start_matches('~').to_owned())
        .unwrap_or_default();
    if resource_path.is_empty() {
        diagnostics.push(error(
            "missing_resource_path",
            path,
            "资源引用缺少 path/file/filename。",
        ));
    }
    Ok(ResourceRef {
        id: first_string(object, &["id"]).or_else(|| id_hint.map(str::to_owned)),
        path: resource_path,
        kind: first_string(object, &["kind", "type"]),
        hash: first_string(object, &["hash", "sha256", "crc64"]),
        permissions: read_string_list_aliases(
            object,
            &["permissions", "permission"],
            &format!("{}.permissions", path),
            diagnostics,
        )?,
        metadata: object
            .get("metadata")
            .and_then(Value::as_object)
            .map(|metadata| {
                metadata
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn parse_action_section(
    value: Option<&Value>,
    page_id: &str,
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<ActionDefinition>, ParseError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value {
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                parse_explicit_action(
                    item,
                    None,
                    page_id,
                    &format!("{}[{}]", path, index),
                    diagnostics,
                )
            })
            .collect(),
        Value::Object(object) => {
            if object.contains_key("command")
                || object.contains_key("script")
                || object.contains_key("control_id")
            {
                return Ok(vec![parse_explicit_action(
                    value,
                    None,
                    page_id,
                    path,
                    diagnostics,
                )?]);
            }
            let mut result = Vec::new();
            for (id, action) in object {
                if let Some(command) = action.as_str() {
                    result.push(ActionDefinition {
                        id: id.clone(),
                        control_id: page_id.to_owned(),
                        action_type: action_type_from_name(
                            id,
                            &format!("{}.{}", path, id),
                            diagnostics,
                        ),
                        command: command.to_owned(),
                        permissions: Vec::new(),
                        nonce: None,
                    });
                } else {
                    result.push(parse_explicit_action(
                        action,
                        Some(id),
                        page_id,
                        &format!("{}.{}", path, id),
                        diagnostics,
                    )?);
                }
            }
            Ok(result)
        }
        _ => Err(ParseError::InvalidType {
            path: path.to_owned(),
            expected: "action object or array",
        }),
    }
}

fn parse_explicit_action(
    value: &Value,
    id_hint: Option<&str>,
    page_id: &str,
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<ActionDefinition, ParseError> {
    let object = expect_object(value, path)?;
    check_unknown(
        object,
        &[
            "id",
            "control_id",
            "control",
            "type",
            "action_type",
            "command",
            "script",
            "value",
            "permission",
            "permissions",
            "nonce",
        ],
        path,
        diagnostics,
    );
    let id = first_string(object, &["id"])
        .or_else(|| id_hint.map(str::to_owned))
        .unwrap_or_else(|| path.to_owned());
    let control_id =
        first_string(object, &["control_id", "control"]).unwrap_or_else(|| page_id.to_owned());
    let action_name =
        first_string(object, &["type", "action_type"]).unwrap_or_else(|| "click".to_owned());
    let action_type = action_type_from_name(&action_name, &format!("{}.type", path), diagnostics);
    let command = first_string(object, &["command", "script", "value"]).unwrap_or_default();
    if command.is_empty() {
        diagnostics.push(warning(
            "empty_action_command",
            &format!("{}.command", path),
            "动作没有 command/script/value；仍保留动作标识，接入层应拒绝空动作。",
        ));
    }
    Ok(ActionDefinition {
        id,
        control_id,
        action_type,
        command,
        permissions: read_string_list_aliases(
            object,
            &["permissions", "permission"],
            &format!("{}.permissions", path),
            diagnostics,
        )?,
        nonce: optional_string(object, "nonce", diagnostics)?,
    })
}

fn collect_control_actions(
    controls: &BTreeMap<String, Control>,
    actions: &mut Vec<ActionDefinition>,
) {
    for control in controls.values() {
        collect_one_control_actions(control, actions);
    }
}

fn collect_one_control_actions(control: &Control, actions: &mut Vec<ActionDefinition>) {
    for (event, command) in &control.actions {
        actions.push(ActionDefinition {
            id: format!("{}:{}", control.id, event),
            control_id: control.id.clone(),
            action_type: action_type_from_name_without_diagnostics(event),
            command: command.clone(),
            permissions: control.permissions.clone(),
            nonce: None,
        });
    }
    collect_control_actions(&control.children, actions);
}

fn collect_ui_actions(
    ui: &UiSettings,
    page_id: &str,
    actions: &mut Vec<ActionDefinition>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let Some(value) = ui.values.get("action") else {
        return;
    };
    let Some(object) = value.as_object() else {
        diagnostics.push(warning(
            "invalid_ui_action",
            "root.ui.action",
            "ui.action 必须是动作名到脚本的对象。",
        ));
        return;
    };
    for (event, command) in object {
        let Some(command) = command.as_str() else {
            diagnostics.push(warning(
                "invalid_ui_action_command",
                &format!("root.ui.action.{}", event),
                "ui.action 的动作值不是字符串，已跳过该动作。",
            ));
            continue;
        };
        actions.push(ActionDefinition {
            id: format!("{}:{}", page_id, event),
            control_id: page_id.to_owned(),
            action_type: action_type_from_name_without_diagnostics(event),
            command: command.to_owned(),
            permissions: Vec::new(),
            nonce: None,
        });
    }
}

fn dedupe_actions(actions: &mut Vec<ActionDefinition>, diagnostics: &mut Vec<Diagnostic>) {
    let mut seen = BTreeSet::new();
    actions.retain(|action| {
        if seen.insert(action.id.clone()) {
            true
        } else {
            diagnostics.push(warning(
                "duplicate_action_id",
                &format!("actions.{}", action.id),
                "重复动作 ID；保留第一次出现的定义。",
            ));
            false
        }
    });
}

fn read_action_map(
    object: &Map<String, Value>,
    aliases: &[&str],
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<BTreeMap<String, String>, ParseError> {
    let Some(value) = aliases.iter().find_map(|alias| object.get(*alias)) else {
        return Ok(BTreeMap::new());
    };
    let action_object = expect_object(value, path)?;
    let mut result = BTreeMap::new();
    for (key, value) in action_object {
        if let Some(command) = value.as_str() {
            result.insert(key.clone(), command.to_owned());
        } else {
            diagnostics.push(warning(
                "invalid_control_action",
                &format!("{}.{}", path, key),
                "控件动作值不是字符串，已跳过该动作。",
            ));
        }
    }
    Ok(result)
}

fn read_open_map(
    object: &Map<String, Value>,
    aliases: &[&str],
    path: &str,
) -> Result<BTreeMap<String, Value>, ParseError> {
    let Some(value) = aliases.iter().find_map(|alias| object.get(*alias)) else {
        return Ok(BTreeMap::new());
    };
    let value = expect_object(value, path)?;
    Ok(value
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn read_string_list_aliases(
    object: &Map<String, Value>,
    aliases: &[&str],
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<String>, ParseError> {
    let Some(value) = aliases.iter().find_map(|alias| object.get(*alias)) else {
        return Ok(Vec::new());
    };
    read_string_list_value(value, path, diagnostics)
}

fn read_string_list_value(
    value: &Value,
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<String>, ParseError> {
    match value {
        Value::String(value) => Ok(vec![value.clone()]),
        Value::Array(values) => Ok(values
            .iter()
            .filter_map(|value| {
                if let Some(value) = value.as_str() {
                    Some(value.to_owned())
                } else {
                    diagnostics.push(warning(
                        "invalid_string_list_item",
                        path,
                        "列表中存在非字符串项，已跳过。",
                    ));
                    None
                }
            })
            .collect()),
        _ => Err(ParseError::InvalidType {
            path: path.to_owned(),
            expected: "string or string array",
        }),
    }
}

fn first_u64(
    object: &Map<String, Value>,
    aliases: &[&str],
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<u64>, ParseError> {
    let Some(value) = aliases.iter().find_map(|alias| object.get(*alias)) else {
        return Ok(None);
    };
    if let Some(value) = value.as_u64() {
        return Ok(Some(value));
    }
    if let Some(value) = value.as_str() {
        if let Ok(parsed) = value.parse::<u64>() {
            diagnostics.push(warning(
                "string_numeric_value",
                path,
                "数字字段以字符串形式出现，已转换。",
            ));
            return Ok(Some(parsed));
        }
    }
    Err(ParseError::InvalidType {
        path: path.to_owned(),
        expected: "positive integer",
    })
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<String>, ParseError> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    if let Some(value) = value.as_str() {
        return Ok(Some(value.to_owned()));
    }
    diagnostics.push(warning(
        "invalid_string_field",
        key,
        format!("字段 {} 不是字符串，已忽略。", key),
    ));
    Ok(None)
}

fn first_string(object: &Map<String, Value>, aliases: &[&str]) -> Option<String> {
    aliases.iter().find_map(|alias| {
        object
            .get(*alias)
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

fn expect_object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, ParseError> {
    value.as_object().ok_or_else(|| ParseError::InvalidType {
        path: path.to_owned(),
        expected: "object",
    })
}

fn check_unknown(
    object: &Map<String, Value>,
    allowed: &[&str],
    path: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for key in object.keys().filter(|key| !allowed.contains(&key.as_str())) {
        diagnostics.push(warning(
            "unknown_field",
            &format!("{}.{}", path, key),
            "未知字段未映射到兼容模型；请确认它是 ArcartX 扩展，或补充适配器。",
        ));
    }
}

fn collect_tilde_resources(value: &Value, resources: &mut Vec<ResourceRef>) {
    match value {
        Value::String(value) if value.starts_with('~') && value.len() > 1 => {
            let path = value.trim_start_matches('~').to_owned();
            if !resources.iter().any(|resource| resource.path == path) {
                resources.push(ResourceRef {
                    path,
                    ..ResourceRef::default()
                });
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| collect_tilde_resources(value, resources)),
        Value::Object(values) => values
            .values()
            .for_each(|value| collect_tilde_resources(value, resources)),
        _ => {}
    }
}

fn action_type_from_name(name: &str, path: &str, diagnostics: &mut Vec<Diagnostic>) -> ActionType {
    match action_type_from_name_inner(name) {
        Some(value) => value,
        None => {
            diagnostics.push(warning(
                "unknown_action_type",
                path,
                format!("未知动作类型 {}；已按 click 处理。", name),
            ));
            ActionType::Click
        }
    }
}

fn action_type_from_name_without_diagnostics(name: &str) -> ActionType {
    action_type_from_name_inner(name).unwrap_or(ActionType::Click)
}

fn action_type_from_name_inner(name: &str) -> Option<ActionType> {
    let normalized = name.trim().to_ascii_lowercase();
    let normalized = normalized.strip_prefix("on_").unwrap_or(&normalized);
    let normalized = normalized.strip_prefix("on").unwrap_or(normalized);
    match normalized {
        "click" | "mouse_click" | "press" => Some(ActionType::Click),
        "submit" => Some(ActionType::Submit),
        "change" | "changed" => Some(ActionType::Change),
        "keypress" | "key_press" | "key" => Some(ActionType::KeyPress),
        _ => None,
    }
}

fn clean_source_id(source_id: &str) -> String {
    let normalized_source_id = source_id.replace('\\', "/");
    let filename = normalized_source_id.rsplit('/').next().unwrap_or(source_id);
    let lower = filename.to_ascii_lowercase();
    [".yaml", ".yml", ".json"]
        .iter()
        .find_map(|extension| {
            lower
                .ends_with(extension)
                .then(|| filename[..filename.len() - extension.len()].to_owned())
        })
        .unwrap_or_else(|| filename.to_owned())
}

fn warning(code: &str, path: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Warning,
        code: code.to_owned(),
        path: path.to_owned(),
        message: message.into(),
    }
}

fn error(code: &str, path: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: code.to_owned(),
        path: path.to_owned(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DocumentKind;

    #[test]
    fn parses_real_arcartx_ui_shape_and_unicode_ids() {
        let source = r#"
ui:
  isHud: "true"
  defaultOpen: "false"
  match: [default]
  action:
    click: "Message.chat('opened')"
root_control:
  type: Canvas
  attribute:
    normal: ~ui/background.png
  children:
    按钮:
      type: Text
      action:
        click: "Message.chat('clicked')"
"#;
        let report = parse_yaml(source, Some("plugins/ArcartX/ui/任务面板.yml"))
            .expect("actual ArcartX UI shape should parse");

        assert_eq!(report.document.kind, DocumentKind::Page);
        assert_eq!(report.document.page_id, "任务面板");
        assert_eq!(report.document.page.match_rules, vec!["default"]);
        assert_eq!(
            report.document.page.root_control.as_ref().unwrap().children["按钮"].id,
            "按钮"
        );
        assert!(report
            .document
            .resources
            .iter()
            .any(|resource| resource.path == "ui/background.png"));
        assert!(report
            .document
            .actions
            .iter()
            .any(|action| { action.control_id == "按钮" && action.command.contains("clicked") }));
        assert_eq!(report.document.raw_model["ui"]["isHud"], "true");
    }

    #[test]
    fn parses_json_components_alias_and_explicit_action() {
        let source = r#"
{
  "id": "inventory",
  "version": "2",
  "components": {
    "close": {"type": "Button", "action": {"click": "console:say closed"}}
  },
  "actions": {
    "refresh": {"control_id": "close", "type": "click", "command": "player:help"}
  }
}
"#;
        let report = parse_json(source, None).expect("JSON alias should parse");
        assert_eq!(report.document.page_id, "inventory");
        assert_eq!(report.document.version, 2);
        assert!(report.document.page.controls.contains_key("close"));
        assert!(report
            .document
            .actions
            .iter()
            .any(|action| action.id == "refresh"));
    }
}
