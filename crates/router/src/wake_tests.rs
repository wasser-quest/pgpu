//! Explicit Wake must dispatch retained contracts, not merely set desired=true.
use crate::{
    state::AppCtx,
    test_support::{MockProvider, TestApp},
};
use axum::{extract::Path, http::StatusCode};

#[tokio::test]
async fn wake_dispatches_retained_gpu_and_repeat_is_idempotent() {
    let t = TestApp::new();
    let provider = MockProvider::new().await;
    provider.attach(&t.app);
    t.insert(11, &crate::db::now_iso());
    t.app.db.set_instance_state(11, "stopped").unwrap();
    t.app
        .db
        .set_slot_desired_audited(1, false, "test sleep")
        .unwrap();
    provider.respond(200, r#"{"success":true}"#);
    let response = crate::api::slot_action(
        AppCtx(t.app.clone()),
        Path((1, "wake".into())),
        t.request("{}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(t.app.db.instance(11).unwrap().state, "start_requested");
    assert!(!t.app.db.instance(11).unwrap().healthy);
    let second = crate::api::slot_action(
        AppCtx(t.app.clone()),
        Path((1, "wake".into())),
        t.request("{}"),
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let calls = provider.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "/api/v0/instances/11/");
    assert_eq!(calls[0].2["state"], "running");
}

#[tokio::test]
async fn wake_reports_provider_failure_without_claiming_scheduling() {
    let t = TestApp::new();
    let provider = MockProvider::new().await;
    provider.attach(&t.app);
    t.insert(11, &crate::db::now_iso());
    t.app.db.set_instance_state(11, "stopped").unwrap();
    provider.respond(503, "provider outage");
    let response = crate::api::slot_action(
        AppCtx(t.app.clone()),
        Path((1, "wake".into())),
        t.request("{}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(t.app.db.instance(11).unwrap().state, "start_requested");
    assert!(t
        .app
        .db
        .events(20, None)
        .iter()
        .any(|e| e.kind == "wake_failed"));
}

#[tokio::test]
async fn force_wake_reenables_auto_rent_and_starts() {
    let t = TestApp::new();
    let provider = MockProvider::new().await;
    provider.attach(&t.app);
    t.insert(11, &crate::db::now_iso());
    t.app.db.set_instance_state(11, "stopped").unwrap();
    t.app.db.set_auto_rent(false).unwrap();
    let blocked = crate::api::slot_action(
        AppCtx(t.app.clone()),
        Path((1, "wake".into())),
        t.request("{}"),
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::CONFLICT);
    assert!(provider.calls.lock().unwrap().is_empty());
    provider.respond(200, r#"{"success":true}"#);
    let response = crate::api::slot_action(
        AppCtx(t.app.clone()),
        Path((1, "wake".into())),
        t.request(r#"{"force":true}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(t.app.db.auto_rent_enabled());
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn explicit_wake_retries_confirmed_start_failure() {
    let t = TestApp::new();
    let provider = MockProvider::new().await;
    provider.attach(&t.app);
    t.insert(11, &crate::db::now_iso());
    t.app.db.set_instance_state(11, "start_failed").unwrap();
    provider.respond(200, r#"{"success":true}"#);
    let response = crate::api::slot_action(
        AppCtx(t.app.clone()),
        Path((1, "wake".into())),
        t.request("{}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
    assert_eq!(t.app.db.instance(11).unwrap().state, "start_requested");
}

#[tokio::test]
async fn locked_slot_cannot_be_started_by_wake() {
    let t = TestApp::new();
    let provider = MockProvider::new().await;
    provider.attach(&t.app);
    t.insert(11, &crate::db::now_iso());
    t.app.db.set_instance_state(11, "stopped").unwrap();
    crate::operations::set_slot_lock(&t.app, 1, true, "test")
        .await
        .unwrap();
    let response = crate::api::slot_action(
        AppCtx(t.app.clone()),
        Path((1, "wake".into())),
        t.request("{}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(provider.calls.lock().unwrap().is_empty());
}
