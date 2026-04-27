# Configuration Reference

Ayatori uses two config files:

- Main config: server settings, stores, token measurement
- LLM config: provider routing definitions

## Main Config

```toml
llm_configuration = "./sample/llm_configuration.toml"

[server]
listen = "0.0.0.0:8080"
client_fallback_enabled = true

[usage_store]
type = "Local"

[response_store]
type = "Local"
ttl_seconds = 86400
max_entries = 10000

[token_measure]
type = "ByteLength"
magnification_ratio = 0.3
```

### `[server]`

- `listen`: bind address
- `tls`: optional rustls certificate/key pair
- `api_key`: static bearer token
- `api_key_file`: bearer token loaded from file
- `client_fallback_enabled`: when `true`, routes unmatched requests to the default provider

### `[usage_store]`

Tracks routing capacity usage for tag/model selection.

- `type = "Local"`: in-memory counters
- `type = "Redis"`: shared counters across instances

### `[response_store]`

Stores Responses API state used by:

- `previous_response_id`
- `store: true`
- `background: true`
- `GET /v1/responses/{id}`
- `DELETE /v1/responses/{id}`
- `POST /v1/responses/{id}/cancel`
- `GET /v1/responses/{id}/input_items`

Current backends:

- `type = "Local"`: in-memory response objects and input chains

Options:

- `ttl_seconds`: expiration for stored responses. `None` disables TTL.
- `max_entries`: local store capacity before evicting the oldest entries.

## LLM Provider Config

```toml
[[providers]]
id = "openai-gpt-4o-mini"
default = true
type = "OpenAI"
responses_native = true
priority = 0
model = "gpt-4o-mini"
tags = ["general", "native-responses"]
credential_file = "./sample/credentials/openai-gpt-4o-mini.toml"
endpoint = "https://api.openai.com/v1"
capacity = { input_tokens = 100000, requests = 50 }
```

### Provider Fields

- `id`: stable routing identifier. Address directly with `model: "id:<provider-id>"`
- `default`: default provider used for fallback
- `type`: `OpenAI`, `Azure`, `Anthropic`, `VertexAI`, `Ollama`, `Bedrock`
- `responses_native`: whether Ayatori should use a native Responses backend for that provider
- `priority`: lower numbers win when multiple providers match
- `model`: upstream model or deployment name
- `tags`: selection labels used by `tags:` and `tag:`
- `credential_file`: TOML file containing provider credentials
- `endpoint`: upstream API base URL
- `capacity`: routing limits

### `responses_native`

Use `responses_native = true` for providers that expose a native Responses API endpoint. In the current codebase:

- `OpenAI`: `true`
- `Azure`: `true`
- `Anthropic`: `false`
- `VertexAI`: `false`
- `Ollama`: `false`
- `Bedrock`: not implemented for Responses yet

When `responses_native = false`, Ayatori converts Responses API requests into the provider's native chat/messages API.

## Credential File Examples

### OpenAI

```toml
[openai-gpt-4o-mini]
type = "OpenAI"
api_key = "OPENAI_API_KEY"
```

### Azure OpenAI

```toml
[azure-gpt-4o-mini]
type = "Azure"
api_key = "AZURE_OPENAI_API_KEY"
deployment = "gpt-4o-mini"
api_version = "2025-04-01-preview"
```

### Anthropic

```toml
[anthropic-claude]
type = "Anthropic"
api_key = "ANTHROPIC_API_KEY"
```

### Vertex AI

```toml
[vertex-gemini]
type = "VertexAI"
api_key = "GOOGLE_API_KEY"
```

### Ollama

```toml
[ollama-gemma3]
type = "Ollama"
```

## Migration Notes

For clients moving from Chat Completions to Responses:

- `messages` becomes `input`
- top-level `system` content becomes `instructions`
- `choices[].message.content` becomes `output[]` plus `output_text`
