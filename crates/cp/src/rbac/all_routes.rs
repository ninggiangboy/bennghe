// SPDX-License-Identifier: AGPL-3.0-only
//! CI gate for access control. These tests read the OpenAPI document generated from the real
//! routes, so a new route cannot slip in without a declared permission and a matrix row.
use std::collections::{BTreeSet, HashMap};

use uuid::Uuid;

use super::{
    decision::{Denial, Grants, Subject, decide},
    perms::{PermTable, operations},
    policy::{ALL_ROLES, REAUTH_ACTIONS, RULES, Role, Scope, rule_for},
};

fn spec() -> utoipa::openapi::OpenApi {
    crate::api::openapi()
}

#[test]
fn every_operation_declares_a_valid_permission() {
    if let Err(problems) = PermTable::from_openapi(&spec()) {
        panic!("route permissions are invalid:\n  {}", problems.join("\n  "));
    }
}

#[test]
fn every_operation_has_a_unique_id_and_a_tag() {
    let api = spec();
    let mut seen = BTreeSet::new();
    for (path, item) in &api.paths.paths {
        for (method, op) in operations(item) {
            let id = op.operation_id.clone().unwrap_or_default();
            assert!(!id.is_empty(), "{method} {path} has no operation id");
            assert!(seen.insert(id.clone()), "duplicate operation id {id}");
            assert!(op.tags.as_ref().is_some_and(|t| !t.is_empty()), "{method} {path} has no tag");
        }
    }
}

#[test]
fn every_matrix_row_is_used_by_a_route() {
    let table = PermTable::from_openapi(&spec()).unwrap();
    let used: BTreeSet<&str> = table.iter().map(|(_, p)| p.action.as_str()).collect();
    for rule in RULES {
        assert!(used.contains(rule.action), "matrix row `{}` has no route; remove it or add the route", rule.action);
    }
}

#[test]
fn each_action_is_declared_by_exactly_one_route() {
    let table = PermTable::from_openapi(&spec()).unwrap();
    let mut by_action: HashMap<&str, Vec<String>> = HashMap::new();
    for ((method, path), perm) in table.iter() {
        by_action.entry(perm.action.as_str()).or_default().push(format!("{method} {path}"));
    }
    for (action, routes) in by_action {
        assert_eq!(routes.len(), 1, "action `{action}` is shared by {routes:?}; give each route its own action");
    }
}

#[test]
fn the_reauth_list_marks_exactly_the_routes_that_exist() {
    let table = PermTable::from_openapi(&spec()).unwrap();
    for (route, perm) in table.iter() {
        assert_eq!(perm.reauth, REAUTH_ACTIONS.contains(&perm.action.as_str()), "{route:?}");
    }
}

/// Independent statement of the rules, written as plainly as possible, to compare with `decide`.
fn expected(action: &str, who: &Who, project: Uuid, other_project: Uuid, target: Uuid) -> bool {
    let rule = rule_for(action).unwrap();
    let min = rule.min_role;
    let role_in =
        |grants: &Grants, id: Uuid| [grants.global, grants.scoped.get(&(Scope::Project, id)).copied()].into_iter().flatten().max();
    match (rule.scope, who) {
        (Scope::Public, _) => true,
        (Scope::Myself, Who::User(_)) => true,
        (Scope::Myself, Who::Token { .. }) => false,
        (Scope::Global, Who::User(g)) => g.global.is_some_and(|r| Some(r) >= min),
        (Scope::Global, Who::Token { .. }) => false,
        (Scope::Project, Who::User(g)) => role_in(g, target).is_some_and(|r| Some(r) >= min),
        (Scope::Project, Who::Token { role, projects, owner }) => {
            let _ = (project, other_project);
            projects.contains(&target) && role_in(owner, target).is_some_and(|have| Some(have.min(*role)) >= min)
        }
        (Scope::Cluster, Who::User(g)) => g.global.is_some_and(|r| Some(r) >= min),
        (Scope::Cluster, Who::Token { .. }) => false,
    }
}

enum Who {
    User(Grants),
    Token { role: Role, projects: Vec<Uuid>, owner: Grants },
}

