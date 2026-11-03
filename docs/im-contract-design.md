# Flare Proto: General IM Contract Design

> Scope: the `flare-proto` common contract and its immediate downstream consumers.

## Goal

Make `flare-proto` the strict, production-grade, business-neutral contract for a generic IM core that can support any business domain through hooks, capability plugins, custom payloads, and SDK extension points.

The contract must preserve IM invariants: per-conversation monotonic sequence, explicit identity separation, idempotent send and operation acknowledgements, offline-first sync, multi-device convergence, CQRS-friendly read models, and opaque extension slots that do not carry stable core semantics.

Compatibility with previous local prototypes is explicitly out of scope. The protocol should be shaped as the clean target contract, with obsolete fields, names, enum values, and mixed responsibilities removed rather than kept as shims. Removed protobuf field numbers and names should still be reserved to prevent accidental wire reuse.

## Architectural Direction

`flare-proto` should define only common IM contracts. It is not the place for Social rules, product-specific content catalogs, WebRTC/SFU control details, legacy prototype adapters, or business workflows.

The core package remains `flare.common.v1` and keeps:

- Identity and context shapes used by IM services and SDKs.
- Message identity, ordering, status, typed content envelope, retention state, and push display hints.
- Conversation read models and user-level conversation settings.
- Event, ACK, sync, error, MQ, data, and notification envelopes.
- Extension envelopes for custom message content, durable custom events, and non-durable capability control packets.

Business and optional capability contracts move outward:

- Social user, relation, and group rules stay in `flare-social` and `flare-social-proto`.
- RTC/WebRTC/SFU-specific signal payloads stay in `flare-sdk-plugin` / `flare-plugin` and service proto such as `sfu_control.proto`.
- Product-specific message content such as vote, task, schedule, mini program, and announcement becomes app-card or custom content, unless a capability plugin owns a typed payload.

## Industry Alignment

Feishu/Lark alignment:

- Keep message pipeline neutral and use pre-send hooks for business gates.
- Keep directory, profile, relation, and group cursors out of IM message seq.
- Provide capability discovery and typed extension slots rather than bloating core message types.

WeChat alignment:

- Preserve reliable send FSM with separate `client_msg_id`, `server_id`, `seq`, `request_id`, ACK, and sync cursor.
- Keep weak-network optimistic UI and server convergence possible.
- Treat recall, edit, delete, read, retention, reaction, pin, and mark as events, not historical message rewrites.

Telegram alignment:

- Keep per-conversation seq sync and cloud replay.
- Support conversation type differences without hard-coding business directories into IM core.
- Support bots and capabilities through command and event extension surfaces.

## Assumptions

- Existing local prototypes do not require compatibility shims.
- `flare-proto` should make breaking API changes wherever they produce a cleaner target contract.
- `flare-im-core` remains the server-side authority for message seq, storage, sync, and push orchestration.
- `flare-im-core-sdk` remains the client-side authority for reliable sending, local storage, sync, and dispatch behavior.
- Optional RTC and burn-after-read behavior currently exists in downstream code, but the target protocol must replace those shapes with capability control packets and generic retention naming.

## Bounded Contexts

### Core Message

Owns:

- `server_id`, `client_msg_id`, `conversation_id`, `channel_id`, `conversation_seq`, `message_seq`, `sender_id`, `message_type`, typed `MessageContent`, and `MessageStatus`.
- Generic retention and visibility state.
- Opaque extension maps named consistently: public string maps use `attributes`, binary maps use `extensions`, and transport/MQ context maps use `headers`.

Does not own:

- Friend rules, group membership policy, red packets, payments, approval flows, SFU rooms, or media-plane negotiation.
- Sender profile rules, user directory shape, or business display names. Those are read-model snapshots or Social projections, not message aggregate invariants.

### Content Envelope

Owns stable, generic renderable content:

- Text, image, video, audio, file, location, card/app-card, sticker, emoji, quote, forward, thread, rich text, image group, system, notification, custom, placeholder.

Product-specific structures become:

- `AppCardContent` for generic application cards.
- `CustomContent` for domain-specific payloads.
- Capability-owned typed payloads outside `flare.common.v1`.

### Event Stream

Owns:

