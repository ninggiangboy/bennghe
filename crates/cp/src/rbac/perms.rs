// SPDX-License-Identifier: AGPL-3.0-only
//! Maps each route to the permission its OpenAPI operation declares in `x-permission`.
//! The table is built from the generated spec, so the spec is the single source of truth and a
//! route that is missing from it cannot exist.
use std::collections::HashMap;

use serde::Deserialize;
use utoipa::openapi::{OpenApi, PathItem, path::Operation};

use super::policy::{REAUTH_ACTIONS, Scope, rule_for};

pub const EXTENSION: &str = "x-permission";

#[derive(Debug, Clone, Deserialize)]
pub struct Permission {
    pub action: String,
    pub scope: Scope,
    #[serde(default)]
    pub reauth: bool,
}

#[derive(Debug, Default)]
pub struct PermTable {
    by_route: HashMap<(String, String), Permission>,
}

pub fn operations(item: &PathItem) -> Vec<(&'static str, &Operation)> {
    [
        ("GET", &item.get),
        ("PUT", &item.put),
        ("POST", &item.post),
        ("DELETE", &item.delete),
        ("OPTIONS", &item.options),
        ("HEAD", &item.head),
        ("PATCH", &item.patch),
        ("TRACE", &item.trace),
    ]
    .into_iter()
    .filter_map(|(m, op)| op.as_ref().map(|o| (m, o)))
    .collect()
}

impl PermTable {
    /// Builds the table, or returns every problem found so that one run lists them all.
    pub fn from_openapi(api: &OpenApi) -> Result<Self, Vec<String>> {
        let mut table = Self::default();
        let mut problems = Vec::new();
        for (path, item) in &api.paths.paths {
            for (method, op) in operations(item) {
                let route = format!("{method} {path}");
                let Some(raw) = op.extensions.as_ref().and_then(|e| e.get(EXTENSION)) else {
                    problems.push(format!("{route}: missing `{EXTENSION}`"));
                    continue;
                };
                let perm: Permission = match serde_json::from_value(raw.clone()) {
                    Ok(p) => p,
                    Err(e) => {
                        problems.push(format!("{route}: malformed `{EXTENSION}`: {e}"));
                        continue;
                    }
                };
                problems.extend(check(&route, path, &perm));
                table.by_route.insert((method.to_owned(), path.clone()), perm);
            }
        }
        if problems.is_empty() { Ok(table) } else { Err(problems) }
    }

    pub fn lookup(&self, method: &str, path_template: &str) -> Option<&Permission> {
        self.by_route.get(&(method.to_owned(), path_template.to_owned()))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&(String, String), &Permission)> {
        self.by_route.iter()
    }
}

fn check(route: &str, path: &str, perm: &Permission) -> Vec<String> {
    let mut problems = Vec::new();
    match rule_for(&perm.action) {
        None => problems.push(format!("{route}: action `{}` has no row in the permission matrix", perm.action)),
        Some(rule) => {
            if rule.scope != perm.scope {
                problems.push(format!(
                    "{route}: declares scope `{}` but the matrix says `{}` for `{}`",
                    perm.scope.as_str(),
                    rule.scope.as_str(),
                    perm.action
                ));
            }
        }
    }
    if let Some(param) = perm.scope.path_param()
        && !path.contains(&format!("{{{param}}}"))
    {
        problems.push(format!("{route}: scope `{}` requires a `{{{param}}}` path parameter", perm.scope.as_str()));
    }
    let listed = REAUTH_ACTIONS.contains(&perm.action.as_str());
    if listed && !perm.reauth {
        problems.push(format!("{route}: `{}` is a re-authentication action and must set `reauth: true`", perm.action));
    }
    if perm.reauth && !listed {
        problems.push(format!("{route}: sets `reauth: true` but `{}` is not in REAUTH_ACTIONS", perm.action));
    }
    problems
}
