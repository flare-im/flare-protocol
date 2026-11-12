# flare-notify-rules

[中文](README.zh-CN.md)

The single implementation of Flare IM's notification rules: for one message and one device, does the
notification ring, appear silently, get coalesced into the conversation's existing notification, or
wait. The push server runs it for devices whose process is not running; the client engine runs it
in-process. Both get the same answer for the same input.

The crate is pure: no IO, no clock reads (time is always the caller's `now_ms`, Unix milliseconds),
no randomness, no async, no protocol dependency. The rules never allocate; only the two formatting
helpers of the `text` module use `alloc`. It is `no_std` and builds for `wasm32-unknown-unknown`.
Feature `serde` adds serde derives to the persisted state.

## Two steps

```rust
use flare_notify_rules::*;

// 1. Preferences: drop, quiet, candidate, delay or badge only.
let decision = decide(&DecideInput {
    class: AlertClass::Normal,
    mentioned: Mentioned::No,
    from_self: false,
    conversation: ConversationPrefs::default(),
    user: UserPrefs::default(),
    local_time: Some(MinuteOfWeek::from_utc_ms(now_ms, utc_offset_minutes)),
    other_device_attentive: false,
});

// 2. Frequency and volume: the gate, with state the caller stores.
if let Some(tier) = decision.tier() {
    let input = GateInput { kind: ConversationKind::Group, tier, tag: ConversationTag::of(id) };
    let admitted = Gate::default().admit(now_ms, input, &mut slot, &mut device);
    // admitted.slot: Loud | Quiet | Defer { due_ms } | CountOnly
    // admitted.digest: Some(DigestAlert) when the digest notification rings as well
}
```

### API

| Item | Purpose |
|---|---|
| `decide(&DecideInput) -> Decision` | Preference rules (below). `Decision::{Drop, Quiet, Candidate { priority }, Delay { delay_ms, then }, BadgeOnly}`; `tier()` gives the gate tier, `without_delay()` drops a delay when there is nowhere to schedule it. |
| `Tier::{Quiet, Normal, Priority}` | Gate input. Quiet never rings; Normal is budgeted; Priority (mentions, urgent) bypasses the slot ladder, bucket and digest and only respects the device minimum gap. |
| `Gate::admit(now_ms, GateInput, &mut SlotState, &mut DeviceState) -> Admitted` | One message into the gate. |
| `Gate::on_due(now_ms, &mut SlotState, &mut DeviceState) -> DueAction` | A scheduled tail refresh is due: `Tail { count }`, `Defer { due_ms }` (reschedule) or `Nothing`. |
| `Gate::on_read(now_ms, &mut SlotState, &DeviceState) -> ReadOutcome` | The conversation was read anywhere: closes the slot; `clear` says whether this device needs a clear correction, `cancel_tail` whether a scheduled tail can be removed. |
| `Gate::on_active(&mut DeviceState)` | The device is `Active` again: all its slots become stale, digest mode ends. |
| `fail_open(Tier) -> SlotAction` | State unreadable or not writable: treat the message as a conversation's first (rings unless the tier is Quiet). |
| `GateConfig` / `GateConfig::validate` | Parameters (below); validate tenant overrides before use. |
| `SlotState`, `DeviceState` | Persisted state, `encode() -> [u8; N]`, `decode(&[u8])`, public `version`. |
| `MinuteOfWeek` | Device local time (Monday 00:00 = 0) from `from_utc_ms(utc_ms, offset_minutes)` or `from_local_ms`. |
| `PreviewMode::strictest` | Combine the user's preview with a tenant's forced `Hidden`. |
| `text::*` | Notification strings shared by the push worker and the engine (below). |

## Notification text

`flare_notify_rules::text` holds the fallback strings once, so a notification reads the same whether
the push worker rendered it or the engine did.

```rust
pub enum Lang { Zh, En }                                    // Default: Zh
impl Lang { pub fn from_locale(tag: &str) -> Lang }
pub enum Phrase { GenericTitle, GenericBody, NoticeBody, SentMessage, Recalled, MentionMarker,
                  Image, Video, Voice, File, Location, Contact, Link, Sticker, Emoji, RichText, Message }
pub const fn phrase(lang: Lang, phrase: Phrase) -> &'static str;
pub fn count_prefix(lang: Lang, count: u32) -> Option<String>;
pub fn digest_text(lang: Lang, conversations: u32, messages: u32) -> String;
pub fn mentions_already(text: &str) -> bool;
```

- `from_locale`: the primary subtag (split on `-`, `_`, `.`, `@`, case-insensitive) `zh` gives `Zh`, any other non-empty tag `En`, an empty or blank tag `Zh`. So `zh-Hant-TW` and `zh_CN.UTF-8` are Chinese.
- `phrase`: fixed strings. The placeholders (`Image` .. `Message`) are bracketed, matching the design kit's message summaries: `[图片]` / `[Image]`; `Message` covers types without their own placeholder and blank text.
- `count_prefix`: `None` below 2; otherwise `[N条] ` / `[N messages] ` with a trailing space, `N` capped at `99+`.
- `digest_text`: `N 个会话有 M 条新消息` / `M new messages in N chats`; with fewer than 2 conversations only the message count (`M 条新消息` / `M new messages`, `1 new message`). `M` is at least the number of conversations and at least 1.
- `mentions_already`: the text already says the receiver was mentioned (`有人@你`, `有人@我`, `[Mentioned you]`, `[Mentioned]`), so the mention marker is not added twice.

## Decision order

1. Class `none`, or the message is the receiver's own: `Drop`.
2. Conversation muted: `urgent` is a priority candidate, everything else `Drop`.
3. Conversation mention-only and not mentioned: `Drop`. `Mentioned::Unknown` counts as mentioned here. `urgent` is not dropped (otherwise mention-only would be stricter than mute).
4. In quiet hours: `urgent` and, when allowed, mentions are priority candidates; everything else `Quiet`. Without local time (no time zone registered) quiet hours do not apply.
5. Another device of the user is active and attentive (not for `urgent`): policy `Delay` gives `Delay { 15000 ms, then }`, `Immediate` continues, `BadgeOnly` gives `BadgeOnly`. Mentions follow the policy.
6. Class `silent`: `Quiet`.
7. Mentions and `urgent`: `Candidate { priority: true }`; the rest `Candidate { priority: false }`.

A mention is class `mention`, or class `normal` with `Mentioned::Yes`. Quiet hours: `start == end` is all day; `end < start` crosses midnight and the part after midnight belongs to the day the window started; start inclusive, end exclusive; `weekdays` bit0 Monday .. bit6 Sunday, 0 every day. Preference lookups that fail use the defaults, which are the fail-open values.

## Gate

One slot per (device, conversation), one notification per slot. One device state per device.

- **First message of an idle slot** is always sent: loud if the device budget allows, otherwise quiet. Only the vendor hard cap can hold it, and then it is the tail at the end of the cap window.
- **Updates** after the first replace the same notification with the latest message and the count. Two adaptive ladders per slot: the update ladder (minimum interval between any two sends for the slot) and the loud ladder (minimum interval between rings of the slot). A ring is also an update, so the update interval restarts from it. A message inside the update interval is absorbed: the first absorption returns `Defer { due_ms }` and the caller schedules a tail refresh; later ones return `CountOnly`. Tail refreshes are always quiet.
- **Ladders step** up by one level at the next send (update ladder) or ring (loud ladder) when the interval just ended absorbed or suppressed a message, and step down one level for every full interval of the current level without messages.
- **Device budget**: loud token bucket; device minimum gap between any two rings (all tiers, the digest included); vendor hard cap per fixed window that starts at the first call after the previous window ended.
- **Digest mode**: entered when the number of conversations with a push within the enter window reaches the threshold; exited when, not counting the current one, fewer than 2 pushes happened within the exit window, or on `on_active`. In digest mode Normal messages never ring on their slot; a separate digest notification rings when one digest interval has passed since both the start of the digest period and the last ring of any kind. `DigestAlert { conversations, messages }` counts the current period (since entering or the previous digest ring).
- **Expiry is lazy**: a slot idle for the expiry time, or opened before the device's last `on_active`, is treated as absent at the next touch. Absent state and `default()` mean the same thing.

| `GateConfig` field | Default |
|---|---|
| `quiet_ladder` | 2 s, 5 s, 15 s, 60 s, 300 s |
| `loud_ladder_direct` | 10 s, 30 s, 120 s, 300 s |
| `loud_ladder_group` | 30 s, 120 s, 600 s |
| `slot_idle_expiry_ms` | 30 min |
| `loud_bucket_capacity` / `loud_bucket_refill_ms` | 3 / 15 s |
| `device_min_loud_gap_ms` | 1 s |
| `digest_enter_conversations` / `digest_enter_window_ms` | 6 / 60 s |
| `digest_exit_window_ms` | 120 s (fewer than 2 pushes) |
| `digest_ladder` | 60 s, 120 s, 300 s |
| `vendor_cap` / `vendor_cap_window_ms` | 30 / 60 s |

Design examples, as replayed by `tests/vectors/gate.json`:

- One group, 10 000 messages over an hour: rings at 0, 30, 150, 750 s and then every 600 s (8), 15 quiet tail refreshes, 23 vendor calls, one notification whose last update shows 10 000.
- Twenty conversations starting within one second: digest mode from the sixth; rings at 0, 60, 180 and 480 s, never two within 60 s.

## Persisted state

Every operation that changes a state increments its `version`. The caller keeps the version it read
and writes back only if the stored value still has it (compare-and-set); on repeated conflicts it
uses `fail_open`. Encoding is fixed-size little-endian; byte 0 is the layout (1) and bytes 1..9 are
`version`, so a script can compare those 8 bytes without decoding. A value that fails to decode is
treated as absent. Timestamps use `i64::MIN` for "never". A store TTL of at least
`slot_idle_expiry_ms` on slots gives the same result as lazy expiry.

`SlotState`: 58 bytes.

| Offset | Type | Field |
|---|---|---|
| 0 | u8 | layout = 1 |
| 1 | u64 | version |
| 9 | u32 | device generation when opened |
| 13 | i64 | last message |
| 21 | i64 | last send (any vendor call for the slot) |
| 29 | i64 | last ring |
| 37 | i64 | scheduled tail |
| 45 | u32 | count since opened |
| 49 | u32 | pending (absorbed, not yet shown) |
| 53 | u8 | update ladder level |
| 54 | u8 | loud ladder level |
| 55 | u8 | flags: bit0 loud suppressed, bit1 update suppressed |
| 56 | u16 | digest period mark |

`DeviceState`: 169 bytes.

| Offset | Type | Field |
|---|---|---|
| 0 | u8 | layout = 1 |
| 1 | u64 | version |
| 9 | u32 | generation |
| 13 | i64 | last ring |
| 21 | u8 | bucket debt (tokens used, not refilled) |
| 22 | i64 | bucket refill clock |
| 30 | i64 | vendor window start |
| 38 | u8 | vendor calls in window |
| 39 | i64 | last push |
| 47 | i64 | push before last |
| 55 | u8 | flags: bit0 digest on, bit1 digest suppressed |
| 56 | u8 | digest ladder level |
| 57 | u16 | digest period (never 0) |
| 59 | i64 | digest period start |
| 67 | u16 | conversations this period |
| 69 | u32 | messages this period |
| 73 | 8 × (u32, i64) | recent conversations: tag, last push |

## Vectors

`tests/vectors/decide.json` and `tests/vectors/gate.json` are replayed by `tests/vectors.rs`. The
push server and the engine replay the same files through their own wiring (state store, due
queue) and must produce the same results.

Inputs (cases, scenarios, config, events) are written by hand; expected outputs are generated by
this crate. After changing the rules, or after adding a case or scenario, run

```sh
FLARE_NOTIFY_RULES_WRITE_VECTORS=1 cargo test --test vectors
```

and review the diff. Only each case's and scenario's `expect` is rewritten; the layout (key order,
short objects on one line) is kept, so an unchanged rule set leaves the files byte for byte the
same. A new scenario declares which form it wants with a placeholder: `"expect": {"log": []}` for
the full log, `"expect": {}` for the summary. The acceptance bounds of the two design examples are
asserted in both modes. Without the variable (and in CI) the test only asserts.

### decide.json

`{"format": 1, "cases": [{"name", "input", "expect"}]}`. `input` has every field:
`class` (`none|silent|normal|mention|urgent`), `mentioned` (`no|yes|unknown`), `from_self`,
`conversation {muted, mention_only}`, `user {quiet_hours: null | {start_minute, end_minute,
weekdays, allow_mentions}, preview: full|sender_only|hidden, multi_device:
delay|immediate|badge_only}`, `local_time: null | {weekday (0 = Monday), minute_of_day}`,
`other_device_attentive`. `expect` is `{"decision": "drop"|"quiet"|"badge_only"}`,
`{"decision": "candidate", "priority": bool}` or
`{"decision": "delay", "delay_ms": n, "then": "quiet"|"normal"|"priority"}`.

### gate.json

`{"format": 1, "scenarios": [{"name", "note", "config", "events", "expect"}]}`. `config` overrides
`GateConfig` fields by name (ladders as arrays of milliseconds). One scenario is one device.

Events:

- `{"at_ms", "op": "admit", "conversation", "kind": "direct"|"group", "tier": "quiet"|"normal"|"priority"}`
- `{"op": "admit_series", "conversation", "kind", "tier", "start_ms", "every_ms", "count"}`: `count` admits at `start_ms + i * every_ms`.
- `{"at_ms", "op": "read", "conversation"}`
- `{"at_ms", "op": "active"}`

Replay rules:

1. Expand series, then stable-sort all events by `at_ms` (equal times keep file order).
2. Slots are keyed by conversation name, created as `default()`; the conversation tag is `ConversationTag::of(name as UTF-8)`.
3. The due queue holds at most one entry per conversation; scheduling replaces it. Before each event at `t`, repeatedly take the entry with the smallest due time (ties by conversation name, byte order) while it is `<= t`, call `on_due(due, ...)` at that due time and log it; `Defer` reschedules, `Tail` and `Nothing` remove.
4. `admit` returning `Defer` schedules its `due_ms`. `read` with `cancel_tail` removes the conversation's entry. `active` clears the whole queue.
5. After the last event, drain the queue the same way without a limit.

Log entries, compared field by field when `expect.log` is present:

- admit: `{"at_ms", "op": "admit", "conversation", "slot": "loud"|"quiet"|"defer"|"count_only", "count"}`, plus `"due_ms"` for `defer` and `"digest": {"conversations", "messages"}` when a digest rings.
- due: `{"at_ms", "op": "due", "conversation", "due": "tail"|"defer"|"nothing"}`, plus `"count"` for `tail` and `"due_ms"` for `defer`.
- read: `{"at_ms", "op": "read", "conversation", "clear", "cancel_tail"}`.
- active: `{"at_ms", "op": "active"}`.

Large scenarios give a summary instead of the log:

- `totals`: counts of `admit`, each `slot` kind (`loud`, `quiet`, `defer`, `count_only`), `digest`, `tail`, `due_defer`, `due_nothing`, `read`, `active`; `vendor_calls` = loud + quiet + tail + digest; `rings` = loud + digest.
- `ring_times_ms`: times of loud admits and digest alerts, in log order.
- `last_count`: per conversation, the `count` of its last loud, quiet or tail.
- `digests`: `{"at_ms", "conversations", "messages"}` per digest alert.
- `max_rings_in_window`: `{"window_ms", "max"}`, the most rings in any half-open window of that length.

## License

Apache-2.0, see [LICENSE](LICENSE).
