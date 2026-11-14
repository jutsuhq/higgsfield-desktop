//! The change broadcast that stands in for `@Query` refreshing: one message per committed write, none for a write
//! that fails.

use jutsu_core::{DeliverySet, JobState};
use jutsu_store::{StoreChange, StoreError};
use time::OffsetDateTime;
use tokio::sync::broadcast::error::TryRecvError;
use uuid::Uuid;

use crate::common::{file_url, job, memory_store};

#[tokio::test]
async fn committed_writes_say_what_changed() {
    let store = memory_store().await;
    let mut changes = store.subscribe();

    let mut done = job(JobState::Done);
    done.output_file = Some(file_url("/tmp/result.mp4"));
    store.save_job(&done).await.unwrap();
    assert_eq!(changes.try_recv().unwrap(), StoreChange::Jobs);
    assert_eq!(changes.try_recv().unwrap(), StoreChange::Materials);

    let set = DeliverySet::new(
        Uuid::new_v4(),
        "Client",
        OffsetDateTime::now_utc(),
        vec![],
        vec![],
    );
    store.save_delivery_set(&set).await.unwrap();
    assert_eq!(changes.try_recv().unwrap(), StoreChange::DeliverySets);

    store
        .set_prompt_starred("neon", true, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(changes.try_recv().unwrap(), StoreChange::StarredPrompts);
    assert!(matches!(changes.try_recv(), Err(TryRecvError::Empty)));
}

#[tokio::test]
async fn a_failed_write_says_nothing_and_changes_nothing() {
    let store = memory_store().await;
    let mut changes = store.subscribe();

    let result = store.make_variant_primary(Uuid::new_v4()).await;

    assert!(matches!(result, Err(StoreError::Material(_))));
    assert!(matches!(changes.try_recv(), Err(TryRecvError::Empty)));
}