fn principals(project: Uuid, other: Uuid) -> Vec<(String, Who)> {
    let mut out = vec![("no roles".to_owned(), Who::User(Grants::default()))];
    for r in ALL_ROLES {
        out.push((format!("global {r:?}"), Who::User(Grants { global: Some(r), ..Default::default() })));
        out.push((
            format!("project {r:?}"),
            Who::User(Grants { scoped: HashMap::from([((Scope::Project, project), r)]), ..Default::default() }),
        ));
        out.push((
            format!("other project {r:?}"),
            Who::User(Grants { scoped: HashMap::from([((Scope::Project, other), r)]), ..Default::default() }),
        ));
    }
    for token_role in [Role::Viewer, Role::Developer, Role::Admin] {
        for owner_role in ALL_ROLES {
            for (label, projects) in [("listed", vec![project]), ("unlisted", vec![other])] {
                let owner = Grants { scoped: HashMap::from([((Scope::Project, project), owner_role)]), ..Default::default() };
                out.push((
                    format!("token {token_role:?} ({label}) of project {owner_role:?}"),
                    Who::Token { role: token_role, projects, owner },
                ));
            }
        }
        let owner = Grants { global: Some(Role::Owner), ..Default::default() };
        out.push((format!("token {token_role:?} of global owner"), Who::Token { role: token_role, projects: vec![project], owner }));
    }
    out
}

#[test]
fn every_operation_times_every_principal_matches_the_rules() {
    let table = PermTable::from_openapi(&spec()).unwrap();
    let (project, other) = (Uuid::new_v4(), Uuid::new_v4());
    let mut checked = 0;
    for (route, perm) in table.iter() {
        let rule = rule_for(&perm.action).unwrap();
        for (label, who) in principals(project, other) {
            // Each principal calls the route aimed at `project`; scoped routes get its id.
            let scope_id = rule.scope.path_param().map(|_| project);
            let subject = match &who {
                Who::User(g) => Subject::User(g),
                Who::Token { role, projects, owner } => Subject::Token { role: *role, project_ids: projects, owner },
            };
            let got = decide(rule, &subject, scope_id).is_ok();
            assert_eq!(got, expected(&perm.action, &who, project, other, project), "{route:?} as {label}");
            checked += 1;
        }
    }
    assert!(checked > 500, "the sweep should cover many combinations, ran {checked}");
}

#[test]
fn scoped_routes_without_an_id_are_refused() {
    let grants = Grants { global: Some(Role::Owner), ..Default::default() };
    let rule = rule_for("project.read").unwrap();
    assert_eq!(decide(rule, &Subject::User(&grants), None), Err(Denial::MissingScopeId));
}

#[test]
fn tokens_are_confined_to_project_scope() {
    let table = PermTable::from_openapi(&spec()).unwrap();
    let project = Uuid::new_v4();
    let owner = Grants { global: Some(Role::Owner), ..Default::default() };
    let ids = [project];
    let token = Subject::Token { role: Role::Admin, project_ids: &ids, owner: &owner };
    for (route, perm) in table.iter() {
        let rule = rule_for(&perm.action).unwrap();
        if matches!(rule.scope, Scope::Myself | Scope::Global | Scope::Cluster) {
            assert!(decide(rule, &token, Some(project)).is_err(), "{route:?} must refuse API tokens");
        }
    }
}

/// `api/openapi.yaml` is generated, committed and must match what the code produces.
/// Run `make gen` to refresh it.
#[test]
fn committed_openapi_matches_the_code() {
    let generated = spec().to_yaml().unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../api/openapi.yaml");
    if std::env::var_os("BN_UPDATE_OPENAPI").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(committed == generated, "api/openapi.yaml is out of date; run `make gen` and commit the result");
}

#[test]
fn a_route_without_x_permission_turns_the_gate_red() {
    let mut api = spec();
    api.paths.paths.get_mut("/api/v1/health").unwrap().get.as_mut().unwrap().extensions = None;
    let problems = PermTable::from_openapi(&api).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("GET /api/v1/health") && p.contains("missing")), "{problems:?}");
}

#[test]
fn an_action_missing_from_the_matrix_turns_the_gate_red() {
    let mut api = spec();
    let op = api.paths.paths.get_mut("/api/v1/health").unwrap().get.as_mut().unwrap();
    let mut ext = utoipa::openapi::extensions::Extensions::default();
    ext.insert("x-permission".to_owned(), serde_json::json!({"action": "health.invented", "scope": "public"}));
    op.extensions = Some(ext);
    let problems = PermTable::from_openapi(&api).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("health.invented") && p.contains("no row")), "{problems:?}");
}

#[test]
fn a_reauth_action_without_the_flag_turns_the_gate_red() {
    let mut api = spec();
    let op = api.paths.paths.get_mut("/api/v1/projects/{project_id}").unwrap().delete.as_mut().unwrap();
    let mut ext = utoipa::openapi::extensions::Extensions::default();
    ext.insert("x-permission".to_owned(), serde_json::json!({"action": "project.delete", "scope": "project"}));
    op.extensions = Some(ext);
    let problems = PermTable::from_openapi(&api).unwrap_err();
    assert!(problems.iter().any(|p| p.contains("reauth: true")), "{problems:?}");
}