- Ordered domain events that mutate message or conversation read models.
- Operation events for recall, edit, delete, read receipt, reaction, pin, mark, retention lifecycle, plugin entity state, conversation update, and durable custom events.

Durable capability facts use `CustomEvent` with a namespaced type only when they intentionally affect replayable state. Live capability signaling such as RTC invite, SDP, ICE, SFU hints, audio level, and network quality does not enter the durable event stream and does not consume conversation sequence.

### Realtime Control

Owns:

- Typing state.
- Presence hints.
- Non-durable capability packets.
- Connection-scoped control packets.

Realtime control packets are best-effort, do not enter history sync, and do not consume `conversation_seq`.

### Sync

Owns:

- Conversation snapshot.
- Per-conversation message/event increment.
- Event replay plan.
- Cursor update.
- Recovery hints for stale cursors and seq gaps.

Sync must not become a generic command bus for unrelated writes. User conversation settings mutation belongs to an explicit command surface in `flare-grpc-proto`; sync only observes the resulting read-model changes.

### ACK

Owns:

- Send ACK.
- Event ACK.
- Push/window ACK.
- Conversation delivery ACK.
- Explicit read ACK.
- Batch ACK for backpressure and weak-network efficiency.

ACK must keep success/failure and structured error information explicit through typed result payloads.

### Capability

Owns:

- `capability_id`, `packet_type`, payload bytes, version, and attributes.
- Routing to plugin implementations.

Specific payloads are owned outside common by `flare-plugin`, `flare-sdk-plugin`, or service-specific proto packages.

Capability control packets are not domain events by default. A plugin may emit a durable custom event only after the core accepts that the event changes replayable IM state.

## Command, Query, Event, Recovery

Command path:

1. SDK creates local pending message with `client_msg_id`.
2. Gateway or orchestrator receives send command.
3. Core runs hooks and capability checks.
4. Core assigns `server_id`, `conversation_seq`, `message_seq`, and server timestamp.
5. Core writes message/event to storage and event bus.
6. Core returns `SendAck`.
7. Downstream consumers update read models and push tasks.

Query path:

1. SDK syncs conversation list by cursor.
2. SDK syncs a conversation by `last_conversation_seq`.
3. SDK replays critical events using event replay policy.
4. SDK reads history and detail views from read-optimized query models.

Event path:

1. All state-changing operations become typed events.
2. Events are idempotent by `event_id` and ordered by `conversation_seq` where they affect a conversation timeline.
3. Ephemeral states such as typing, presence, and live call hints use realtime control packets instead of durable events.

Recovery path:

1. Client reports last applied `conversation_seq` and cursor.
2. Server detects stale cursor, retention cutoff, or missing event range.
3. Server returns structured recovery hints.
4. Client refetches snapshot, resyncs one conversation, or replays events based on the hint.

## Recommended Design

### 0. Rewrite Around Target Invariants

Do not carry previous prototype shapes forward. The target protocol should be internally coherent even if every downstream crate must be updated.

Hard decisions:

- `Message.content` is a typed `MessageContent`, not raw bytes.
- `conversation_seq` is the replay sequence for all durable conversation events.
- `message_seq` is present only for message-created records and may have gaps relative to `conversation_seq`.
- `MessageStatus` represents author/persistence lifecycle only; delivery and read state are per-user events/read models.
- `request_id`, `client_msg_id`, `server_id`, `event_id`, ACK id, and sync cursor remain separate identifiers.
- Body-level tenant, actor, and request context is avoided on client-facing messages; gRPC metadata, MQ headers, or connection context are authoritative.
- `business_type` is removed from stable common contracts. Business routing uses hooks, capability ids, labels, custom content, or extension attributes.

### 1. Rename Burn Semantics To Generic Retention

Replace business-specific burn terminology with generic retention naming in `message.proto` and `event.proto`.

Recommended common concepts:

- `MessageRetentionPolicy`: retention mode, expire trigger, delay seconds, visibility after expiration.
- `MessageRetentionState`: current lifecycle, first trigger time, expire at, expired at, purged at.
- `MessageRetentionEvent`: scheduled, expired, purged.

Existing burn-after-read maps to:

