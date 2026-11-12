# flare-notify-rules

[English](README.md)

Flare IM 提示规则的唯一实现：一条消息对一台设备，是响铃、静默出现、合并进这个会话已有的那条通知，
还是等一等。推送服务端在设备进程不在时运行它，客户端引擎在进程内运行它，同样的输入在两边得到同样的结果。

这个 crate 只做纯计算：不做 IO、不读时钟（时刻一律是调用方传入的 `now_ms`，Unix 毫秒）、不用随机数、
不起异步、不依赖协议 crate。规则本身从不分配内存，只有 `text` 模块里拼数字的两个函数用到 `alloc`。
它是 `no_std`，可以编到 `wasm32-unknown-unknown`。`serde` 特性给持久化状态加上 serde 派生。

## 两步

```rust
use flare_notify_rules::*;

// 1. 偏好：不提示、静默、候选、延迟或只更新角标。
let decision = decide(&DecideInput {
    class: AlertClass::Normal,
    mentioned: Mentioned::No,
    from_self: false,
    conversation: ConversationPrefs::default(),
    user: UserPrefs::default(),
    local_time: Some(MinuteOfWeek::from_utc_ms(now_ms, utc_offset_minutes)),
    other_device_attentive: false,
});

// 2. 频率与总量：闸门，状态由调用方保存。
if let Some(tier) = decision.tier() {
    let input = GateInput { kind: ConversationKind::Group, tier, tag: ConversationTag::of(id) };
    let admitted = Gate::default().admit(now_ms, input, &mut slot, &mut device);
    // admitted.slot：Loud | Quiet | Defer { due_ms } | CountOnly
    // admitted.digest：摘要通知同时响铃时为 Some(DigestAlert)
}
```

### 接口

| 项 | 作用 |
|---|---|
| `decide(&DecideInput) -> Decision` | 偏好规则（见下）。`Decision::{Drop, Quiet, Candidate { priority }, Delay { delay_ms, then }, BadgeOnly}`；`tier()` 给出进闸门的档位，`without_delay()` 在无处登记到期时去掉推迟。 |
| `Tier::{Quiet, Normal, Priority}` | 闸门输入。Quiet 从不响铃；Normal 受预算约束；Priority（点名、紧急）绕过槽位阶梯、令牌桶与摘要，只受设备最小间隔约束。 |
| `Gate::admit(now_ms, GateInput, &mut SlotState, &mut DeviceState) -> Admitted` | 一条消息进闸门。 |
| `Gate::on_due(now_ms, &mut SlotState, &mut DeviceState) -> DueAction` | 登记的尾部刷新到期：`Tail { count }`、`Defer { due_ms }`（改登记）或 `Nothing`。 |
| `Gate::on_read(now_ms, &mut SlotState, &DeviceState) -> ReadOutcome` | 会话在任一处被读：关闭槽位；`clear` 表示这台设备要不要发清除修正，`cancel_tail` 表示可以移除登记的尾部刷新。 |
| `Gate::on_active(&mut DeviceState)` | 设备回到 `Active`：它的全部槽位作废，退出摘要模式。 |
| `fail_open(Tier) -> SlotAction` | 状态读不到或写不回：按会话第一条处理（档位不是 Quiet 就响铃）。 |
| `GateConfig` / `GateConfig::validate` | 参数（见下）；租户覆盖值先校验再用。 |
| `SlotState`、`DeviceState` | 持久化状态，`encode() -> [u8; N]`、`decode(&[u8])`，`version` 公开。 |
| `MinuteOfWeek` | 设备当地时间（周一 00:00 为 0），由 `from_utc_ms(utc_ms, offset_minutes)` 或 `from_local_ms` 得到。 |
| `PreviewMode::strictest` | 合并用户的预览设置与租户强制的 `Hidden`。 |
| `text::*` | 推送 worker 与引擎共用的通知文案（见下）。 |

## 通知文案

`flare_notify_rules::text` 把兜底文案只写一次：同一条通知不论由推送 worker 还是引擎渲染，说法都一样。

