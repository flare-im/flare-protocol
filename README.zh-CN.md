# Flare Protocol

[English](README.md) · 简体中文

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

`flare-proto` 包含 Flare IM 的共享 protobuf 模型层。它发布用于通用信封、
消息、会话同步载荷、元数据、通知、数据包以及事件总线信封的生成的 Rust 类型。

本 crate 有意专注于通用的传输契约。gRPC 服务桩和 tonic 客户端位于
`flare-grpc-proto`。

API 文档：[docs.rs/flare-proto](https://docs.rs/flare-proto)

## 安装

```toml
[dependencies]
flare-proto = "2.1"
```

可选的 feature 标志为客户端与服务端集成预留：

```toml
flare-proto = { version = "2.1", features = ["client"] }
flare-proto = { version = "2.1", features = ["server"] }
```

## 包含内容

- 通用消息与内容模型。
- 会话同步请求与响应载荷。
- 事件、主题与 MQ 信封模型。
- 用于分页、过滤器、操作者、设备、审计上下文与时间范围的元数据辅助工具。
- 推送信封与投递结果模型。
- 用于打包 `prost_types::Any` 的便捷辅助工具。

## 快速开始

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

## 构建行为

该包在构建期间使用 `prost-build` 和一个内置（vendored）的 `protoc` 二进制文件，
因此使用者无需为了消费该 crate 而安装系统级的 protobuf 编译器。`.proto` 文件
仍是唯一真源，并包含在已发布的包中。

## 相关 Crate

| Crate | 用途 |
|-------|------|
| `flare-proto` | 通用 protobuf 模型类型与辅助工具。 |
| `flare-grpc-proto` | gRPC 服务定义与 tonic 生成的桩。 |
| `flare-server-core` | 服务端运行时、传输、消息、鉴权与遥测基础设施。 |

## 许可证

依据 [Apache License 2.0](LICENSE) 授权。

---

## 下一步

| 想做什么 | 去哪里 |
|---|---|
| **五分钟跑起来** | [QUICKSTART](https://github.com/flare-im/flare-core-server/blob/main/docs/QUICKSTART.zh-CN.md) —— 起服务、手签 token、调通接口，**不需要自建用户体系** |
| 接入自己的用户系统 | 实现 `TokenValidator`（`CoreJwtTokenValidator` 本地验签 / `HttpHookTokenValidator` 调你的接口） |
| 加自己的业务规则 | `flare-im-hooks` 的 9 个扩展点：PreSend / PostSend / Delivery / Recall / MessageRead / MessageReaction / ConversationLifecycle / ConversationMember / GetConversationParticipants |
| 做界面 | [`@flare-im/vue-ui`](https://www.npmjs.com/package/@flare-im/vue-ui) —— 107 个组件，四端一致的契约 |
| 报安全问题 | [SECURITY.md](.github/SECURITY.md)，**请勿开公开 issue** |

## 需要账号体系与社交能力时

开源部分是**通信基础设施**。如果你需要的是现成的账号、好友关系、群治理（角色 / 入群审批 / 禁言）、朋友圈，
这些在商业模块里 —— 自研这一层通常要数月，且都是与通信无关的重复劳动。

企业场景另有 SSO / 组织架构 / 审计导出 / 数据驻留 / SLA 支持。

咨询：`flare1522@163.com`

> 边界划分与不变承诺见 [GOVERNANCE](https://github.com/flare-im/flare-core-server/blob/main/.github/GOVERNANCE.md)。
> 简言之：**已开源的不会被收回，鉴权与 hooks 契约永远开源、不会为逼迫付费而阉割。**

## 插件平台协议

通用 `flare-plugin-protocol` crate 位于 [flare-plugin-protocol](flare-plugin-protocol/README.md)。Core、客户端 SDK 与插件宿主复用 Manifest、发布者签名、Registry、租户授权与生命周期契约，不依赖业务插件仓库或市场在线。

## 兼容性

线协议变更由服务端与 SDK 维护者共同评审。服务 RPC 定义放在 [flare-protocol-grpc](https://github.com/flare-im/flare-protocol-grpc)，影响生成代码的变更需要协调发布。
