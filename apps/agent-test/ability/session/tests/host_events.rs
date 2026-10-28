//! 会话能力事件走 Host；用户事件走 Host 用户通道。

use std::sync::Arc;

use ability_core::AbilityHost;
use ability_support::{
    BusEvent, EventClass, EventFilter, EventListener, InstanceId, LoadSpec, MethodId, SourceId,
    UserEvent, UserEventSink,
};
use frame_core::InvokeRequest;
use serde_json::json;
use tokio::sync::Mutex;

#[tokio::test]
async fn ability_events_and_host_user_channel_are_separate() {
    let host = AbilityHost::new();
    host.install_str("session", ai_session::provider())
        .await
        .unwrap();
    host.load(LoadSpec::new(
        SourceId::new("session").unwrap(),
        InstanceId::new("session").unwrap(),
    ))
    .await
    .unwrap();

    let mut ability_ev = host
        .events()
        .listen(EventFilter {
            class: Some(EventClass::Ability),
            instance: Some(InstanceId::new("session").unwrap()),
            name: None,
        })
        .await;

    let got_user = Arc::new(Mutex::new(None::<String>));
    struct Cap(Arc<Mutex<Option<String>>>);
    #[async_trait::async_trait]
    impl UserEventSink for Cap {
        async fn on_user_event(&self, event: &UserEvent) {
            *self.0.lock().await = Some(event.name.clone());
        }
    }
    host.events()
        .subscribe_user(Arc::new(Cap(Arc::clone(&got_user))))
        .await;

    let gw = host.clone().into_gateway();
    let created = gw
        .invoke(InvokeRequest::new(
            InstanceId::new("session").unwrap(),
            MethodId::new("create").unwrap(),
            json!({ "title": "e" }),
        ))
        .await
        .unwrap();
    let id = created.data["id"].as_str().unwrap();

    let ev = ability_ev.next().await.unwrap();
    match ev {
        BusEvent::Ability(a) => {
            assert_eq!(a.name, "created");
            assert_eq!(a.payload["id"], id);
        }
        other => panic!("{other:?}"),
    }

    host.emit_user(UserEvent::new("bookmark", json!({ "n": 1 })))
        .await;
    assert_eq!(got_user.lock().await.as_deref(), Some("bookmark"));

    gw.invoke(InvokeRequest::new(
        InstanceId::new("session").unwrap(),
        MethodId::new("append_user").unwrap(),
        json!({ "id": id, "text": "z" }),
    ))
    .await
    .unwrap();

    let ev = ability_ev.next().await.unwrap();
    match ev {
        BusEvent::Ability(a) => {
            assert_eq!(a.name, "items_appended");
            assert_eq!(a.payload["role"], "user");
        }
        other => panic!("{other:?}"),
    }
}