```rust
pub enum Lang { Zh, En }                                    // 默认 Zh
impl Lang { pub fn from_locale(tag: &str) -> Lang }
pub enum Phrase { GenericTitle, GenericBody, NoticeBody, SentMessage, Recalled, MentionMarker,
                  Image, Video, Voice, File, Location, Contact, Link, Sticker, Emoji, RichText, Message }
pub const fn phrase(lang: Lang, phrase: Phrase) -> &'static str;
pub fn count_prefix(lang: Lang, count: u32) -> Option<String>;
pub fn digest_text(lang: Lang, conversations: u32, messages: u32) -> String;
pub fn mentions_already(text: &str) -> bool;
```

- `from_locale`：主语言子标签（按 `-`、`_`、`.`、`@` 切开，不分大小写）是 `zh` 的为 `Zh`，其余非空标签为 `En`，空串或空白为 `Zh`。所以 `zh-Hant-TW`、`zh_CN.UTF-8` 都是中文。
- `phrase`：固定文案。占位（`Image` 到 `Message`）带方括号，与设计组件库的消息摘要一致：`[图片]` / `[Image]`；`Message` 用于没有专门占位的类型与空白文本。
- `count_prefix`：不到 2 为 `None`；否则是 `[N条] ` / `[N messages] `（末尾带空格），`N` 超过 99 写成 `99+`。
- `digest_text`：`N 个会话有 M 条新消息` / `M new messages in N chats`；会话数不到 2 时只说条数（`M 条新消息` / `M new messages`、`1 new message`）。`M` 不少于会话数，也不少于 1。
- `mentions_already`：文字里已经说了被点名（`有人@你`、`有人@我`、`[Mentioned you]`、`[Mentioned]`），不再重复加点名标记。

## 决策顺序

1. 等级 `none`，或消息是接收者本人发的：`Drop`。
2. 会话免打扰：`urgent` 为优先候选，其余 `Drop`。
3. 会话仅 @我 且没点名：`Drop`。这里 `Mentioned::Unknown` 按点名处理。`urgent` 不被丢弃（否则「仅 @我」比免打扰还严）。
4. 勿扰时段内：`urgent` 以及设置放行时的点名为优先候选，其余 `Quiet`。没有当地时间（未登记时区）时勿扰时段不生效。
5. 同一用户另有设备活跃且在看（`urgent` 除外）：策略 `Delay` 给出 `Delay { 15000 ms, then }`，`Immediate` 继续往下，`BadgeOnly` 给出 `BadgeOnly`。点名跟随策略。
6. 等级 `silent`：`Quiet`。
7. 点名与 `urgent`：`Candidate { priority: true }`；其余 `Candidate { priority: false }`。

点名指等级 `mention`，或等级 `normal` 且 `Mentioned::Yes`。勿扰时段：`start == end` 为全天；`end < start` 跨午夜，零点以后的部分属于开始的那一天；含起点不含终点；`weekdays` bit0 周一 … bit6 周日，0 为每天。偏好查询失败时用默认值，默认值就是放行取值。

## 闸门

每个（设备，会话）一个槽位，一个槽位对应一条通知；每台设备一份设备状态。

- **空闲槽位的第一条**一定发出：设备预算允许就响铃，否则静默。只有厂商调用硬上限能让它等，那时它成为上限窗口结束时的尾部刷新。
- **之后的更新**替换同一条通知，显示最新一条与计数。每个槽位两条自适应阶梯：更新阶梯（本槽位任意两次发送的最小间隔）与响铃阶梯（本槽位两次响铃的最小间隔）。响铃也是一次更新，更新间隔从响铃那一刻重新算。落在更新间隔内的消息被吞并：第一次吞并返回 `Defer { due_ms }`，调用方登记尾部刷新；之后返回 `CountOnly`。尾部刷新总是静默的。
- **阶梯升降**：刚结束的间隔里有消息被吞并（更新阶梯）或被压住没响（响铃阶梯），就在下一次发送或响铃时升一档；当前档位每有一个完整间隔没有消息就降一档。
- **设备预算**：响铃令牌桶；任意两次响铃之间的设备最小间隔（所有档位，含摘要）；厂商调用硬上限，按固定窗口计，窗口从上一个窗口结束后的第一次调用起算。
- **摘要模式**：进入窗口内产生推送的会话数达到阈值时进入；不算当前这次、退出窗口内推送少于 2 次，或 `on_active` 时退出。摘要模式下普通消息不在槽位上响铃；另有一条摘要通知，距本期起点与上一次任何响铃都满一个摘要间隔时响铃。`DigestAlert { conversations, messages }` 是本期（进入或上一次摘要响铃以来）的数。
- **过期是惰性的**：空闲满过期时长、或在设备上一次 `on_active` 之前打开的槽位，下次碰到时视同不存在。不存在与 `default()` 同义。

