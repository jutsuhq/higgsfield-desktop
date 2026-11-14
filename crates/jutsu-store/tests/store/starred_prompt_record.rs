//! `StarredPromptRecordTests`.

use jutsu_core::{JobState, Mode};
use uuid::Uuid;

use crate::common::{at, make_job, memory_store};

#[tokio::test]
async fn stars_are_unique_by_text_and_newest_first() {
    let store = memory_store().await;
    store.set_prompt_starred("clay", true, at(1)).await.unwrap();
    store.set_prompt_starred("neon", true, at(2)).await.unwrap();
    store.set_prompt_starred("clay", true, at(3)).await.unwrap();
    store.set_prompt_starred("", true, at(4)).await.unwrap();

    let texts: Vec<String> = store
        .starred_prompts()
        .await
        .unwrap()
        .into_iter()
        .map(|star| star.text)
        .collect();
    assert_eq!(texts, ["neon", "clay"]);
    store.set_prompt_starred("neon", false, at(5)).await.unwrap();
    let texts: Vec<String> = store
        .starred_prompts()
        .await
        .unwrap()
        .into_iter()
        .map(|star| star.text)
        .collect();
    assert_eq!(texts, ["clay"]);
}

#[tokio::test]
async fn prompt_history_fetch_skips_empty_prompts_and_cleared_jobs() {
    let store = memory_store().await;
    let old = make_job(Mode::MotionTransfer, at(10), JobState::Preparing);
    let new = make_job(Mode::MotionTransfer, at(30), JobState::Preparing);
    let mut empty = make_job(Mode::MotionTransfer, at(40), JobState::Preparing);
    empty.spec.prompt = String::new();
    for job in [&old, &new, &empty] {
        store.save_job(job).await.unwrap();
    }

    let ids = async |after| -> Vec<Uuid> {
        store
            .prompt_history(after)
            .await
            .unwrap()
            .iter()
            .map(|prompt| prompt.job_id)
            .collect()
    };
    assert_eq!(ids(None).await, [new.id, old.id]);
    assert_eq!(ids(Some(at(20))).await, [new.id]);
}
