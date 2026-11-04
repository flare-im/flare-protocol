# Flare Protocol

English · [Simplified Chinese](README.zh-CN.md)

## Integration Boundary

The open-source stack provides business-neutral messaging and pluggable authentication. Use the Core quickstart with a locally signed token or integrate your identity service through the Core authentication contract. Flare Social provides account and social features in a separate stack. See [Governance](.github/GOVERNANCE.md) for the boundary.


[![Crates.io](https://img.shields.io/crates/v/flare-proto.svg)](https://crates.io/crates/flare-proto)
[![Documentation](https://docs.rs/flare-proto/badge.svg)](https://docs.rs/flare-proto)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.94%2B-orange.svg)](https://www.rust-lang.org/)

`flare-proto` contains the shared protobuf model layer for Flare IM.
It publishes the generated Rust types for common envelopes, messages,
conversation sync payloads, metadata, notifications, data packets, and
event-bus envelopes.

This crate intentionally stays focused on common wire contracts. gRPC service
stubs and tonic clients live in `flare-grpc-proto`.

API documentation: [docs.rs/flare-proto](https://docs.rs/flare-proto)

## Installation

```toml
[dependencies]
flare-proto = "2.1"
```

Optional feature flags are reserved for client and server integrations:

```toml
flare-proto = { version = "2.1", features = ["client"] }
flare-proto = { version = "2.1", features = ["server"] }
```

## What Is Included

- Common message and content models.
- Conversation sync request and response payloads.
- Event, topic, and MQ envelope models.
- Metadata helpers for pagination, filters, actors, devices, audit context,
  and time ranges.
- Push envelope and delivery result models.
- Convenience helpers for packing `prost_types::Any`.

## Quick Start

```rust
use flare_proto::common::message_content::Content;
use flare_proto::{
    Message,
    MessageContent,
    MessageContentExt,
    TextContent,
    encode_message_content,
    pagination_first,
};

let text = TextContent {
    text: "hello flare".to_string(),
    ..Default::default()
};

let content = MessageContent {
    content: Some(Content::Text(text)),
};

// Encode the content value directly.
let content_bytes = content.encode_to_bytes()?;

// encode_message_content takes a complete Message.
let message = Message {
    content: Some(content),
    ..Default::default()
};
let encoded = encode_message_content(&message)?;

let page = pagination_first(20);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Build Behavior

The package uses `prost-build` and a vendored `protoc` binary during builds, so
users do not need to install a system protobuf compiler just to consume the
crate. The `.proto` files remain the source of truth and are included in the
published package.

## Related Crates

| Crate | Purpose |
|-------|---------|
| `flare-proto` | Common protobuf model types and helpers. |
| `flare-grpc-proto` | gRPC service definitions and tonic-generated stubs. |
| `flare-server-core` | Server-side runtime, transport, messaging, auth, and telemetry infrastructure. |

## License

Licensed under the [Apache License 2.0](LICENSE).

---

## Next Steps

| Goal | Start here |
| --- | --- |
| Start a Core messaging server | [Core quickstart](https://github.com/flare-im/flare-core-server/blob/main/docs/QUICKSTART.md) |
| Connect your own identity provider | [Core authentication and token issuance](https://github.com/flare-im/flare-core-server/blob/main/docs/AUTH-TOKEN-ISSUANCE.md) |
| Add business rules around messaging | [Core hook contract](https://github.com/flare-im/flare-core-server/blob/main/docs/HOOK-PLUGIN-CONTRACT.md) |
| Build an IM interface | [Flare UI](https://github.com/flare-im/flare-im-design) |
| Report a security issue | [Security policy](.github/SECURITY.md) |

## Identity and Social Features

The open-source Core provides messaging infrastructure. You can connect your own identity and business logic through the Core authentication and hook contracts. Flare Social is maintained separately and provides accounts, contacts, group management, and moments. See the [Core governance policy](https://github.com/flare-im/flare-core-server/blob/main/.github/GOVERNANCE.md) for the open-source boundary.

## Plugin platform protocol

The business-neutral `flare-plugin-protocol` crate lives in [flare-plugin-protocol](flare-plugin-protocol/README.md). Core, client SDKs and plugin hosts share its Manifest, publisher signatures, Registry, tenant grant and lifecycle contracts. It is independent of business plugin repositories and marketplace availability.

## Compatibility

Review wire changes with the server and SDK maintainers. Keep service RPC definitions in [flare-protocol-grpc](https://github.com/flare-im/flare-protocol-grpc) and coordinate releases when generated consumers change.
