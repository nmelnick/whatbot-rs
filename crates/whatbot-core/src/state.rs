use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Mutex;

use crate::context::{ChannelId, Context, ServiceId};

/// Inner typed slot
type TypeMap = HashMap<TypeId, Box<dyn Any + Send>>;

/// A single per-service or per-channel slot
type SharedSlot = Arc<Mutex<TypeMap>>;

/// Dispatcher-owned per-context scratch state
#[derive(Debug, Default, Clone)]
pub struct StateMap {
    inner: Arc<DashMap<(ServiceId, ChannelId), SharedSlot>>,
}

impl StateMap {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    pub fn slot_for(&self, ctx: &Context) -> StateSlot {
        let key = ctx.state_key();
        let entry = self
            .inner
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(HashMap::new())))
            .clone();
        StateSlot { inner: entry }
    }
}

#[derive(Debug, Clone)]
pub struct StateSlot {
    inner: Arc<Mutex<TypeMap>>,
}

impl StateSlot {
    /// Read or insert-default a typed value for this slot.
    pub async fn with<T, F, R>(&self, f: F) -> R
    where
        T: Any + Send + Default + 'static,
        F: FnOnce(&mut T) -> R,
    {
        let mut guard = self.inner.lock().await;
        let entry = guard
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::<T>::default());
        let value = entry
            .downcast_mut::<T>()
            .expect("type-keyed slot must downcast");
        f(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ChannelId, Context, ServiceId, Visibility};
    use crate::identity::Account;

    #[derive(Default, Debug)]
    struct Counter(i32);

    #[derive(Default, Debug)]
    struct Flag(bool);

    fn ctx(service: &str, channel: &str) -> Context {
        let svc = ServiceId::new(service);
        Context {
            service: svc.clone(),
            channel: ChannelId::new(channel),
            visibility: Visibility::Public,
            author: Account::synthetic(svc.clone(), "nichelle"),
            bot: Account::synthetic(svc, "whatbot"),
            addressed_to_bot: false,
            mention_renderer: crate::mentions::default_mention_renderer(),
        }
    }

    #[tokio::test]
    async fn slot_persists_across_calls_in_same_context() {
        let map = StateMap::new();
        let c = ctx("svc", "general");
        map.slot_for(&c).with::<Counter, _, _>(|n| n.0 += 1).await;
        map.slot_for(&c).with::<Counter, _, _>(|n| n.0 += 1).await;
        let final_value = map.slot_for(&c).with::<Counter, _, _>(|n| n.0).await;
        assert_eq!(final_value, 2);
    }

    #[tokio::test]
    async fn channel_isolation() {
        let map = StateMap::new();
        let a = ctx("svc", "channel-a");
        let b = ctx("svc", "channel-b");
        map.slot_for(&a).with::<Counter, _, _>(|n| n.0 = 7).await;
        let in_b = map.slot_for(&b).with::<Counter, _, _>(|n| n.0).await;
        assert_eq!(in_b, 0, "channel-b must not see channel-a state");
    }

    #[tokio::test]
    async fn service_isolation() {
        let map = StateMap::new();
        let s1 = ctx("svc-one", "general");
        let s2 = ctx("svc-two", "general");
        map.slot_for(&s1).with::<Flag, _, _>(|f| f.0 = true).await;
        let in_s2 = map.slot_for(&s2).with::<Flag, _, _>(|f| f.0).await;
        assert!(
            !in_s2,
            "same channel name on a different service must not share state"
        );
    }

    #[tokio::test]
    async fn type_isolation_within_one_slot() {
        // Different concrete types in the same (service, channel) slot
        // must not collide on each other's storage.
        let map = StateMap::new();
        let c = ctx("svc", "general");
        map.slot_for(&c).with::<Counter, _, _>(|n| n.0 = 42).await;
        map.slot_for(&c).with::<Flag, _, _>(|f| f.0 = true).await;
        let n = map.slot_for(&c).with::<Counter, _, _>(|n| n.0).await;
        let f = map.slot_for(&c).with::<Flag, _, _>(|f| f.0).await;
        assert_eq!(n, 42);
        assert!(f);
    }
}
