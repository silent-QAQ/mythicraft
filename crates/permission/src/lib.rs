use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const PERMISSION_STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionNode {
    pub node: String,
    pub value: bool,
    pub expiry_tick: Option<u64>,
    pub contexts: HashMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub weight: i32,
    pub permissions: Vec<PermissionNode>,
    pub parents: Vec<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub meta: HashMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    pub groups: Vec<String>,
    pub permissions: Vec<PermissionNode>,
    pub meta: HashMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    value: bool,
    specificity: usize,
    source_rank: u8,
}
#[derive(Debug, Default)]
pub struct PermissionEngine {
    pub groups: HashMap<String, Group>,
    pub users: HashMap<Uuid, User>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PermissionState {
    pub schema_version: u32,
    pub groups: HashMap<String, Group>,
    pub users: HashMap<Uuid, User>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PermissionStateError {
    #[error("unsupported permission state schema {0}")]
    UnsupportedSchema(u32),
    #[error("invalid permission state: {0}")]
    Invalid(String),
    #[error("invalid permission state JSON: {0}")]
    Json(String),
}
impl PermissionEngine {
    pub fn snapshot(&self) -> PermissionState {
        PermissionState {
            schema_version: PERMISSION_STATE_VERSION,
            groups: self.groups.clone(),
            users: self.users.clone(),
        }
    }

    pub fn from_snapshot(state: PermissionState) -> Result<Self, PermissionStateError> {
        if state.schema_version != PERMISSION_STATE_VERSION {
            return Err(PermissionStateError::UnsupportedSchema(
                state.schema_version,
            ));
        }
        if state.groups.iter().any(|(name, group)| {
            name.trim().is_empty()
                || group.name != *name
                || group
                    .permissions
                    .iter()
                    .any(|node| node.node.trim().is_empty())
        }) {
            return Err(PermissionStateError::Invalid(
                "group key, group name, or permission node is invalid".into(),
            ));
        }
        if state.users.iter().any(|(id, user)| {
            id != &user.id
                || user
                    .permissions
                    .iter()
                    .any(|node| node.node.trim().is_empty())
        }) {
            return Err(PermissionStateError::Invalid(
                "user key, user id, or permission node is invalid".into(),
            ));
        }
        Ok(Self {
            groups: state.groups,
            users: state.users,
        })
    }

    pub fn to_json(&self) -> Result<String, PermissionStateError> {
        serde_json::to_string_pretty(&self.snapshot())
            .map_err(|error| PermissionStateError::Json(error.to_string()))
    }

    pub fn from_json(source: &str) -> Result<Self, PermissionStateError> {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let state: PermissionState = serde_json::from_str(source)
            .map_err(|error| PermissionStateError::Json(error.to_string()))?;
        Self::from_snapshot(state)
    }

    pub fn check(
        &self,
        user: Uuid,
        node: &str,
        tick: u64,
        context: &HashMap<String, String>,
    ) -> bool {
        let Some(account) = self.users.get(&user) else {
            return false;
        };
        let mut candidates = Vec::new();
        for permission in &account.permissions {
            if let Some(candidate) = self.candidate(permission, node, tick, context, 3) {
                candidates.push(candidate);
            }
        }
        let mut visited = HashSet::new();
        for group_name in &account.groups {
            self.collect_candidates(
                group_name,
                node,
                tick,
                context,
                &mut visited,
                &mut candidates,
                2,
            );
        }
        candidates.sort_by_key(|candidate| (candidate.specificity, candidate.source_rank));
        let Some(best) = candidates.last() else {
            return false;
        };
        let best_score = (best.specificity, best.source_rank);
        candidates
            .iter()
            .rev()
            .find(|candidate| (candidate.specificity, candidate.source_rank) == best_score)
            .map(|candidate| candidate.value)
            .unwrap_or(false)
    }
    pub fn weight(&self, user: Uuid) -> i32 {
        let Some(account) = self.users.get(&user) else {
            return 0;
        };
        account
            .groups
            .iter()
            .filter_map(|name| self.groups.get(name))
            .map(|group| group.weight)
            .max()
            .unwrap_or(0)
    }
    pub fn prefix(&self, user: Uuid) -> Option<String> {
        self.best_group_field(user, |group| group.prefix.clone())
    }
    pub fn suffix(&self, user: Uuid) -> Option<String> {
        self.best_group_field(user, |group| group.suffix.clone())
    }
    pub fn meta(&self, user: Uuid, key: &str) -> Option<String> {
        let account = self.users.get(&user)?;
        account
            .meta
            .get(key)
            .cloned()
            .or_else(|| self.best_group_field(user, |group| group.meta.get(key).cloned()))
    }
    fn best_group_field<T>(&self, user: Uuid, field: impl Fn(&Group) -> Option<T>) -> Option<T> {
        let account = self.users.get(&user)?;
        account
            .groups
            .iter()
            .filter_map(|name| self.groups.get(name))
            .max_by_key(|group| group.weight)
            .and_then(field)
    }
    fn candidate(
        &self,
        permission: &PermissionNode,
        node: &str,
        tick: u64,
        context: &HashMap<String, String>,
        source_rank: u8,
    ) -> Option<Candidate> {
        if permission
            .expiry_tick
            .map(|expiry| expiry <= tick)
            .unwrap_or(false)
            || !permission
                .contexts
                .iter()
                .all(|(key, value)| context.get(key) == Some(value))
        {
            return None;
        }
        let specificity = match_node(&permission.node, node)?;
        Some(Candidate {
            value: permission.value,
            specificity,
            source_rank,
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn collect_candidates(
        &self,
        name: &str,
        node: &str,
        tick: u64,
        context: &HashMap<String, String>,
        visited: &mut HashSet<String>,
        output: &mut Vec<Candidate>,
        source_rank: u8,
    ) {
        if !visited.insert(name.into()) {
            return;
        }
        if let Some(group) = self.groups.get(name) {
            for permission in &group.permissions {
                if let Some(candidate) =
                    self.candidate(permission, node, tick, context, source_rank)
                {
                    output.push(candidate)
                }
            }
            for parent in &group.parents {
                self.collect_candidates(
                    parent,
                    node,
                    tick,
                    context,
                    visited,
                    output,
                    source_rank.saturating_sub(1),
                );
            }
        }
    }
}
fn match_node(pattern: &str, node: &str) -> Option<usize> {
    if pattern == node {
        return Some(10_000 + pattern.len());
    }
    if pattern == "*" {
        return Some(0);
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        if node == prefix || node.starts_with(&format!("{prefix}.")) {
            return Some(prefix.len() + 1);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(
        name: &str,
        value: bool,
        expiry: Option<u64>,
        contexts: HashMap<String, String>,
    ) -> PermissionNode {
        PermissionNode {
            node: name.into(),
            value,
            expiry_tick: expiry,
            contexts,
        }
    }
    #[test]
    fn specificity_negative_and_expiry_are_defined() {
        let user = Uuid::new_v4();
        let mut engine = PermissionEngine::default();
        engine.groups.insert(
            "default".into(),
            Group {
                name: "default".into(),
                permissions: vec![
                    node("mythic.*", true, None, HashMap::new()),
                    node("mythic.cast", false, None, HashMap::new()),
                    node("temporary", true, Some(10), HashMap::new()),
                ],
                ..Default::default()
            },
        );
        engine.users.insert(
            user,
            User {
                id: user,
                groups: vec!["default".into()],
                ..Default::default()
            },
        );
        let ctx = HashMap::new();
        assert!(!engine.check(user, "mythic.cast", 1, &ctx));
        assert!(engine.check(user, "mythic.spawn", 1, &ctx));
        assert!(!engine.check(user, "temporary", 10, &ctx));
    }
    #[test]
    fn context_meta_weight_prefix_and_suffix_work() {
        let user = Uuid::new_v4();
        let mut ctx = HashMap::new();
        ctx.insert("world".into(), "arena".into());
        let mut engine = PermissionEngine::default();
        engine.groups.insert(
            "vip".into(),
            Group {
                name: "vip".into(),
                weight: 20,
                prefix: Some("[VIP]".into()),
                suffix: Some("!".into()),
                meta: HashMap::from([(String::from("color"), String::from("gold"))]),
                permissions: vec![node("arena.enter", true, None, ctx.clone())],
                ..Default::default()
            },
        );
        engine.users.insert(
            user,
            User {
                id: user,
                groups: vec!["vip".into()],
                ..Default::default()
            },
        );
        assert!(engine.check(user, "arena.enter", 0, &ctx));
        assert!(!engine.check(user, "arena.enter", 0, &HashMap::new()));
        assert_eq!(engine.weight(user), 20);
        assert_eq!(engine.prefix(user).as_deref(), Some("[VIP]"));
        assert_eq!(engine.suffix(user).as_deref(), Some("!"));
        assert_eq!(engine.meta(user, "color").as_deref(), Some("gold"));
    }
}
