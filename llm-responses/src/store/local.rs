use super::{ResponseStore, StoreError, request_input_items};
use crate::types::{CreateResponseRequest, InputItem, ResponseObject};
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub struct LocalResponseStore {
    inner: RwLock<LocalStoreState>,
    max_entries: usize,
}

struct LocalStoreState {
    entries: HashMap<String, StoredEntry>,
    next_touch: u64,
}

struct StoredEntry {
    response: ResponseObject,
    request_input: Vec<InputItem>,
    expires_at: Option<Instant>,
    touch_order: u64,
}

impl Default for LocalStoreState {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            next_touch: 1,
        }
    }
}

impl LocalResponseStore {
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: RwLock::new(LocalStoreState::default()),
            max_entries: max_entries.max(1),
        }
    }
}

#[async_trait]
impl ResponseStore for LocalResponseStore {
    async fn put(&self, response: &ResponseObject, ttl: Option<Duration>) -> Result<(), StoreError> {
        let mut state = self.inner.write().await;
        purge_expired(&mut state);
        let expires_at = ttl.map(|ttl| Instant::now() + ttl);
        let touch_order = next_touch(&mut state);

        if let Some(entry) = state.entries.get_mut(&response.id) {
            entry.response = response.clone();
            entry.expires_at = expires_at;
            entry.touch_order = touch_order;
        } else {
            state.entries.insert(
                response.id.clone(),
                StoredEntry {
                    response: response.clone(),
                    request_input: Vec::new(),
                    expires_at,
                    touch_order,
                },
            );
        }

        enforce_capacity(&mut state, self.max_entries);
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<ResponseObject>, StoreError> {
        let mut state = self.inner.write().await;
        if remove_if_expired(&mut state, id) {
            return Ok(None);
        }

        let touch_order = next_touch(&mut state);
        Ok(state.entries.get_mut(id).map(|entry| {
            entry.touch_order = touch_order;
            entry.response.clone()
        }))
    }

    async fn delete(&self, id: &str) -> Result<bool, StoreError> {
        let mut state = self.inner.write().await;
        Ok(state.entries.remove(id).is_some())
    }

    async fn append_input_chain(
        &self,
        id: &str,
        request: &CreateResponseRequest,
    ) -> Result<(), StoreError> {
        let mut state = self.inner.write().await;
        if remove_if_expired(&mut state, id) {
            return Err(StoreError::NotFound);
        }

        let touch_order = next_touch(&mut state);
        let entry = state.entries.get_mut(id).ok_or(StoreError::NotFound)?;
        entry.request_input = request_input_items(request);
        entry.touch_order = touch_order;
        Ok(())
    }

    async fn rebuild_input(&self, id: &str) -> Result<Vec<InputItem>, StoreError> {
        let mut state = self.inner.write().await;
        if remove_if_expired(&mut state, id) {
            return Err(StoreError::NotFound);
        }

        let touch_order = next_touch(&mut state);
        let entry = state.entries.get_mut(id).ok_or(StoreError::NotFound)?;
        entry.touch_order = touch_order;
        Ok(entry.request_input.clone())
    }
}

fn next_touch(state: &mut LocalStoreState) -> u64 {
    let touch = state.next_touch;
    state.next_touch = state.next_touch.saturating_add(1);
    touch
}

fn purge_expired(state: &mut LocalStoreState) {
    let now = Instant::now();
    state
        .entries
        .retain(|_, entry| entry.expires_at.is_none_or(|expires_at| expires_at > now));
}

fn remove_if_expired(state: &mut LocalStoreState, id: &str) -> bool {
    let expired = state
        .entries
        .get(id)
        .is_some_and(|entry| entry.expires_at.is_some_and(|expires_at| expires_at <= Instant::now()));
    if expired {
        state.entries.remove(id);
    }
    expired
}

fn enforce_capacity(state: &mut LocalStoreState, max_entries: usize) {
    while state.entries.len() > max_entries {
        let Some(id) = state
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.touch_order)
            .map(|(id, _)| id.clone())
        else {
            break;
        };
        state.entries.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::LocalResponseStore;
    use crate::store::ResponseStore;
    use crate::types::{CreateResponseRequest, ResponseInput, ResponseObject, ResponseStatus};
    use std::time::Duration;

    fn request(text: &str) -> CreateResponseRequest {
        CreateResponseRequest {
            model: "gpt-test".to_string(),
            input: ResponseInput::Text(text.to_string()),
            instructions: None,
            previous_response_id: None,
            store: None,
            background: None,
            stream: None,
            tools: None,
            tool_choice: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            reasoning: None,
            text: None,
            metadata: None,
            user: None,
            parallel_tool_calls: None,
            truncation: None,
        }
    }

    fn response(id: &str) -> ResponseObject {
        ResponseObject {
            id: id.to_string(),
            object: "response".to_string(),
            created_at: 1,
            status: ResponseStatus::Completed,
            model: "gpt-test".to_string(),
            output: vec![],
            output_text: None,
            usage: None,
            error: None,
            incomplete_details: None,
            previous_response_id: None,
            metadata: None,
            parallel_tool_calls: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            reasoning: None,
            text: None,
            tool_choice: None,
            tools: None,
            truncation: None,
            user: None,
            ayatori_client_id: String::new(),
        }
    }

    #[tokio::test]
    async fn ttl_expiration_returns_none() {
        let store = LocalResponseStore::new(4);
        store
            .put(&response("resp_1"), Some(Duration::from_millis(5)))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(store.get("resp_1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rebuild_input_returns_stored_chain() {
        let store = LocalResponseStore::new(4);
        store.put(&response("resp_1"), None).await.unwrap();
        store
            .append_input_chain("resp_1", &request("hello"))
            .await
            .unwrap();

        let items = store.rebuild_input("resp_1").await.unwrap();
        assert_eq!(items.len(), 1);
    }

    #[tokio::test]
    async fn lru_eviction_removes_oldest_entry() {
        let store = LocalResponseStore::new(1);
        store.put(&response("resp_1"), None).await.unwrap();
        store.put(&response("resp_2"), None).await.unwrap();

        assert!(store.get("resp_1").await.unwrap().is_none());
        assert!(store.get("resp_2").await.unwrap().is_some());
    }
}
