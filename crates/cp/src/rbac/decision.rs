// SPDX-License-Identifier: AGPL-3.0-only
//! The authorisation decision, kept free of HTTP and the database so it can be tested exhaustively.
use std::collections::HashMap;

use uuid::Uuid;

use super::policy::{Role, Rule, Scope};

/// The roles one user holds, loaded from `memberships`.
#[derive(Debug, Clone, Default)]
pub struct Grants {
    pub global: Option<Role>,
    pub scoped: HashMap<(Scope, Uuid), Role>,
}

impl Grants {
    /// The strongest role the user has at a scope; a global role counts everywhere.
    pub fn role_at(&self, scope: Scope, id: Uuid) -> Option<Role> {
        self.global.into_iter().chain(self.scoped.get(&(scope, id)).copied()).max()
    }

    /// Whether the user holds owner or admin anywhere. Used by the "require 2FA" setting.
    pub fn is_admin_anywhere(&self) -> bool {
        self.global.into_iter().chain(self.scoped.values().copied()).any(|r| r >= Role::Admin)
    }
}

pub enum Subject<'a> {
    User(&'a Grants),
    Token { role: Role, project_ids: &'a [Uuid], owner: &'a Grants },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denial {
    /// The user has no role, or too low a role, at the scope.
    InsufficientRole,
    /// API tokens may only call project-scoped routes.
    TokenScope,
    /// The token was not issued for this project.
    TokenProject,
    /// A scoped route was reached without the id that names its scope.
    MissingScopeId,
}

impl Denial {
    pub fn reason(self) -> &'static str {
        match self {
            Denial::InsufficientRole => "insufficient_role",
            Denial::TokenScope => "token_scope",
            Denial::TokenProject => "token_project",
            Denial::MissingScopeId => "missing_scope_id",
        }
    }
}

pub fn decide(rule: &Rule, subject: &Subject<'_>, scope_id: Option<Uuid>) -> Result<(), Denial> {
    match (rule.scope, subject) {
        (Scope::Public, _) => Ok(()),
        (Scope::Myself, Subject::User(_)) => Ok(()),
        (Scope::Myself, Subject::Token { .. }) => Err(Denial::TokenScope),
        (Scope::Global, Subject::User(g)) => at_least(rule, g.global),
        (Scope::Global, Subject::Token { .. }) => Err(Denial::TokenScope),
        (scope @ (Scope::Cluster | Scope::Project), Subject::User(g)) => {
            let id = scope_id.ok_or(Denial::MissingScopeId)?;
            at_least(rule, g.role_at(scope, id))
        }
        (Scope::Cluster, Subject::Token { .. }) => Err(Denial::TokenScope),
        (Scope::Project, Subject::Token { role, project_ids, owner }) => {
            let id = scope_id.ok_or(Denial::MissingScopeId)?;
            if !project_ids.contains(&id) {
                return Err(Denial::TokenProject);
            }
            // The token can never do more than its user can do today.
            let current = owner.role_at(Scope::Project, id);
            at_least(rule, current.map(|c| c.min(*role)))
        }
    }
}

fn at_least(rule: &Rule, held: Option<Role>) -> Result<(), Denial> {
    match (held, rule.min_role) {
        (Some(have), Some(need)) if have >= need => Ok(()),
        _ => Err(Denial::InsufficientRole),
    }
}
