# Ayatori

An intelligent OpenAI-compatible API gateway that routes requests to multiple LLM providers based on tags, usage limits,
and priorities.

## Features

- **OpenAI-like API** - Drop-in replacement for OpenAI API like endpoints
- **Responses API** - `POST /v1/responses` plus stored-response management endpoints
- **Multi-Provider Support** - Anthropic, OpenAI, Azure OpenAI, Ollama, VertexAI
- **Intelligent Routing** - Route requests based on:
    - Tags (e.g., "fast", "cheap", "summarize") with include/exclude support
    - Model names
    - Provider IDs
    - Usage capacity limits
- **Priority-Based Selection** - Automatically select providers by priority when multiple match
- **Capacity Management** - Automatically exclude providers that exceed usage limits
- **Flexible Backends** - In-memory or Redis-based usage tracking
- **TLS Support** - Built-in HTTPS with rustls
- **Bearer Authentication** - Optional API key protection
- **Fallback Support** - Configurable fallback to default provider

## Quick Start

```bash
# Build the project
cargo build --release

# Run with sample configuration
cargo run --release -- --config sample/config.toml

# Test the API
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "tags:light&summarize",
    "messages": [
      {"role": "user", "content": "Hello!"}
    ]
  }'
```

## Installation

### From Source

```bash
git clone https://github.com/SiLeader/ayatori.git
cd ayatori
cargo build --release
```

The binary will be located at `target/release/ayatori`.

## Configuration

Ayatori uses two configuration files:

1. **Main configuration** - Server and usage store settings
2. **LLM configuration** - Provider definitions and credentials

### Main Configuration

Create a configuration file (e.g., `/etc/ayatori/config.toml`):

```toml
llm_configuration = "/etc/ayatori/llm_configuration.toml"

[server]
listen = "0.0.0.0:8080"
client_fallback_enabled = true

# Optional TLS configuration
# tls = { private_key_file = "/path/to/privkey.pem", certificate_chain_file = "/path/to/cert.pem" }

# Optional API authentication
# api_key = "your-secret-key"
# api_key_file = "/path/to/apikey.txt"

[usage_store]
type = "Local"

[response_store]
type = "Local"
ttl_seconds = 86400
max_entries = 10000

# Or use Redis for distributed tracking
# [usage_store]
# type = "Redis"
# host = "localhost"
# port = 6379
# db = 0
# password_env = "REDIS_PASSWORD"

[token_measure]
type = "ByteLength"
magnification_ratio = 0.3
```

Please see [sample/config.toml](./sample/config.toml) and [docs/configuration.md](./docs/configuration.md).

### LLM Provider Configuration

Create an LLM configuration file with your providers:

```toml
[[providers]]
id = "anthropic-claude"
default = true
type = "Anthropic"
responses_native = false
priority = 1
model = "claude-3-5-sonnet-20241022"
tags = ["smart", "fast"]
credential_file = "/etc/ayatori/credentials/anthropic.toml"
endpoint = "https://api.anthropic.com"
capacity = { input_tokens = 100000, requests = 1000 }

[[providers]]
id = "openai-gpt4"
default = false
type = "OpenAI"
responses_native = true
priority = 2
model = "gpt-4"
tags = ["smart", "expensive"]
credential_file = "/etc/ayatori/credentials/openai.toml"
endpoint = "https://api.openai.com/v1"
capacity = { input_tokens = 50000, requests = 500 }

[[providers]]
id = "ollama-local"
default = false
type = "Ollama"
responses_native = false
priority = 3
model = "llama3:8b"
tags = ["fast", "cheap", "local"]
credential_file = "/etc/ayatori/credentials/ollama.toml"
endpoint = "http://localhost:11434"
capacity = { requests = 100 }
```

Please see [sample/llm_configuration.toml](./sample/llm_configuration.toml).

### Credential Files

Each provider needs a credential file. The structure varies by provider type:

**Anthropic** (`/etc/ayatori/credentials/anthropic.toml`):

