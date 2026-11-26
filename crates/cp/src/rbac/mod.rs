// SPDX-License-Identifier: AGPL-3.0-only
//! Role-based access control: the policy matrix, the decision function and the permission table.
#[cfg(test)]
mod all_routes;
pub mod decision;
pub mod perms;
pub mod policy;

pub use decision::{Denial, Grants, Subject, decide};
pub use perms::{PermTable, Permission};
pub use policy::{Role, Scope};