- trigger: `RETENTION_TRIGGER_AFTER_READ`
- visibility after expiration: `CONTENT_VISIBILITY_HIDDEN_OR_REDACTED`
- lifecycle: scheduled, expired, purged

This keeps Signal/Telegram-style self-destruct messages possible without making the core contract sound like one specific feature.

### 2. Move RTC-Specific Signaling Out Of Common Events

Remove direct `event.proto` dependency on `call_signal.proto`, and do not replace it with a durable `EVENT_CAPABILITY_SIGNAL`.

Add a non-durable capability control packet outside the conversation event log:

- `CapabilityPacket`
- fields: `capability_id`, `packet_type`, `version`, `payload`, `attributes`, `correlation_id`

RTC sends:

- `capability_id = "rtc.call"`
- `packet_type = "invite" | "accept" | "hangup" | "ice" | "sfu.join_hints" | ...`
- `payload` is plugin-owned protobuf or JSON.

The `flare-sdk-plugin-call` package keeps typed builders for call payloads, but those builders encode plugin-owned payloads into `CapabilityPacket`. This preserves typed call ergonomics without binding common IM proto to SFU details. If a call needs a timeline-visible artifact, it creates a normal system/custom message or app card through the message pipeline.

Detailed WebRTC/SFU signaling guidance is documented in `docs/webrtc-sfu-signaling-over-flare-proto.md`.

### 3. Collapse Product-Specific Content Into App Card Or Custom

Keep stable generic types only.

Recommended retained content types:

- Text
- Image
- Video
- Audio
- File
- Location
- Card/AppCard
- Sticker
- Emoji
- Quote
- Forward
- Thread
- RichText
- ImageGroup
- System
- Notification
- Custom
- Placeholder

Remove from common:

- Vote
- Task
- Schedule
- Announcement
- MiniProgram

These become `AppCardContent` or `CustomContent`. Generated SDKs can still expose convenience builders, but those builders must emit app-card/custom payloads rather than requiring new core enum variants.

### 4. Normalize Sync As Query And Recovery

Keep `Sync` as a data-channel request/response surface for IM sync, not as a mixed command bus.

Recommended shape:

- Snapshot query.
- Conversation list incremental query.
- Conversation detail query.
- Conversation participants query.
- Single and multi-conversation message/event query.
- Event stream ACK.
- Cursor get/update.

`UpdateConversationUserSettingsSync` is removed from target sync. The settings command writes through application command APIs, emits a conversation/settings event, and appears to clients through conversation list/detail sync.

### 5. Keep ACK First-Class And Typed

The current addition of `ReadAck` and correction from `ACK_TYPE_CONVERSTION` to `ACK_TYPE_CONVERSATION` is correct.

Improve the contract by ensuring:

- Every ACK has idempotency identity where needed.
- Send ACK uses a result `oneof`: `accepted` carries server message identity and sequence, while `error` carries structured `ErrorDetail`.
- Read ACK is separate from delivery ACK.
- Push/window ACK is tied to `EventEnvelope.window_id`.
- Batch ACK supports multiple conversations and devices.

## Alternatives

### Alternative A: Minimal Patch

Only fix downstream compile drift by updating `flare-core-server/src/message/convert.rs` to current burn fields.

Why not:

- Leaves RTC/SFU in common.
- Leaves product-specific message catalog in core.
- Does not achieve the requested generic IM core standard.

### Alternative B: Keep Common As A Full Product Catalog

Keep burn, call, vote, task, schedule, announcement, and mini program as first-class common types.

Why not:

- Makes `flare-proto` resemble one application suite instead of a generic IM core.
- Forces every SDK/platform to carry every optional business feature.
- Makes plugin and capability boundaries weaker over time.

### Recommended: Strict Core Plus Extension/Capability

This is the best fit for a generic IM platform. Core remains small, stable, and production-grade. Optional capabilities stay typed, but outside common.

## Contract Impact

`flare-proto`:

- Modify `message.proto`, `event.proto`, `message_content.proto`, `data.proto`, `sync.proto`, `ack.proto`, `conversation.proto`, `README.md`, and `IM_PROTO_DESIGN.md`.
- Add retention/capability-neutral naming.
- Remove common import dependency from event to call-specific proto.
- Remove body-level `business_type` from common message/conversation contracts.
- Replace raw `bytes content` with typed `MessageContent content`.
- Split replay `conversation_seq` from message-only `message_seq`.
- Reserve removed field numbers and enum values.