```toml
type = "Anthropic"
api_key = "sk-ant-api03-..."
```

**OpenAI** (`/etc/ayatori/credentials/openai.toml`):

```toml
type = "OpenAI"
api_key = "sk-..."
```

**Azure OpenAI** (`/etc/ayatori/credentials/azure.toml`):

```toml
type = "Azure"
api_key = "your-azure-key"
deployment = "gpt-4o-mini"
api_version = "2025-04-01-preview"
```

**Ollama** (`/etc/ayatori/credentials/ollama.toml`):

```toml
type = "Ollama"
```

## Usage

### Starting the Server

```bash
# Default configuration path (/etc/ayatori/config.toml)
ayatori

# Custom configuration
ayatori --config /path/to/config.toml

# Enable JSON logging
ayatori --json-log --config /path/to/config.toml
```

### API Endpoints

#### Chat Completions

**Endpoint:** `POST /v1/chat/completions`

**Authentication:** Optional Bearer token (if configured)

```bash
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-api-key" \
  -d '{
    "model": "tags:fast&cheap",
    "messages": [
      {"role": "user", "content": "Hello!"}
    ]
  }'
```

#### Responses API

**Endpoint:** `POST /v1/responses`

Ayatori exposes an OpenAI-compatible Responses API across native and translated backends.

```bash
curl -X POST http://localhost:8080/v1/responses \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-api-key" \
  -d '{
    "model": "tags:fast",
    "input": "Hello!"
  }'
```

Supported companion endpoints:

- `GET /v1/responses/{response_id}`
- `DELETE /v1/responses/{response_id}`
- `POST /v1/responses/{response_id}/cancel`
- `GET /v1/responses/{response_id}/input_items`

Common Responses features:

| Feature | OpenAI | Azure | Anthropic | VertexAI | Ollama | Bedrock |
|---|---|---|---|---|---|---|
| Basic create | Yes | Yes | Yes | Yes | Yes | No |
| Streaming | Yes | Yes | Yes | Yes | Yes | No |
| Function calling | Yes | Yes | Yes | Yes | Yes | No |
| Built-in tools | Yes | Yes | No | No | No | No |
| Reasoning config | Yes | Yes | Yes | Yes | No | No |
| Structured output | Yes | Yes | No | Yes | Yes | No |
| Image input | Yes | Yes | Yes | Yes | Yes | No |
| Native Responses backend | Yes | Yes | No | No | No | No |

Gateway-managed features:

- `previous_response_id`
- `store: true | false`
- `background: true`
- stored response management endpoints

Example: function calling

```bash
curl -X POST http://localhost:8080/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "id:openai-gpt-4o-mini",
    "input": "What is the weather in Tokyo?",
    "tools": [{
      "type": "function",
      "name": "lookup_weather",
      "description": "Lookup the weather",
      "parameters": {
        "type": "object",
        "properties": {
          "city": { "type": "string" }
        },
        "required": ["city"]
      }
    }],
    "tool_choice": "required"
  }'
```

Example: structured output

```bash
curl -X POST http://localhost:8080/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "id:openai-gpt-4o-mini",
    "input": "Return Tokyo as JSON",
    "text": {
      "format": {
        "type": "json_schema",
        "name": "city_response",
        "schema": {
          "type": "object",
          "properties": {
            "city": { "type": "string" }
          },
          "required": ["city"]
        },
        "strict": true
      }
    }
  }'
```

Example: streaming

```bash
curl -N -X POST http://localhost:8080/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "id:anthropic-claude",
    "input": "Count to five",
    "stream": true
  }'
```

Example: previous response chaining

```bash
curl -X POST http://localhost:8080/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "id:openai-gpt-4o-mini",
    "input": "Remember that my city is Tokyo"
  }'
```

Then use the returned response ID:

```bash
curl -X POST http://localhost:8080/v1/responses \
  -H "Content-Type: application/json" \
  -d '{
    "model": "id:openai-gpt-4o-mini",
    "input": "What city did I mention?",
    "previous_response_id": "resp_..."
  }'
```

