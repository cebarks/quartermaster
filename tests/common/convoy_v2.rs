use spt_quartermaster::convoy::publisher::{PublicationStatus, Publisher};

pub async fn wait_for(
    publisher: &Publisher,
    accept: impl Fn(&PublicationStatus) -> bool,
) -> PublicationStatus {
    let mut status = publisher.subscribe();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        status.wait_for(accept).await.unwrap().clone()
    })
    .await
    .expect("publication did not reach expected status")
}

pub async fn ready(publisher: &Publisher) -> String {
    match wait_for(publisher, |status| {
        matches!(
            status,
            PublicationStatus::Ready { .. } | PublicationStatus::Failed { .. }
        )
    })
    .await
    {
        PublicationStatus::Ready { generation, .. } => generation,
        other => panic!("expected ready, got {other:?}"),
    }
}
