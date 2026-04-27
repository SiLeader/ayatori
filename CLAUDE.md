# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Ayatori is an OpenAI-compatible API gateway that intelligently routes requests to multiple LLM providers based on tags, model selection, usage limits, and priorities. It provides a unified interface to multiple LLM backends (Anthropic, OpenAI, Azure, Ollama, VertexAI) with smart client selection based on capacity constraints and tags.

## Architecture

The project is structured as a Rust workspace with four main crates:

### 1. `configuration` (configuration/)
- Defines the LLM provider configuration schema (`Configuration`, `LlmProvider`)
- Handles credential management for different provider types (Azure, Anthropic, OpenAI, Ollama, VertexAI)
- Loads credentials from TOML files specified in the provider configuration
- Each provider has: `id`, `model`, `tags`, `priority`, `capacity` limits, and `credential_file`

### 2. `llm-composer` (llm-composer/)
- Manages the pool of LLM clients using the `genai` crate
- Creates and caches `genai::Client` instances for each configured provider
- Maps models to client IDs for direct model selection
- Key file: `client.rs` - creates clients with custom `ServiceTargetResolver` for each provider type

### 3. `llm-selector` (llm-selector/)
- Core routing logic with three selection strategies:
  - **Model-based**: Direct mapping of model name to client
  - **Tag-based**: Filters clients by tag intersection and selects by priority/usage
  - **ID-based**: Direct client lookup by provider ID
- **TagSelector** (tag_selector.rs): Finds clients matching ALL required tags while excluding any client that has an excluded tag; sorts results by priority. Empty include-tags list returns all providers (minus excluded).
- **UsageSelector** (usage/selector.rs): Filters clients based on capacity limits (input_tokens, requests)
- **UsageStore** trait: Tracks usage per client (currently only `LocalUsageStore` in-memory implementation)

### 4. `server` (server/)
- Actix-web HTTP server exposing OpenAI-compatible API
- Endpoint: `POST /v1/chat/completions` (chat_completion.rs)
- Supports bearer token authentication (optional)
- Special model selection syntax (parsed in `model.rs` → `RequestModel`):
  - `"model": "gpt-4"` - selects by model name
  - `"model": "tags:fast&cheap"` or `"model": "tag:fast&cheap"` - selects by tags (AND logic)
  - `"model": "tags:fast&!vision"` - include tag `fast`, exclude providers tagged `vision`
  - `"model": "id:my-provider"` - selects by provider ID
- Tags prefixed with `!` are exclude tags; remaining tags are include tags (all must match)
- Returns custom field `ayatori_client_id` in response showing which backend was used
- Supports TLS/HTTPS via rustls
- `client_fallback_enabled`: Falls back to default client if no matching provider found

### 5. Main binary (src/)
- Loads configuration from `/etc/ayatori/config.toml` (override with `--config`)
- Initializes LlmSelector with usage store and configuration
- Starts OpenAiServer with selector, TLS config, and API key

## Common Commands

### Build
```bash
cargo build
cargo build --release
```

### Run
```bash
# Development
cargo run -- --config path/to/config.toml

# With JSON logging
cargo run -- --json-log --config path/to/config.toml

# Release build
cargo run --release -- --config path/to/config.toml
```

### Test
```bash
# Run all tests
cargo test

# Run tests for specific crate
cargo test -p llm-selector
cargo test -p configuration
cargo test -p llm-composer
cargo test -p server

# Run specific test
cargo test test_name
```

### Lint
```bash
cargo clippy
cargo clippy --all-targets --all-features
```

### Format
```bash
cargo fmt
cargo fmt --check  # Check without modifying
```

## Configuration Structure

The main config file (typically `/etc/ayatori/config.toml`) has:
- `llm_configuration`: Path to the LLM providers configuration TOML
- `server.listen`: Bind address (e.g., "0.0.0.0:8080")
- `server.tls`: Optional TLS config with `private_key_file` and `certificate_chain_file`
- `server.api_key` or `server.api_key_file`: Optional bearer token authentication
- `server.client_fallback_enabled`: Whether to use default client when no match found
- `usage_store`: Currently only `"Local"` supported

The LLM configuration file contains an array of `llm_providers`, each with:
- `id`: Unique identifier
- `default`: Boolean, exactly one provider must be default
- `type`: Provider type (Azure, Anthropic, Ollama, OpenAI, VertexAI)
- `model`: Model identifier for the provider
- `tags`: Array of tags for selection (e.g., ["fast", "cheap"])
- `priority`: Lower number = higher priority when multiple providers match
- `credential_file`: Path to TOML file containing credentials
- `endpoint`: API endpoint URL
- `capacity.input_tokens`: Optional max input tokens before provider is excluded
- `capacity.requests`: Optional max requests before provider is excluded

## Key Design Patterns

### Request Flow
1. HTTP request arrives at `server/endpoints/chat_completion.rs`
2. Bearer auth validated against configured API key
3. Model string parsed to determine selection strategy (RequestModel enum)
4. LlmSelector uses appropriate strategy:
   - Tags → TagSelector finds matching IDs → UsageSelector filters by capacity
   - Model → Direct lookup in composer's model map
   - ID → Direct lookup in composer's client map
5. If no client found and `client_fallback_enabled`, use default client
6. Execute chat request via `genai::Client`
7. Convert response to OpenAI-compatible format with `ayatori_client_id`

### Usage Tracking
- UsageStore trait allows pluggable backends (currently only in-memory)
- Usage includes `input_tokens` and `requests` counters
- UsageSelector checks if client has reached capacity limits before selection
- When all clients in priority order are at capacity, returns None

### Client Creation
- Each LlmProvider gets a `genai::Client` with custom `ServiceTargetResolver`
- Resolver returns fixed `ServiceTarget` with provider's endpoint, auth, and model
- AdapterKind mapped from LlmProviderType (e.g., Azure → OpenAI adapter)

## Development Notes

- Uses Rust 2024 edition
- Dependencies managed at workspace level in root `Cargo.toml`
- All crates are `publish = false` (private workspace)
- Uses `tracing` for logging (supports JSON output via `--json-log`)
- Server uses Actix-web with rustls for TLS support
- Async runtime: Tokio with full features
