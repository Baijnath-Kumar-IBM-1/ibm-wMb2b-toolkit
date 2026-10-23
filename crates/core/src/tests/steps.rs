use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use crate::Error;
use crate::events::{EventBus, EventPublisher};
use crate::steps::{
    CancelToken, FnStep, Step, StepContext, StepOutcome, StepRunner, StepStatus, StepStore,
};
use crate::testing::{FixedClock, MemoryDb, RecordingPublisher};

type Calls = Arc<AtomicUsize>;

fn ok_step(name: &str, calls: &Calls, output: Value) -> Box<dyn Step> {
    let calls = calls.clone();
    Box::new(FnStep::new(name, move |_ctx: StepContext| {
        calls.fetch_add(1, Ordering::SeqCst);
        let out = output.clone();
        Box::pin(async move { Ok(out) })
    }))
}

fn flaky_step(name: &str, calls: &Calls, fail_times: usize) -> Box<dyn Step> {
    let calls = calls.clone();
    Box::new(FnStep::new(name, move |_ctx: StepContext| {
        let n = calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if n < fail_times {
                Err(Error::Http("connection refused".into()))
            } else {
                Ok(json!({"id": "ok"}))
            }
        })
    }))
}

fn runner(db: &Arc<MemoryDb>) -> (StepRunner, Arc<RecordingPublisher>) {
    let events = Arc::new(RecordingPublisher::default());
    let store: Arc<dyn StepStore> = db.clone();
    (
        StepRunner::new(store, events.clone(), Arc::new(FixedClock::default())),
        events,
    )
}

#[tokio::test]
async fn runs_all_steps_in_order_and_saves_outputs() {
    let db = MemoryDb::new();
    let (r, events) = runner(&db);
    let calls: Calls = Arc::default();
    let steps = vec![
        ok_step("a", &calls, json!({"id": "1"})),
        ok_step("b", &calls, json!({"id": "2"})),
    ];
    let report = r.run("s", "t", &steps, &CancelToken::new()).await.unwrap();
    assert!(report.is_complete());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let saved = StepStore::list(&*db, "s").unwrap();
    assert_eq!(
        saved.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(
        events.kinds(),
        [
            "t.step.started",
            "t.step.succeeded",
            "t.step.started",
            "t.step.succeeded"
        ]
    );
}

#[tokio::test]
async fn failure_stops_the_flow_and_resume_skips_done_steps() {
    let db = MemoryDb::new();
    let (r, _) = runner(&db);
    let a: Calls = Arc::default();
    let b: Calls = Arc::default();
    let c: Calls = Arc::default();
    let build = || -> Vec<Box<dyn Step>> {
        vec![
            ok_step("a", &a, json!({"id": "1"})),
            flaky_step("b", &b, 1),
            ok_step("c", &c, json!({})),
        ]
    };

    let first = r
        .run("s", "t", &build(), &CancelToken::new())
        .await
        .unwrap();
    assert!(!first.is_complete());
    assert_eq!(
        first.steps.iter().map(|s| s.outcome).collect::<Vec<_>>(),
        [
            StepOutcome::Succeeded,
            StepOutcome::Failed,
            StepOutcome::NotRun
        ]
    );
    assert_eq!(
        first.steps[1].error.as_deref(),
        Some("http error: connection refused")
    );
    assert_eq!(c.load(Ordering::SeqCst), 0);

    let second = r
        .run("s", "t", &build(), &CancelToken::new())
        .await
        .unwrap();
    assert!(second.is_complete());
    assert_eq!(
        second.steps.iter().map(|s| s.outcome).collect::<Vec<_>>(),
        [
            StepOutcome::Skipped,
            StepOutcome::Succeeded,
            StepOutcome::Succeeded
        ]
    );
    assert_eq!(a.load(Ordering::SeqCst), 1, "step a ran only once");
    assert_eq!(b.load(Ordering::SeqCst), 2);
    let saved = StepStore::list(&*db, "s").unwrap();
    assert!(saved.iter().all(|s| s.status == StepStatus::Succeeded));
}

#[tokio::test]
async fn later_steps_see_earlier_outputs_even_after_resume() {
    let db = MemoryDb::new();
    let (r, _) = runner(&db);
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let build = |fail_b: bool| -> Vec<Box<dyn Step>> {
        let seen = seen.clone();
        vec![
            Box::new(FnStep::new("a", |_| {
                Box::pin(async { Ok(json!({"partner_id": "P-42"})) })
            })),
            Box::new(FnStep::new("b", move |ctx: StepContext| {
                let seen = seen.clone();
                Box::pin(async move {
                    if fail_b {
                        return Err(Error::Http("down".into()));
                    }
                    let id = ctx
                        .output_str("a", "partner_id")
                        .unwrap_or("missing")
                        .to_string();
                    seen.lock().unwrap().push(id);
                    Ok(Value::Null)
                })
            })),
        ]
    };
    r.run("s", "t", &build(true), &CancelToken::new())
        .await
        .unwrap();
    r.run("s", "t", &build(false), &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), ["P-42"]);
}