`flare-grpc-proto`:

- Update re-exports.
- Update message service burn/read commands to retention naming.
- Keep plugin/capability service responsible for capability payload routing.

`flare-im-core`:

- Replace `BurnConfig` / `MessageBurnState` conversion with new retention fields.
- Update storage writer/reader/orchestrator to use retention naming.
- Rename domain and storage-facing fields to retention naming. Existing migrations may be superseded because compatibility is out of scope.

`flare-im-core-sdk`:

- Update message model and sync policy to retention naming.
- Apply capability packet handling for call plugin control traffic.
- Keep offline-first send/sync behavior unchanged.

`flare-sdk-plugin-call`:

- Own typed call payloads and builder APIs.
- Encode/decode call payloads through non-durable `CapabilityPacket`.

`flare-im-core-client-sdk`:

- Update sdk-spec message models.
- Regenerate platform bindings and docs.
- Expose app-card/custom builders for product-specific content.

## Consistency Guarantees

- `conversation_seq` is the only durable replay sequence.
- `message_seq` is a message-created ordering aid and is not used as the event replay cursor.
- `client_msg_id`, `server_id`, `event_id`, `request_id`, ACK ids, and cursor values remain distinct.
- Retention expiration does not rewrite historical ordering.
- Recall/edit/delete/read/reaction/pin/mark/retention are events, not in-place timeline mutations.
- Conversation read models can be eventually consistent, but recovery hints must tell SDKs when snapshot refetch is required.
- Social cursors and IM seq remain separate.
- Capability control packets do not affect core ordering. Only explicit durable custom events or messages enter `conversation_seq`.

## Error Handling

- gRPC surfaces transport/application errors through `tonic::Status` with `ErrorDetail` where available.
- `SendAck` retains structured error details through its `error` result for optimistic UI convergence.
- Internal push delivery reports use result `oneof` payloads instead of `success` flags plus loose error strings.
- Hook denial maps to structured error reason and retry advice.
- Sync stale states use `SyncStaleContext` and `SyncRecoveryHint`.
- Capability packet failures should not poison the core sync stream; they return capability-specific errors through the capability service or custom payload response.

## Testing Strategy

Smallest checks:

- `cargo test` in `flare-proto`.
- Focused contract tests for ACK, retention event wire values, and capability packet encoding.

Downstream checks:

- `cargo check` in `flare-grpc-proto`.
- `cargo check -p flare-im-core` or equivalent in the `flare-im-core` workspace.
- Focused tests for message conversion, retention lifecycle, sync policy, and event application.
- SDK contract/codegen verification for `flare-im-core-client-sdk`.

Regression focus:

- Send ACK identity mapping.
- Read ACK versus delivery ACK.
- Retention scheduling and expiration idempotency.
- Critical event replay includes retention events.
- Capability packets do not introduce dependency from common proto to plugin proto and do not consume `conversation_seq`.
- Conversation sync still returns per-conversation `conversation_seq`, snapshot, cursor, and recovery hints.

## Bottlenecks And Risks

- Downstream drift already exists: `flare-im-core` currently references removed `BurnConfig` / `MessageBurnState`.
- Generated SDKs may require wide updates if message content enum values change.
- Moving call signals out of common affects `flare-sdk-plugin-call`, `flare-im-core-sdk` listeners, and existing generated client bindings.
- Overusing `CustomContent` can become stringly typed. Capability-owned payload schemas and SDK builders should be used for high-value recurring capabilities.

## Implementation Order

1. Stabilize `flare-proto` common contract.
2. Delete common RTC/burn/product-catalog shapes and replace them with retention, app-card/custom, and capability packet contracts.
3. Update `flare-grpc-proto` extern paths and re-exports.
4. Update `flare-im-core` conversion and orchestration.
5. Update `flare-im-core-sdk` models, sync policy, and event handling.
6. Update `flare-sdk-plugin-call` to encode call payloads through capability packets.
7. Update sdk-spec and regenerate platform adapters.
8. Run targeted checks from common contract outward.
