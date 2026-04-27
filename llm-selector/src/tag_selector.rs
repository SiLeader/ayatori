use configuration::Configuration;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub(crate) struct ModelId(String);

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub(crate) struct ModelTag(String);

impl ModelId {
    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TagSelector {
    id_by_tag: HashMap<ModelTag, HashSet<ModelId>>,
    tag_by_id: HashMap<ModelId, HashSet<ModelTag>>,
    client_priorities: HashMap<ModelId, usize>,
}

impl TagSelector {
    pub(crate) fn new(configuration: Configuration) -> Self {
        let mut client_priorities = HashMap::new();
        let mut id_by_tag = HashMap::new();
        let mut tag_by_id = HashMap::new();

        for provider in &configuration.providers {
            let id = ModelId(provider.id.clone());
            for tag in provider.tags.iter().map(|t| ModelTag(t.clone())) {
                id_by_tag
                    .entry(tag.clone())
                    .or_insert_with(HashSet::new)
                    .insert(id.clone());
                tag_by_id
                    .entry(id.clone())
                    .or_insert_with(HashSet::new)
                    .insert(tag);
            }
            let priority = provider.priority;
            client_priorities.insert(id, priority);
        }

        Self {
            id_by_tag,
            tag_by_id,
            client_priorities,
        }
    }

    pub(crate) fn get_client_ids_by_tags(
        &self,
        tags: &[ModelTag],
        exclude_tags: &[ModelTag],
    ) -> Vec<ModelId> {
        let mut candidates = self.get_candidate_ids(tags);
        if candidates.is_empty() {
            return Vec::new();
        }

        candidates.retain(|id| !self.has_any_excluded_tag(id, exclude_tags));

        let mut client_ids = candidates.into_iter().collect::<Vec<_>>();
        client_ids.sort_by(|lhs, rhs| {
            let lhs_priority = self
                .client_priorities
                .get(lhs)
                .copied()
                .unwrap_or(usize::MAX);
            let rhs_priority = self
                .client_priorities
                .get(rhs)
                .copied()
                .unwrap_or(usize::MAX);

            lhs_priority
                .cmp(&rhs_priority)
                .then_with(|| lhs.as_str().cmp(rhs.as_str()))
        });

        client_ids
    }

    fn get_candidate_ids(&self, tags: &[ModelTag]) -> HashSet<ModelId> {
        if tags.is_empty() {
            return self.client_priorities.keys().cloned().collect();
        }

        let mut tags_iter = tags.iter();
        let Some(first_tag) = tags_iter.next() else {
            return HashSet::new();
        };

        let Some(mut candidate_ids) = self.id_by_tag.get(first_tag).cloned() else {
            return HashSet::new();
        };

        for tag in tags_iter {
            let Some(ids) = self.id_by_tag.get(tag) else {
                return HashSet::new();
            };

            candidate_ids.retain(|id| ids.contains(id));
            if candidate_ids.is_empty() {
                return HashSet::new();
            }
        }

        candidate_ids
    }

    fn has_any_excluded_tag(&self, id: &ModelId, exclude_tags: &[ModelTag]) -> bool {
        if exclude_tags.is_empty() {
            return false;
        }

        self.tag_by_id
            .get(id)
            .is_some_and(|tags| exclude_tags.iter().any(|tag| tags.contains(tag)))
    }
}

impl From<String> for ModelId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<ModelId> for String {
    fn from(value: ModelId) -> Self {
        value.0
    }
}

impl From<String> for ModelTag {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ModelTag {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{ModelTag, TagSelector};
    use configuration::{CapacityLimits, Configuration, LlmProvider, LlmProviderType};

    fn build_selector() -> TagSelector {
        TagSelector::new(Configuration {
            providers: vec![
                LlmProvider {
                    id: "fast-primary".to_string(),
                    default: Some(true),
                    provider_type: LlmProviderType::OpenAI,
                    priority: 0,
                    model: "model-a".to_string(),
                    tags: vec!["fast".to_string(), "cheap".to_string()],
                    credential_file: "unused".to_string(),
                    endpoint: "http://localhost".to_string(),
                    capacity: CapacityLimits {
                        input_tokens: None,
                        requests: None,
                    },
                },
                LlmProvider {
                    id: "fast-secondary".to_string(),
                    default: Some(false),
                    provider_type: LlmProviderType::OpenAI,
                    priority: 1,
                    model: "model-b".to_string(),
                    tags: vec![
                        "fast".to_string(),
                        "cheap".to_string(),
                        "vision".to_string(),
                    ],
                    credential_file: "unused".to_string(),
                    endpoint: "http://localhost".to_string(),
                    capacity: CapacityLimits {
                        input_tokens: None,
                        requests: None,
                    },
                },
                LlmProvider {
                    id: "smart-only".to_string(),
                    default: Some(false),
                    provider_type: LlmProviderType::OpenAI,
                    priority: 2,
                    model: "model-c".to_string(),
                    tags: vec!["smart".to_string()],
                    credential_file: "unused".to_string(),
                    endpoint: "http://localhost".to_string(),
                    capacity: CapacityLimits {
                        input_tokens: None,
                        requests: None,
                    },
                },
            ],
        })
    }

    #[test]
    fn returns_clients_matching_all_tags_in_priority_order() {
        let selector = build_selector();

        let ids = selector
            .get_client_ids_by_tags(&[ModelTag::from("fast"), ModelTag::from("cheap")], &[]);
        let ids = ids.into_iter().map(String::from).collect::<Vec<_>>();

        assert_eq!(ids, vec!["fast-primary", "fast-secondary"]);
    }

    #[test]
    fn excludes_clients_with_any_excluded_tag() {
        let selector = build_selector();

        let ids =
            selector.get_client_ids_by_tags(&[ModelTag::from("fast")], &[ModelTag::from("vision")]);
        let ids = ids.into_iter().map(String::from).collect::<Vec<_>>();

        assert_eq!(ids, vec!["fast-primary"]);
    }

    #[test]
    fn returns_empty_when_no_model_matches() {
        let selector = build_selector();

        let ids = selector.get_client_ids_by_tags(&[ModelTag::from("missing")], &[]);

        assert!(ids.is_empty());
    }

    #[test]
    fn empty_required_tags_returns_all_non_excluded_clients() {
        let selector = build_selector();

        let ids = selector.get_client_ids_by_tags(&[], &[ModelTag::from("vision")]);
        let ids = ids.into_iter().map(String::from).collect::<Vec<_>>();

        assert_eq!(ids, vec!["fast-primary", "smart-only"]);
    }
}
