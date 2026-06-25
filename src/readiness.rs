//! Readiness evaluation for issue dependency state.

use std::collections::HashMap;

use uuid::Uuid;

use crate::{
    error::{MissingDependencySnafu, Result},
    model::{Config, Issue},
};

pub(crate) fn issue_is_ready(
    config: &Config,
    issue: &Issue,
    by_id: &HashMap<Uuid, &Issue>,
) -> Result<bool> {
    if config.status_is_resolved(&issue.status) || issue.is_claimed() {
        return Ok(false);
    }

    for blocker_id in &issue.blocked_by {
        let blocker = by_id.get(blocker_id).ok_or_else(|| {
            MissingDependencySnafu {
                issue: issue.reference.clone(),
                blocker: *blocker_id,
            }
            .build()
        })?;
        if !config.status_is_resolved(&blocker.status) {
            return Ok(false);
        }
    }

    Ok(true)
}

pub(crate) fn issue_map(issues: &[Issue]) -> HashMap<Uuid, &Issue> {
    issues.iter().map(|issue| (issue.id, issue)).collect()
}
