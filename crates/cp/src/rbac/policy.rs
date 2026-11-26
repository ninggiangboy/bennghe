// SPDX-License-Identifier: AGPL-3.0-only
//! The permission matrix. Every operation in the API names one `action`; this file says what
//! scope that action lives at and the lowest role that may perform it.
//!
//! Adding a route without an entry here, or with a different scope than the one listed, fails the
//! `all_routes` test, which is a CI gate.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Developer,
    Admin,
    Owner,
}

pub const ALL_ROLES: [Role; 4] = [Role::Viewer, Role::Developer, Role::Admin, Role::Owner];

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Developer => "developer",
            Role::Admin => "admin",
            Role::Owner => "owner",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        ALL_ROLES.into_iter().find(|r| r.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Reachable without signing in.
    Public,
    /// Needs a signed-in user but concerns only that user's own account.
    #[serde(rename = "self")]
    Myself,
    Global,
    Cluster,
    Project,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Public => "public",
            Scope::Myself => "self",
            Scope::Global => "global",
            Scope::Cluster => "cluster",
            Scope::Project => "project",
        }
    }

    /// Name of the path parameter that carries the scope's id.
    pub fn path_param(self) -> Option<&'static str> {
        match self {
            Scope::Cluster => Some("cluster_id"),
            Scope::Project => Some("project_id"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Rule {
    pub action: &'static str,
    pub scope: Scope,
    /// Lowest role allowed. `None` for public and self-service actions.
    pub min_role: Option<Role>,
}

const fn rule(action: &'static str, scope: Scope, min_role: Option<Role>) -> Rule {
    Rule { action, scope, min_role }
}

use Role::{Admin, Developer, Owner, Viewer};
use Scope::{Global, Myself, Project, Public};

pub static RULES: &[Rule] = &[
    // Public: no session needed.
    rule("health.read", Public, None),
    rule("setup.status", Public, None),
    rule("setup.create", Public, None),
    rule("auth.login", Public, None),
    rule("auth.login_totp", Public, None),
    // The signed-in user's own account.
    rule("auth.session.read", Myself, None),
    rule("auth.logout", Myself, None),
    rule("auth.reauth", Myself, None),
    rule("account.totp.begin", Myself, None),
    rule("account.totp.confirm", Myself, None),
    rule("account.totp.disable", Myself, None),
    rule("account.api_token.list", Myself, None),
    rule("account.api_token.create", Myself, None),
    rule("account.api_token.revoke", Myself, None),
    rule("project.list", Myself, None),
    // Installation-wide.
    rule("settings.read", Global, Some(Admin)),
    rule("settings.update", Global, Some(Owner)),
    rule("audit.read", Global, Some(Admin)),
    rule("user.totp_reset", Global, Some(Owner)),
    rule("project.create", Global, Some(Admin)),
    // Inside one project.
    rule("project.read", Project, Some(Viewer)),
    rule("project.update", Project, Some(Admin)),
    rule("project.delete", Project, Some(Admin)),
    rule("project.member.list", Project, Some(Developer)),
    rule("project.member.set", Project, Some(Admin)),
    rule("project.member.remove", Project, Some(Admin)),
];

pub fn rule_for(action: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.action == action)
}

/// Actions that need a password or code check within the last five minutes. API tokens can never
/// satisfy this, so these operations are possible only in the web UI. Later phases add the routes
/// named here that do not exist yet and must mark them `reauth: true`.
pub static REAUTH_ACTIONS: &[&str] = &[
    "account.api_token.create",
    "account.totp.disable",
    "user.totp_reset",
    "project.delete",
    // Reserved for routes that arrive in later phases.
    "recovery_kit.download",
    "secret.reveal",
    "service.privileged.enable",
    "database.public.enable",
    "cluster.delete",
];

/// What a user who must still enrol a second factor may do.
pub static ENROLMENT_ACTIONS: &[&str] = &["auth.session.read", "auth.logout", "auth.reauth", "account.totp.begin", "account.totp.confirm"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_unique() {
        let mut names: Vec<_> = RULES.iter().map(|r| r.action).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
    }

    #[test]
    fn roles_are_ordered_by_privilege() {
        assert!(Role::Owner > Role::Admin && Role::Admin > Role::Developer && Role::Developer > Role::Viewer);
    }

    #[test]
    fn role_only_rules_have_a_minimum_and_others_do_not() {
        for r in RULES {
            let needs_role = matches!(r.scope, Scope::Global | Scope::Cluster | Scope::Project);
            assert_eq!(r.min_role.is_some(), needs_role, "{}", r.action);
        }
    }

    #[test]
    fn enrolment_actions_are_real_rules() {
        for a in ENROLMENT_ACTIONS {
            assert!(rule_for(a).is_some(), "{a}");
        }
    }
}
