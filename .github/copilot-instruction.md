# GitHub Copilot Instructions for Ayatori

## Project Overview

Ayatori is an OpenAI-compatible API gateway written in Rust that intelligently routes requests to multiple LLM providers (Anthropic, OpenAI, Azure, Ollama, VertexAI) based on tags, model selection, usage limits, and priorities.

## Workspace Structure

This is a Rust workspace with four main crates:

- `configuration/` - Provider configuration schema and credential management
- `llm-composer/` - LLM client pool management using the `genai` crate
- `llm-selector/` - Core routing logic with tag-based (include/exclude), model-based, and ID-based selection
- `server/` - Actix-web HTTP server with OpenAI-compatible API endpoints

## Code Style and Conventions

### General Rust Practices
- Use Rust 2024 edition
- Follow standard Rust naming conventions (snake_case for functions/variables, PascalCase for types)
- Prefer explicit error handling with `Result<T, E>` over panics
- Use `anyhow::Result` for application errors, custom error types for library crates
- Add documentation comments (`///`) for public APIs
- Keep functions focused and concise

### Async/Await
- Use Tokio runtime for async operations
- Prefer `async fn` over manual `Future` implementations
- Use `tokio::spawn` for concurrent tasks when needed
- Handle cancellation gracefully

### Logging
- Use `tracing` crate for all logging
- Log levels: `error!`, `warn!`, `info!`, `debug!`, `trace!`
- Include contextual information in log messages
- Use structured logging with key-value pairs: `info!(client_id = %id, "Selected client")`

### Error Handling
- Return descriptive error messages
- Use `context()` from `anyhow` to add context to errors
- Map external errors to appropriate internal error types
- Don't expose internal implementation details in user-facing errors

### Configuration
- All configuration structures should derive `Deserialize` from `serde`
- Use `#[serde(rename_all = "snake_case")]` for enum variants
- Validate configuration at load time, not at use time
- Use `PathBuf` for file paths in config structs

### Testing
- Write unit tests in the same file as the code (`#[cfg(test)] mod tests`)
- Write integration tests in the `tests/` directory
- Mock external dependencies using traits
- Test both success and error cases
- Use descriptive test names: `test_tag_selector_filters_by_tags`

## Architecture Patterns

### Request Selection Flow
1. Parse model string to determine strategy (tags, model name, or ID) via `server/src/model.rs`
2. Tag-based: `TagSelector` finds clients matching all include tags, filtered by exclude tags → `UsageSelector` filters by capacity
3. Model-based: Direct lookup in composer's model map
4. ID-based: Direct lookup in composer's client map
5. Fallback to default client if enabled and no match found

### Tag-based Selection Details
- Include tags: all must be present on a provider (AND logic)
- Exclude tags: provider is dropped if it has **any** of these tags
- Empty include list: all providers are candidates (useful with exclude-only queries)
- Parsed from model string: `tags:fast&cheap&!vision` → include `[fast, cheap]`, exclude `[vision]`

### Usage Tracking
- `UsageStore` trait defines interface for tracking client usage
- Implementations track `input_tokens` and `requests` per client
- `UsageSelector` checks capacity limits before returning clients
- Usage is updated after successful requests

### Client Management
- Each provider gets a `genai::Client` with custom `ServiceTargetResolver`
- Clients are cached and reused across requests
- Model names are mapped to client IDs for fast lookup

## Key Files and Their Purposes

- `configuration/src/credential.rs` - Provider credential types (Azure, Anthropic, etc.)
- `llm-composer/src/client.rs` - Client creation and caching logic
- `llm-selector/src/tag_selector.rs` - Tag-based filtering (include/exclude) and priority sorting
- `llm-selector/src/usage/selector.rs` - Capacity-based filtering
- `server/src/model.rs` - `RequestModel` enum and model string parsing (tag/model/id strategies)
- `server/src/endpoints/chat_completion.rs` - Main API endpoint handler
- `src/config.rs` - Application configuration loading
- `src/main.rs` - Application entry point and setup

## Common Patterns to Use

### Model Selection Syntax
When working with model strings (parsed in `server/src/model.rs`):
- `"gpt-4"` - Direct model name
- `"tags:fast&cheap"` or `"tag:fast&cheap"` - Tag-based (AND logic, all must match)
- `"tags:fast&!vision"` - Include `fast`, exclude providers tagged `vision`
- `"id:my-provider"` - Direct provider ID
- Tags prefixed with `!` are exclude tags; all other tags are include tags

### Adding New Provider Types
1. Add variant to `LlmProviderType` enum in `configuration/src/lib.rs`
2. Add credential struct in `configuration/src/credential.rs`
3. Update `ServiceTargetResolver` logic in `llm-composer/src/client.rs`
4. Map to appropriate `AdapterKind` from `genai` crate

### Adding New API Endpoints
1. Create handler function in `server/src/endpoints/`
2. Take `LlmSelector` as parameter via `web::Data<LlmSelector>`
3. Use `ChatRequestAuthentication` extractor for auth
4. Return JSON responses via OpenAI-compatible structs
5. Register route in `server/src/lib.rs`

## Dependencies

- `actix-web` - HTTP server framework
- `genai` - Unified LLM client library
- `tokio` - Async runtime
- `serde` - Serialization/deserialization
- `tracing` - Structured logging
- `anyhow` - Error handling
- `rustls` - TLS support

## Testing Tips

- Use `cargo test` to run all tests
- Use `cargo test -p <crate>` to test specific crate
- Mock `UsageStore` for testing selectors
- Test configuration parsing with sample TOML files
- Test error cases (missing credentials, capacity exceeded, etc.)

## Performance Considerations

- Clients are cached and reused, not created per request
- Tag filtering is done in-memory with simple set operations
- Usage tracking should be fast (consider Redis backend for production)
- Priority sorting happens only after tag/capacity filtering

## Security Notes

- Bearer token authentication is optional but recommended
- API keys should be loaded from files, not hardcoded
- TLS should be enabled in production
- Don't log sensitive credentials
- Validate all user inputs at API boundaries
