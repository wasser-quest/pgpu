use crate::test_support::{MockProvider, TestApp};
use std::sync::Arc;

fn setup(action: &str, age: i64) -> TestApp {
    let t = TestApp::new();
    let mut cfg = (*t.app.cfg()).clone();
    cfg.slots[0].scheduling.after_s = 60;
    cfg.slots[0].scheduling.action = action.into();
    *t.app.cfg.write().unwrap() = Arc::new(cfg);
    t.insert(11, &crate::db::now_iso());
    t.app.db.set_instance_state(11, "scheduling").unwrap();
    let conn = rusqlite::Connection::open(t.dir.join("test.sqlite")).unwrap();
    let since = (chrono::Utc::now() - chrono::Duration::seconds(age)).to_rfc3339();
    conn.execute("UPDATE instances SET boot_started_at=?1,actual_status='stopped',intended_status='running' WHERE vast_id=11",[since]).unwrap();
    t
}

#[tokio::test]
async fn none_and_before_deadline_do_nothing() {
    for (action, age) in [("none", 120), ("destroy", 30)] {
        let t = setup(action, age);
        let p = MockProvider::new().await;
        p.attach(&t.app);
        crate::scheduling::enforce(&t.app).await.unwrap();
        assert!(p.calls.lock().unwrap().is_empty());
        assert!(!crate::scheduling::held(&t.app, 1).unwrap());
    }
}
#[tokio::test]
async fn stop_retains_disk_and_holds_slot_until_wake() {
    let t = setup("stop", 120);
    let p = MockProvider::new().await;
    p.attach(&t.app);
    p.respond(200, r#"{"success":true}"#);
    crate::scheduling::enforce(&t.app).await.unwrap();
    assert_eq!(t.app.db.instance(11).unwrap().state, "stopped");
    assert!(crate::scheduling::held(&t.app, 1).unwrap());
    assert!(!t.app.db.slot_desired(1));
    assert!(crate::operations::check_admission(&t.app, 1, 1.0, 0.1, Some(11)).is_err());
    assert_eq!(p.calls.lock().unwrap()[0].2["state"], "stopped");
    let response = crate::api::slot_action(
        crate::state::AppCtx(t.app.clone()),
        axum::extract::Path((1, "wake".into())),
        t.request("{}"),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(!crate::scheduling::held(&t.app, 1).unwrap());
    assert_eq!(t.app.db.instance(11).unwrap().state, "start_requested");
}
#[tokio::test]
async fn destroy_is_opt_in_and_deletes_expired_wait() {
    let t = setup("destroy", 120);
    let p = MockProvider::new().await;
    p.attach(&t.app);
    p.respond(200, r#"{"success":true}"#);
    crate::scheduling::enforce(&t.app).await.unwrap();
    assert_eq!(t.app.db.instance(11).unwrap().state, "destroyed");
    assert!(crate::scheduling::held(&t.app, 1).unwrap());
    assert_eq!(p.calls.lock().unwrap()[0].0, "DELETE");
}
#[tokio::test]
async fn timeout_does_not_apply_to_booting_or_healthy() {
    for state in ["booting", "healthy", "provisioning", "start_failed"] {
        let t = setup("destroy", 120);
        let p = MockProvider::new().await;
        p.attach(&t.app);
        t.app.db.set_instance_state(11, state).unwrap();
        crate::scheduling::enforce(&t.app).await.unwrap();
        assert!(p.calls.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn locks_pins_and_manual_instances_are_protected() {
    for kind in ["lock", "pin", "manual"] {
        let t = setup("destroy", 120);
        let p = MockProvider::new().await;
        p.attach(&t.app);
        let conn = rusqlite::Connection::open(t.dir.join("test.sqlite")).unwrap();
        match kind {
            "lock" => crate::operations::set_slot_lock(&t.app, 1, true, "test")
                .await
                .unwrap(),
            "pin" => {
                conn.execute("UPDATE instances SET pinned=1 WHERE vast_id=11", [])
                    .unwrap();
            }
            _ => {
                conn.execute("UPDATE instances SET mode='manual' WHERE vast_id=11", [])
                    .unwrap();
            }
        }
        crate::scheduling::enforce(&t.app).await.unwrap();
        assert!(p.calls.lock().unwrap().is_empty());
        assert!(!crate::scheduling::held(&t.app, 1).unwrap());
    }
}
#[tokio::test]
async fn failed_destroy_is_pending_and_cancelled_if_allocation_starts() {
    let t = setup("destroy", 120);
    let p = MockProvider::new().await;
    p.attach(&t.app);
    p.respond(503, "outage");
    crate::scheduling::enforce(&t.app).await.unwrap();
    assert_eq!(t.app.db.instance(11).unwrap().state, "scheduling");
    assert_eq!(t.app.db.pending_operations().unwrap().len(), 1);
    assert!(crate::scheduling::held(&t.app, 1).unwrap());
    // Early retries must wait for a fresh inventory before timeout deletion.
    crate::operations::retry_non_scheduling_pending(&t.app)
        .await
        .unwrap();
    assert_eq!(p.calls.lock().unwrap().len(), 1);
    t.app.db.set_instance_state(11, "booting").unwrap();
    p.respond(200, r#"{"success":true}"#);
    crate::operations::retry_scheduling_pending(&t.app)
        .await
        .unwrap();
    assert!(t.app.db.pending_operations().unwrap().is_empty());
    assert_eq!(p.calls.lock().unwrap().len(), 1);
    assert_eq!(t.app.db.instance(11).unwrap().state, "booting");
}
#[tokio::test]
async fn disabling_rule_cancels_pending_cleanup() {
    let t = setup("destroy", 120);
    let p = MockProvider::new().await;
    p.attach(&t.app);
    p.respond(503, "outage");
    crate::scheduling::enforce(&t.app).await.unwrap();
    let mut cfg = (*t.app.cfg()).clone();
    cfg.slots[0].scheduling.action = "none".into();
    *t.app.cfg.write().unwrap() = Arc::new(cfg);
    crate::operations::retry_scheduling_pending(&t.app)
        .await
        .unwrap();
    assert!(t.app.db.pending_operations().unwrap().is_empty());
    assert_eq!(p.calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn hold_survives_restart_and_overrides_scheduled_desire() {
    let t = setup("stop", 120);
    let p = MockProvider::new().await;
    p.attach(&t.app);
    p.respond(200, r#"{"success":true}"#);
    crate::scheduling::enforce(&t.app).await.unwrap();
    let reopened = crate::db::Db::open(&t.dir.join("test.sqlite")).unwrap();
    assert_eq!(
        reopened
            .try_setting("scheduling_hold:1")
            .unwrap()
            .as_deref(),
        Some("1")
    );
    t.app
        .db
        .set_slot_desired_audited(1, true, "scheduled wake")
        .unwrap();
    let snap = crate::reconciler::build_snapshot(&t.app).await.unwrap();
    assert!(!snap.slots[0].desired_running);
}
#[tokio::test]
async fn zero_timeout_and_current_provider_progress_are_protected() {
    for actual in ["running", "loading", "created", "provisioning"] {
        let t = setup("destroy", 120);
        let p = MockProvider::new().await;
        p.attach(&t.app);
        let conn = rusqlite::Connection::open(t.dir.join("test.sqlite")).unwrap();
        conn.execute(
            "UPDATE instances SET actual_status=?1 WHERE vast_id=11",
            [actual],
        )
        .unwrap();
        crate::scheduling::enforce(&t.app).await.unwrap();
        assert!(p.calls.lock().unwrap().is_empty());
        assert!(!crate::scheduling::held(&t.app, 1).unwrap());
    }
    let t = setup("destroy", 120);
    let p = MockProvider::new().await;
    p.attach(&t.app);
    let mut cfg = (*t.app.cfg()).clone();
    cfg.slots[0].scheduling.after_s = 0;
    *t.app.cfg.write().unwrap() = Arc::new(cfg);
    crate::scheduling::enforce(&t.app).await.unwrap();
    assert!(p.calls.lock().unwrap().is_empty());
}
#[test]
fn scheduling_config_validates_and_defaults_to_disabled() {
    let base = "[[slots]]\nid=1\nname='test'\nrole='llm'\n";
    let cfg = crate::config::Config::load_str(base).unwrap();
    assert_eq!(cfg.slots[0].scheduling.action, "none");
    assert_eq!(cfg.slots[0].scheduling.after_s, 0);
    for rule in [
        "after_s=-1\naction='stop'",
        "after_s=60\naction='delete'",
        "after_s=60\naction='stop'\nafter=10",
    ] {
        assert!(
            crate::config::Config::load_str(&format!("{base}[slots.scheduling]\n{rule}")).is_err()
        );
    }
}