| `GateConfig` 字段 | 默认值 |
|---|---|
| `quiet_ladder` | 2s、5s、15s、60s、300s |
| `loud_ladder_direct` | 10s、30s、120s、300s |
| `loud_ladder_group` | 30s、120s、600s |
| `slot_idle_expiry_ms` | 30 分钟 |
| `loud_bucket_capacity` / `loud_bucket_refill_ms` | 3 / 15s |
| `device_min_loud_gap_ms` | 1s |
| `digest_enter_conversations` / `digest_enter_window_ms` | 6 / 60s |
| `digest_exit_window_ms` | 120s（推送少于 2 次） |
| `digest_ladder` | 60s、120s、300s |
| `vendor_cap` / `vendor_cap_window_ms` | 30 / 60s |

设计里的例子，由 `tests/vectors/gate.json` 回放：

- 一个群一小时一万条：响铃在第 0、30、150、750 秒，之后每 600 秒一次（共 8 次）；静默尾部刷新 15 次；厂商调用 23 次；始终一条通知，最后一次更新显示 10000。
- 二十个会话在一秒内同时开始：第六个会话起进入摘要模式；响铃在第 0、60、180、480 秒，任意 60 秒内不超过一次。

## 持久化状态

每个改动了状态的操作都给它的 `version` 加一。调用方记住读出时的版本，只在存储里的值仍是这个版本时写回
（比较后写入）；冲突重试用完就用 `fail_open`。编码定长、小端序；第 0 字节是布局号（1），第 1 到 8 字节是
`version`，脚本不解码也能比较这 8 个字节。解码失败的值按不存在处理。时刻用 `i64::MIN` 表示「从未」。
给槽位设不短于 `slot_idle_expiry_ms` 的存储过期时间，与惰性过期结果一致。

`SlotState`：58 字节。

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 | u8 | 布局号 = 1 |
| 1 | u64 | version |
| 9 | u32 | 打开时的设备代号 |
| 13 | i64 | 最近一条消息 |
| 21 | i64 | 最近一次发送（本槽位的任意厂商调用） |
| 29 | i64 | 最近一次响铃 |
| 37 | i64 | 登记的尾部刷新 |
| 45 | u32 | 打开以来的消息数 |
| 49 | u32 | 吞并未显示的消息数 |
| 53 | u8 | 更新阶梯档位 |
| 54 | u8 | 响铃阶梯档位 |
| 55 | u8 | 标志：bit0 响铃被压住，bit1 更新被吞并 |
| 56 | u16 | 摘要期号标记 |

`DeviceState`：169 字节。

| 偏移 | 类型 | 字段 |
|---|---|---|
| 0 | u8 | 布局号 = 1 |
| 1 | u64 | version |
| 9 | u32 | 设备代号 |
| 13 | i64 | 最近一次响铃 |
| 21 | u8 | 令牌欠账（已用未补的令牌数） |
| 22 | i64 | 令牌补充计时起点 |
| 30 | i64 | 厂商调用窗口起点 |
| 38 | u8 | 窗口内厂商调用次数 |
| 39 | i64 | 最近一次推送 |
| 47 | i64 | 再前一次推送 |
| 55 | u8 | 标志：bit0 摘要模式，bit1 摘要被压住 |
| 56 | u8 | 摘要阶梯档位 |
| 57 | u16 | 摘要期号（从不为 0） |
| 59 | i64 | 本期起点 |
| 67 | u16 | 本期会话数 |
| 69 | u32 | 本期消息数 |
| 73 | 8 × (u32, i64) | 最近会话：标签、最近一次推送 |

## 向量

`tests/vectors/decide.json` 与 `tests/vectors/gate.json` 由 `tests/vectors.rs` 回放。推送服务端与引擎用
各自的接线（状态存储、到期队列）回放同样的文件，结果必须一致。

输入（决策用例、场景、参数、事件）是手写的；期望输出由本 crate 生成。改了规则、或加了用例与场景之后，运行

