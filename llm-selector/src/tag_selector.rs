use configuration::Configuration;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub(crate) struct TagSelector {
    id_by_tag: HashMap<String, HashSet<String>>,
    client_priorities: HashMap<String, usize>,
}

impl TagSelector {
    pub(crate) fn new(configuration: Configuration) -> Self {
        let mut client_priorities = HashMap::new();
        let mut id_by_tag_with_priority = HashMap::new();

        for provider in configuration.providers {
            let id = provider.id.clone();
            for tag in provider.tags.clone() {
                id_by_tag_with_priority
                    .entry(tag)
                    .or_insert_with(Vec::new)
                    .push((provider.priority, id.clone()));
            }
            let priority = provider.priority;
            client_priorities.insert(id, priority);
        }

        let id_by_tag = id_by_tag_with_priority
            .into_iter()
            .map(|(k, mut v)| {
                v.sort_by_key(|(p, _)| *p);
                (k, v.into_iter().map(|(_, id)| id).collect())
            })
            .collect();

        Self {
            id_by_tag,
            client_priorities,
        }
    }

    fn get_client_ids_by_tags_impl(&self, tags: &[String]) -> Option<Vec<String>> {
        if tags.is_empty() {
            return None;
        }
        let id_sets = tags.iter().filter_map(|t| self.id_by_tag.get(t).cloned());
        let ids = id_sets.reduce(|acc, set| acc.intersection(&set).cloned().collect())?;

        let mut ids = ids
            .into_iter()
            .filter_map(|id| self.client_priorities.get(&id).map(|p| (id, *p)))
            .collect::<Vec<_>>();
        ids.sort_by_key(|(_, p)| *p);
        Some(ids.into_iter().map(|(id, _)| id).collect())
    }

    pub(crate) fn get_client_ids_by_tags(&self, tags: &[String]) -> Vec<String> {
        self.get_client_ids_by_tags_impl(tags).unwrap_or_default()
    }
}