#[tokio::test]
async fn storage_and_secret_errors_are_not_leaked_into_records() {
    let db = MemoryDb::new();
    let (r, _) = runner(&db);
    let steps: Vec<Box<dyn Step>> = vec![Box::new(FnStep::new("a", |_| {
        Box::pin(async {
            Err(Error::Storage(
                "UNIQUE constraint failed: /Users/me/app.db".into(),
            ))
        })
    }))];
    let report = r.run("s", "t", &steps, &CancelToken::new()).await.unwrap();
    let msg = report.steps[0].error.clone().unwrap();
    assert!(!msg.contains("/Users/me"));
    let saved = StepStore::list(&*db, "s").unwrap();
    assert_eq!(saved[0].error.as_deref(), Some(msg.as_str()));
}

#[tokio::test]
async fn cancel_stops_before_next_step() {
    let db = MemoryDb::new();
    let (r, _) = runner(&db);
    let token = CancelToken::new();
    let t2 = token.clone();
    let calls: Calls = Arc::default();
    let steps: Vec<Box<dyn Step>> = vec![
        Box::new(FnStep::new("a", move |_| {
            t2.cancel();
            Box::pin(async { Ok(Value::Null) })
        })),
        ok_step("b", &calls, Value::Null),
    ];
    let report = r.run("s", "t", &steps, &token).await.unwrap();
    assert!(report.cancelled);
    assert!(!report.is_complete());
    assert_eq!(report.steps[1].outcome, StepOutcome::NotRun);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn scopes_are_independent() {
    let db = MemoryDb::new();
    let (r, _) = runner(&db);
    let calls: Calls = Arc::default();
    let steps = vec![ok_step("a", &calls, Value::Null)];
    r.run("one", "t", &steps, &CancelToken::new())
        .await
        .unwrap();
    r.run("two", "t", &steps, &CancelToken::new())
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    StepStore::clear(&*db, "one").unwrap();
    assert!(StepStore::list(&*db, "one").unwrap().is_empty());
    assert_eq!(StepStore::list(&*db, "two").unwrap().len(), 1);
}

#[tokio::test]
async fn event_bus_can_carry_runner_events() {
    let db = MemoryDb::new();
    let bus = EventBus::default();
    let mut rx = bus.subscribe();
    let store: Arc<dyn StepStore> = db.clone();
    let publisher: Arc<dyn EventPublisher> = Arc::new(bus);
    let r = StepRunner::new(store, publisher, Arc::new(FixedClock::default()));
    let calls: Calls = Arc::default();
    r.run(
        "s",
        "prov",
        &[ok_step("a", &calls, Value::Null)],
        &CancelToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(rx.recv().await.unwrap().kind, "prov.step.started");
    assert_eq!(rx.recv().await.unwrap().kind, "prov.step.succeeded");
}
