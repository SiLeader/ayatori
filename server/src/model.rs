use llm_responses::ResponsesProvider;
use llm_selector::LlmSelector;
use llm_selector::genai::Client;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RequestModel {
    Model(String),
    Tags {
        include: Vec<String>,
        exclude: Vec<String>,
    },
    Id(String),
}

impl RequestModel {
    pub(crate) async fn select_model(self, llm_selector: &LlmSelector) -> Option<(String, Client)> {
        match self {
            RequestModel::Model(model) => llm_selector.select_client_by_model(&model).await,
            RequestModel::Tags { include, exclude } => {
                llm_selector.select_client_by_tags(include, exclude).await
            }
            RequestModel::Id(id) => llm_selector.select_client_by_id(&id).await,
        }
    }

    pub(crate) async fn select_responses_provider(
        self,
        llm_selector: &LlmSelector,
    ) -> Option<(String, Arc<dyn ResponsesProvider>)> {
        match self {
            RequestModel::Model(model) => {
                llm_selector
                    .select_responses_provider_by_model(&model)
                    .await
            }
            RequestModel::Tags { include, exclude } => {
                llm_selector
                    .select_responses_provider_by_tags(include, exclude)
                    .await
            }
            RequestModel::Id(id) => llm_selector.select_responses_provider_by_id(&id).await,
        }
    }
}

impl From<String> for RequestModel {
    fn from(value: String) -> Self {
        let Some((scheme, content)) = value.split_once(':') else {
            return Self::Model(value);
        };

        match scheme {
            "tags" | "tag" => {
                let (exclude, include): (Vec<_>, Vec<_>) = content
                    .split('&')
                    .map(|s| s.trim().to_string())
                    .partition(|s| s.starts_with('!'));
                let exclude = exclude
                    .into_iter()
                    .map(|s| s.trim_start_matches('!').to_string())
                    .collect::<Vec<_>>();
                Self::Tags { include, exclude }
            }
            "id" => Self::Id(content.to_string()),
            _ => Self::Model(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RequestModel;

    #[test]
    fn parses_plain_model_name() {
        assert_eq!(
            RequestModel::from("gpt-4.1".to_string()),
            RequestModel::Model("gpt-4.1".to_string())
        );
    }

    #[test]
    fn leaves_unknown_scheme_as_model() {
        assert_eq!(
            RequestModel::from("custom:model".to_string()),
            RequestModel::Model("custom:model".to_string())
        );
    }

    #[test]
    fn parses_id_scheme() {
        assert_eq!(
            RequestModel::from("id:primary".to_string()),
            RequestModel::Id("primary".to_string())
        );
    }

    #[test]
    fn parses_tags_with_include_and_exclude() {
        assert_eq!(
            RequestModel::from("tags:fast & cheap & !vision & !slow".to_string()),
            RequestModel::Tags {
                include: vec!["fast".to_string(), "cheap".to_string()],
                exclude: vec!["vision".to_string(), "slow".to_string()],
            }
        );
    }

    #[test]
    fn parses_tag_alias() {
        assert_eq!(
            RequestModel::from("tag:fast & !vision".to_string()),
            RequestModel::Tags {
                include: vec!["fast".to_string()],
                exclude: vec!["vision".to_string()],
            }
        );
    }
}