```sh
FLARE_NOTIFY_RULES_WRITE_VECTORS=1 cargo test --test vectors
```

再看 diff。只有每个用例与场景的 `expect` 被重写；排版（键序、短对象一行）保持不变，规则没变时文件逐字不变。
新场景用占位声明要哪种期望：`"expect": {"log": []}` 要完整日志，`"expect": {}` 要汇总。两个设计例子的验收口径
在两种模式下都会断言。不设这个变量时（CI 里也是）测试只做断言。

### decide.json

`{"format": 1, "cases": [{"name", "input", "expect"}]}`。`input` 每个字段都给出：
`class`（`none|silent|normal|mention|urgent`）、`mentioned`（`no|yes|unknown`）、`from_self`、
`conversation {muted, mention_only}`、`user {quiet_hours: null | {start_minute, end_minute, weekdays,
allow_mentions}, preview: full|sender_only|hidden, multi_device: delay|immediate|badge_only}`、
`local_time: null | {weekday（0 为周一）, minute_of_day}`、`other_device_attentive`。
`expect` 是 `{"decision": "drop"|"quiet"|"badge_only"}`、`{"decision": "candidate", "priority": bool}`
或 `{"decision": "delay", "delay_ms": n, "then": "quiet"|"normal"|"priority"}`。

### gate.json

`{"format": 1, "scenarios": [{"name", "note", "config", "events", "expect"}]}`。`config` 按字段名覆盖
`GateConfig`（阶梯写成毫秒数组）。一个场景就是一台设备。

事件：

- `{"at_ms", "op": "admit", "conversation", "kind": "direct"|"group", "tier": "quiet"|"normal"|"priority"}`
- `{"op": "admit_series", "conversation", "kind", "tier", "start_ms", "every_ms", "count"}`：在 `start_ms + i * every_ms` 各进闸门一次，共 `count` 次。
- `{"at_ms", "op": "read", "conversation"}`
- `{"at_ms", "op": "active"}`

回放规则：

1. 展开 series，再把全部事件按 `at_ms` 稳定排序（同一时刻保持文件顺序）。
2. 槽位按会话名区分，初始为 `default()`；会话标签是 `ConversationTag::of(会话名的 UTF-8 字节)`。
3. 到期队列每个会话至多一项，登记即覆盖。处理时刻为 `t` 的事件前，反复取到期时刻最小的一项（同一时刻按会话名字节序），只要它 `<= t` 就在它的到期时刻调用 `on_due(due, ...)` 并记日志；`Defer` 改登记，`Tail` 与 `Nothing` 移除。
4. `admit` 返回 `Defer` 时登记其 `due_ms`。`read` 的 `cancel_tail` 为真时移除该会话的登记。`active` 清空整个队列。
5. 最后一个事件之后，用同样的方式不设上限地清空队列。

日志条目；有 `expect.log` 时逐字段比较：

- admit：`{"at_ms", "op": "admit", "conversation", "slot": "loud"|"quiet"|"defer"|"count_only", "count"}`，`defer` 另带 `"due_ms"`，摘要响铃时另带 `"digest": {"conversations", "messages"}`。
- due：`{"at_ms", "op": "due", "conversation", "due": "tail"|"defer"|"nothing"}`，`tail` 另带 `"count"`，`defer` 另带 `"due_ms"`。
- read：`{"at_ms", "op": "read", "conversation", "clear", "cancel_tail"}`。
- active：`{"at_ms", "op": "active"}`。

大场景给汇总而不是完整日志：

- `totals`：`admit`、各 `slot` 结果（`loud`、`quiet`、`defer`、`count_only`）、`digest`、`tail`、`due_defer`、`due_nothing`、`read`、`active` 的次数；`vendor_calls` = loud + quiet + tail + digest；`rings` = loud + digest。
- `ring_times_ms`：响铃的 admit 与摘要响铃的时刻，按日志顺序。
- `last_count`：每个会话最后一次 loud、quiet 或 tail 的 `count`。
- `digests`：每次摘要响铃的 `{"at_ms", "conversations", "messages"}`。
- `max_rings_in_window`：`{"window_ms", "max"}`，任意这么长的半开窗口里最多的响铃次数。

## 许可

Apache-2.0，见 [LICENSE](LICENSE)。
