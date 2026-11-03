# Flare Proto

English · [中文](README.zh-CN.md)

> ## ℹ 这是通信基础设施，不是开箱即用的 IM 产品
>
> 说在前面，免得你 clone 完才发现登不上去：**开源部分不含账号体系**
> （没有注册登录、好友关系、群角色/审批/禁言、朋友圈）。
>
> 但它自带完整且可插拔的鉴权契约，两条路都在开源侧：
>
> - **`CoreJwtTokenValidator`** —— 本地验 JWT。手签一个 token 就能跑起来做
>   demo / POC，**不需要任何用户体系**。
> - **`HttpHookTokenValidator`** —— 把 token POST 到你自己的接口，
>   **这是接入自有用户体系的入口**。
>
> 业务规则同理：`flare-core-server/crates/flare-im-hooks` 提供 9 个扩展点
> （PreSend / PostSend / Delivery / Recall / MessageRead / MessageReaction /
> ConversationLifecycle / ConversationMember / GetConversationParticipants）。
>
> 要上生产，你需要自行实现用户体系并按上述契约接入 —— 与 Sendbird /
> Twilio Conversations 的「自带身份」模型一致，区别是 Flare 可自托管、
> 协议与核心可审计。
>
> 边界详情见 [GOVERNANCE.md](.github/GOVERNANCE.md)。


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

// 直接编码 content 本身
let content_bytes = content.encode_to_bytes()?;

// encode_message_content 接的是一条完整的 Message，不是 MessageContent
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

## 下一步

| 想做什么 | 去哪里 |
|---|---|
| **五分钟跑起来** | [QUICKSTART](https://github.com/flare-im/flare-core-server/blob/main/QUICKSTART.md) —— 起服务、手签 token、调通接口，**不需要自建用户体系** |
| 接入自己的用户系统 | 实现 `TokenValidator`（`CoreJwtTokenValidator` 本地验签 / `HttpHookTokenValidator` 调你的接口） |
| 加自己的业务规则 | `flare-im-hooks` 的 9 个扩展点：PreSend / PostSend / Delivery / Recall / MessageRead / MessageReaction / ConversationLifecycle / ConversationMember / GetConversationParticipants |
| 做界面 | [`@flare-im/vue-ui`](https://www.npmjs.com/package/@flare-im/vue-ui) —— 107 个组件，四端一致的契约 |
| 报安全问题 | [SECURITY.md](.github/SECURITY.md)，**请勿开公开 issue** |

## 需要账号体系与社交能力时

开源部分是**通信基础设施**。如果你需要的是现成的账号、好友关系、群治理（角色 / 入群审批 / 禁言）、朋友圈，
这些在商业模块里 —— 自研这一层通常要数月，且都是与通信无关的重复劳动。

企业场景另有 SSO / 组织架构 / 审计导出 / 数据驻留 / SLA 支持。

咨询：`flare1522@163.com`

> 边界划分与不变承诺见 [GOVERNANCE](https://github.com/flare-im/flare-core-server/blob/main/GOVERNANCE.md)。
> 简言之：**已开源的不会被收回，鉴权与 hooks 契约永远开源、不会为逼迫付费而阉割。**

## Plugin platform protocol

The business-neutral `flare-plugin-protocol` crate lives in [flare-plugin-protocol](flare-plugin-protocol/README.md). Core, client SDKs and plugin hosts share its Manifest, publisher signatures, Registry, tenant grant and lifecycle contracts. It is independent of business plugin repositories and marketplace availability.