Migration notes from Chat Completions:

- `messages` becomes `input`
- top-level system prompt becomes `instructions`
- `choices[].message.content` becomes `output[]` and `output_text`

### Model Selection Strategies

Ayatori supports three ways to select providers:

#### 1. Tag-Based Selection

Select providers matching ALL specified tags (AND logic):

```json
{
  "model": "tags:fast&cheap",
  "messages": [...]
}
```

Or use the singular alias `tag:`:

```json
{
  "model": "tag:smart&local",
  "messages": [...]
}
```

**Exclude tags** — prefix a tag with `!` to skip providers that carry it:

```json
{
  "model": "tags:fast&!vision",
  "messages": [...]
}
```

This selects providers tagged `fast` while excluding any provider tagged `vision`.

You can combine multiple include and exclude tags:

```json
{
  "model": "tags:fast&cheap&!vision&!slow",
  "messages": [...]
}
```

**Exclude-only** — omit include tags to match all providers, then filter by exclude tags:

```json
{
  "model": "tags:!expensive",
  "messages": [...]
}
```

Ayatori will:

1. Find all providers matching the include tags (or all providers if none specified)
2. Remove providers that have any of the exclude tags
3. Filter out providers exceeding capacity limits
4. Select the provider with the highest priority (lowest priority number)

#### 2. Model-Based Selection

Select provider by model name:

```json
{
  "model": "gpt-4",
  "messages": [
    ...
  ]
}
```

#### 3. ID-Based Selection

Select provider by exact ID:

```json
{
  "model": "id:anthropic-claude",
  "messages": [
    ...
  ]
}
```

### Response Format

Responses follow the OpenAI API format with an additional field:

```json
{
  "id": "chatcmpl-123",
  "object": "chat.completion",
  "created": 1677652288,
  "model": "gpt-4",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Hello! How can I help you?"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 10,
    "completion_tokens": 8,
    "total_tokens": 18
  },
  "ayatori_client_id": "anthropic-claude"
}
```

The `ayatori_client_id` field shows which provider handled the request.

## Architecture

### Workspace Structure

```
ayatori/
├── configuration/      # Provider configuration schema
├── llm-composer/       # Client pool management
├── llm-selector/       # Routing logic
├── server/             # HTTP API server
├── token-measure/      # Token estimation
└── src/                # Main binary
```

### Request Flow

1. **HTTP Request** arrives at `/v1/chat/completions`
2. **Authentication** validates bearer token (if configured)
3. **Model Parsing** determines selection strategy (tags/model/id)
4. **Provider Selection**:
    - Tag-based: Match include tags → remove exclude-tagged providers → filter by capacity → sort by priority
    - Model-based: Direct model-to-provider lookup
    - ID-based: Direct provider lookup
5. **Fallback** to default provider if enabled and no match found
6. **Request Execution** via selected provider's client
7. **Response** converted to OpenAI format with `ayatori_client_id`

### Usage Tracking

- **LocalUsageStore**: In-memory tracking (single instance)
- **RedisUsageStore**: Distributed tracking (multiple instances)

Usage is tracked per provider:

- `input_tokens`: Total input tokens processed
- `requests`: Total number of requests

Providers are automatically excluded when they reach capacity limits.

## Development

### Building

```bash
cargo build
```

### Running Tests

```bash
# All tests
cargo test

# Specific crate
cargo test -p llm-selector

# Specific test
cargo test test_tag_selector
```

### Linting

```bash
cargo clippy --all-targets --all-features
```

### Formatting

```bash
cargo fmt
```

### Project Structure

- `configuration/src/lib.rs` - Provider and credential types
- `llm-composer/src/client.rs` - Client creation and caching
- `llm-selector/src/tag_selector.rs` - Tag-based filtering
- `llm-selector/src/usage/selector.rs` - Capacity filtering
- `server/src/endpoints/chat_completion.rs` - API endpoint
- `src/main.rs` - Application entry point

## License

Apache-2.0

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Support

For issues and questions, please open an issue on GitHub.
