//! Opt-in allocation-wait cleanup. Never applies to model download/boot phases.
use crate::{db::InstanceRow, state::SharedApp};

pub fn held(app: &SharedApp, slot: i64) -> anyhow::Result<bool> {
    Ok(app
        .db
        .try_setting(&format!("scheduling_hold:{slot}"))?
        .as_deref()
        == Some("1"))
}
pub fn set_hold(app: &SharedApp, slot: i64, held: bool) -> anyhow::Result<()> {
    app.db.set_setting(
        &format!("scheduling_hold:{slot}"),
        if held { "1" } else { "0" },
    )
}

pub fn waiting(inst: &InstanceRow) -> bool {
    matches!(inst.state.as_str(), "start_requested" | "scheduling")
        && inst.intended_status == "running"
        && inst.destroyed_at.is_none()
}

/// Caller holds the management lock. Revalidate config, state and allocation age.
pub fn check(
    app: &SharedApp,
    inst: &InstanceRow,
    operation: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    let cfg = app.cfg();
    let slot = cfg
        .slot(inst.slot_id)
        .ok_or_else(|| anyhow::anyhow!("unknown slot"))?;
    let rule = &slot.scheduling;
    let pending_timeout =
        app.db.pending_operations()?.iter().any(|p| {
            p.0 == inst.vast_id && p.1 == operation && p.2.starts_with("scheduling timeout:")
        });
    // Persisting stop/delete changes intended_status before the provider call.
    let still_waiting = matches!(inst.state.as_str(), "start_requested" | "scheduling")
        && inst.destroyed_at.is_none();
    anyhow::ensure!(
        (waiting(inst) || (pending_timeout && still_waiting))
            && !matches!(
                inst.actual_status.as_str(),
                "running" | "loading" | "created" | "provisioning"
            )
            && inst.mode != praxis_common::Mode::Manual,
        "stale/protected scheduling timeout"
    );
    anyhow::ensure!(
        rule.after_s > 0 && rule.action == operation && matches!(operation, "stop" | "destroy"),
        "scheduling timeout disabled/changed"
    );
    let since = app
        .db
        .boot_started_at(inst.vast_id)?
        .ok_or_else(|| anyhow::anyhow!("allocation start time unknown"))?;
    anyhow::ensure!(
        (now - since).num_seconds() >= rule.after_s,
        "scheduling deadline not reached"
    );
    Ok(())
}

pub async fn enforce(app: &SharedApp) -> anyhow::Result<()> {
    let now = chrono::Utc::now();
    for row in app.db.try_instances(false)? {
        if !waiting(&row) || row.pinned || row.mode == praxis_common::Mode::Manual {
            continue;
        }
        let cfg = app.cfg();
        let Some(slot) = cfg.slot(row.slot_id) else {
            continue;
        };
        let rule = slot.scheduling.clone();
        if rule.after_s <= 0 || rule.action == "none" {
            continue;
        }
        let Some(since) = app.db.boot_started_at(row.vast_id)? else {
            continue;
        };
        if (now - since).num_seconds() < rule.after_s {
            continue;
        }
        // Persisted intent/retry owns any already requested cleanup.
        if app
            .db
            .pending_operations()?
            .iter()
            .any(|p| p.0 == row.vast_id)
        {
            continue;
        }
        let reason = format!(
            "scheduling timeout: {}s since start request; {}; slot paused until explicit wake",
            rule.after_s, rule.action
        );
        if let Err(error) =
            crate::operations::automatic_request(app, row.vast_id, &rule.action, &reason).await
        {
            tracing::warn!(instance_id=row.vast_id,%error,"scheduling timeout cleanup blocked or pending");
        }
    }
    Ok(())
}
