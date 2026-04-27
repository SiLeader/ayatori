# Changelog

## Unreleased (target: v0.3.0)

### Added

- OpenAI-compatible `POST /v1/responses` endpoint with streaming support
- Responses state management endpoints:
    - `GET /v1/responses/{id}`
    - `DELETE /v1/responses/{id}`
    - `POST /v1/responses/{id}/cancel`
    - `GET /v1/responses/{id}/input_items`
- Provider adapters for OpenAI, Azure OpenAI, Anthropic, Vertex AI, and Ollama
- Built-in response storage for `previous_response_id`, `store`, and `background`
- Integration coverage for Responses API create, streaming, state, built-in tools, and feature gating

### Changed

- CI now validates workspace tests with all targets and stricter clippy settings
- Sample configuration now includes `response_store`
- Add `!` (NOT) operator to the `tags` query
