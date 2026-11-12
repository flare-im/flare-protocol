# 变更记录

本文件只记录**已发布到 crates.io 的版本**。版本号遵循
[语义化版本](https://semver.org/lang/zh-CN/)。

## 2.1.0 — 未发布

### 新增

- `schemas/` 增加接入清单、发布清单、在线密钥证书、密钥状态文档的 JSON Schema
  （`access_manifest.v1`、`release_manifest.v1`、`keycert.v1`、`keystate.v1`）。
  真源是 `flare-access` 的 Rust 类型，由它的 `json_schemas` 测试生成并校验。

- `PushPayloadKind` 增加 `PUSH_PAYLOAD_KIND_CAPABILITY = 5`，`PushEnvelope.payload` 增加
  `CapabilityPacket capability = 15`；`PushTaskPayloadKind` 增加 `PUSH_TASK_PAYLOAD_KIND_CAPABILITY = 6`。
  插件实时包（宿主能力 `im.realtime.deliver`）经推送链路只投在线连接，下行为 `DataPacket.capability`，
  不入库、不做离线推送。对 `PushPayloadKind`、`PushTaskPayloadKind` 或 `push_envelope::Payload`
  做穷尽匹配的下游代码需要补上新分支。

- `ConversationUserSettings` 增加 `is_mention_only`（字段号 7）：
  只接收提到我的消息。与 `is_muted` 正交——muted 是「一条都别响」，
  mention_only 是「只有点名才响」，两者可同时开启。
- `ConversationParticipant` 增加 `visible_from_seq`（字段号 7）：
  该成员可见的历史下限。负值是「从加入时刻起」的哨兵，由核心解析成当时的 seq。

- `EventType` 增加 `EVENT_MESSAGE_PLUGIN_STATE = 17`，`Event.payload` 增加
  `PluginStateEvent plugin_state = 25`：插件实体的共享状态快照（`server_msg_id`、
  `plugin_id`、`entity`、`version`、`values_json`），按版本单调覆盖。

- `metadata.proto` 增加设备状态：`DeviceAppState`（`ACTIVE`、`STANDBY`、`PARKING`、`DEFERRED`）、
  `DeviceAttention`（`ATTENTIVE`、`IDLE`）、`DeviceStateReport`、`DeviceStateAck`。
  它们是传输层 `DEVICE_STATE` / `DEVICE_STATE_ACK` 系统命令的载荷：客户端上报应用在前台、
  后台还是即将被挂起，服务端据此决定新消息的提示由这条链路负责还是交给推送通道。
- `topic_envelope.proto` 增加 `AlertClass`（提示等级）与 `PushDisposition`（推送处置），
  `PushTaskEnvelope` 增加字段 10 到 18：`device_id`、`collapse_key`、`thread_key`、
  `disposition`、`alert_class`、`count`、`badge`、`conversation_seq`、`idempotency_key`。
  离线任务由「每个用户一条」改为「每台设备一条」，这些字段只有离线任务填写。
- `topic_envelope.proto` 增加 `DeviceHandoff`、`DeviceHandoffItem`、`DeviceHandoffCause`：
  一台设备的提示从链路转交给推送通道（客户端托管、送达确认超时、链路断开、网关失联）。

- 用户级通知设置，归核心所有：`conversation.proto` 增加 `UserNotificationSettings`
  （`quiet_hours`、`preview`、`multi_device`、`settings_version`）、`QuietHours`
  （起止分钟 0..1439、`weekdays` 按位周一到周日、`allow_mentions`）、`NotificationPreview`
  （`FULL`、`SENDER_ONLY`、`HIDDEN`）与 `MultiDeviceAlertPolicy`（`DELAY`、`IMMEDIATE`、`BADGE_ONLY`）；
  `ConversationUserSettings` 增加 `notify_all`（字段号 9）：大群里显式开启「全部消息通知」。
- `sync.proto`：`Sync.payload` 增加 `UserNotificationSettingsSync user_notification_settings = 26`
  （部分更新：勿扰时段整体替换或 `clear_quiet_hours` 取消、预览、多端策略、
  `base_settings_version` 为 0 时不校验）与 `GetUserNotificationSettingsSync
  get_user_notification_settings = 27`；`SyncRes.payload` 增加同号的
  `UserNotificationSettingsSyncRes` 与 `GetUserNotificationSettingsSyncRes`，都带完整的
  `UserNotificationSettings`。`ConversationUserSettingsSync` 增加 `notify_all`（字段号 8）。
- `sync.proto` 增加 `UserSettingsChanged { conversation_id, settings_version, notify_all }`：
  经同步通道改了用户级通知设置或某会话的 `notify_all` 之后，服务端在该用户的 sync 收件箱
  `sync:<user_id>` 上给他其余在线连接发一帧 `DataPacket.realtime_control`（`control_type` 为
  `custom`，`custom.type` 为 `flare.user_settings_changed`，`custom.payload` 是本消息的编码），
  客户端据此重新读取（会话的 `notify_all` 新值直接在信号里）。只在线、尽力而为；客户端发往
  `sync:` 收件箱的实时帧一律被网关丢弃。对 `sync::Payload` 或 `sync_res::Payload` 做穷尽匹配、或按全部字段
  构造 `ConversationUserSettingsSync` 的下游代码需要补上新分支与新字段。

- `PushTaskEnvelope` 增加 `preview`（字段 19，接收者的通知预览设置）与 `digest_conversations`
  （字段 20，摘要通知代表的会话数）。`ConversationSummary` 增加 `notify_all`（字段 25），客户端据此读回
  自己在大群里是否开启了「全部消息通知」。

以上都是**向后兼容的新增**：字段号与枚举值此前未被占用，老客户端不发送、老服务端忽略。
因此升 minor 而非 major。对 `EventType` 或 `event::Payload` 做穷尽匹配的下游代码需要补上新分支。

## 2.0.1 — 2026-08-03

与实现层 1.1.0 对齐的契约层发布。
