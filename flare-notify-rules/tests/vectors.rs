//! 回放 `tests/vectors/` 下的向量；设置 `FLARE_NOTIFY_RULES_WRITE_VECTORS=1` 时改为回写。
//!
//! 服务端闸门与引擎提示中心用同样的回放规则（见 README「Vectors」一节）核对各自的接线：
//! 这里核对的是规则本身，那边核对的是状态读写、到期队列与修正指令有没有接错。
//!
//! 向量的输入（场景、事件、决策输入）是手写的；期望输出由这里的实现生成。规则改了之后
//! 带上环境变量跑一次，只有各场景的 `expect` 被重写，再看 diff 判断变化是不是想要的。
//! 回写的排版与手写部分一致（键序保留，短对象一行），没有变化时文件逐字不变。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use flare_notify_rules::{
    Admitted, AlertClass, ConversationKind, ConversationPrefs, ConversationTag, DecideInput,
    Decision, DeviceState, DueAction, Gate, GateConfig, GateInput, Ladder, Mentioned, MinuteOfWeek,
    MultiDevicePolicy, PreviewMode, QuietHours, ReadOutcome, SlotAction, SlotState, Tier,
    UserPrefs, decide,
};
use serde_json::{Map, Value, json};

/// 回写开关。只认 `1`，避免随手设的值误触发重写。
const WRITE_ENV: &str = "FLARE_NOTIFY_RULES_WRITE_VECTORS";

/// 汇总里没写窗口时用的窗口长度。
const DEFAULT_RING_WINDOW_MS: i64 = 60_000;

fn write_mode() -> bool {
    std::env::var(WRITE_ENV).is_ok_and(|value| value == "1")
}

fn vector_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("vectors")
        .join(name)
}

fn load(name: &str) -> Value {
    let path = vector_path(name);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

/// 先写临时文件再改名：同时运行的其他测试也在读这个文件，不能让它们读到写了一半的内容。
fn save(name: &str, value: &Value) {
    let path = vector_path(name);
    let staging = path.with_extension("json.tmp");
    fs::write(&staging, render(value, 0) + "\n")
        .unwrap_or_else(|error| panic!("write {}: {error}", staging.display()));
    fs::rename(&staging, &path)
        .unwrap_or_else(|error| panic!("replace {}: {error}", path.display()));
}

fn is_scalar(value: &Value) -> bool {
    !value.is_array() && !value.is_object()
}

/// 一行写完的形式：`{"k": v, "k2": v2}`、`[a, b]`。
fn inline(value: &Value) -> String {
    match value {
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(inline).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(map) => {
            let fields: Vec<String> = map
                .iter()
                .map(|(key, value)| format!("{}: {}", Value::from(key.as_str()), inline(value)))
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        scalar => scalar.to_string(),
    }
}

/// 向量文件的排版：标量数组一行；值都是标量（或只含标量的对象）且一行不到 160 字节的对象一行；
/// 其余逐项换行、两格缩进。事件与日志因此一条一行，diff 好读。字符串按 UTF-8 原样写出，
/// 向量文件只用 ASCII，与手写时的转义规则没有分歧。
fn render(value: &Value, depth: usize) -> String {
    let pad = "  ".repeat(depth);
    match value {
        Value::Array(items) if items.iter().all(is_scalar) => inline(value),
        Value::Array(items) => {
            let lines: Vec<String> = items
                .iter()
                .map(|item| format!("{pad}  {}", render(item, depth + 1)))
                .collect();
            format!("[\n{}\n{pad}]", lines.join(",\n"))
        }
        Value::Object(map) => {
            let flat = map.values().all(|field| {
                is_scalar(field) || field.as_object().is_some_and(|m| m.values().all(is_scalar))
            });
            let line = inline(value);
            if flat && line.len() < 160 {
                return line;
            }
            let lines: Vec<String> = map
                .iter()
                .map(|(key, field)| {
                    format!(
                        "{pad}  {}: {}",
                        Value::from(key.as_str()),
                        render(field, depth + 1)
                    )
                })
                .collect();
            format!("{{\n{}\n{pad}}}", lines.join(",\n"))
        }
        scalar => scalar.to_string(),
    }
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("missing string `{key}` in {value}"))
}

fn i64_of(value: &Value, key: &str) -> i64 {
    value[key]
        .as_i64()
        .unwrap_or_else(|| panic!("missing integer `{key}` in {value}"))
}

fn bool_of(value: &Value, key: &str) -> bool {
    value[key]
        .as_bool()
        .unwrap_or_else(|| panic!("missing bool `{key}` in {value}"))
}

fn small<T: TryFrom<i64>>(value: &Value, key: &str) -> T {
    T::try_from(i64_of(value, key)).unwrap_or_else(|_| panic!("`{key}` out of range in {value}"))
}

fn parse_tier(name: &str) -> Tier {
    match name {
        "quiet" => Tier::Quiet,
        "normal" => Tier::Normal,
        "priority" => Tier::Priority,
        other => panic!("unknown tier `{other}`"),
    }
}

fn tier_name(tier: Tier) -> &'static str {
    match tier {
        Tier::Quiet => "quiet",
        Tier::Normal => "normal",
        Tier::Priority => "priority",
    }
}

fn parse_kind(name: &str) -> ConversationKind {
    match name {
        "direct" => ConversationKind::Direct,
        "group" => ConversationKind::Group,
        other => panic!("unknown conversation kind `{other}`"),
    }
}

// ------------------------------------------------------------------------------------ decide

fn decide_input(value: &Value) -> DecideInput {
    let class = match str_of(value, "class") {
        "none" => AlertClass::None,
        "silent" => AlertClass::Silent,
        "normal" => AlertClass::Normal,
        "mention" => AlertClass::Mention,
        "urgent" => AlertClass::Urgent,
        other => panic!("unknown alert class `{other}`"),
    };
    let mentioned = match str_of(value, "mentioned") {
        "no" => Mentioned::No,
        "yes" => Mentioned::Yes,
        "unknown" => Mentioned::Unknown,
        other => panic!("unknown mention state `{other}`"),
    };
    let conversation = &value["conversation"];
    let user = &value["user"];
    let quiet_hours = match &user["quiet_hours"] {
        Value::Null => None,
        quiet => Some(QuietHours {
            start_minute: small(quiet, "start_minute"),
            end_minute: small(quiet, "end_minute"),
            weekdays: small(quiet, "weekdays"),
            allow_mentions: bool_of(quiet, "allow_mentions"),
        }),
    };
    let preview = match str_of(user, "preview") {
        "full" => PreviewMode::Full,
        "sender_only" => PreviewMode::SenderOnly,
        "hidden" => PreviewMode::Hidden,
        other => panic!("unknown preview `{other}`"),
    };
    let multi_device = match str_of(user, "multi_device") {
        "delay" => MultiDevicePolicy::Delay,
        "immediate" => MultiDevicePolicy::Immediate,
        "badge_only" => MultiDevicePolicy::BadgeOnly,
        other => panic!("unknown multi-device policy `{other}`"),
    };
    let local_time = match &value["local_time"] {
        Value::Null => None,
        local => Some(
            MinuteOfWeek::new(small(local, "weekday"), small(local, "minute_of_day"))
                .unwrap_or_else(|| panic!("invalid local time {local}")),
        ),
    };
    DecideInput {
        class,
        mentioned,
        from_self: bool_of(value, "from_self"),
        conversation: ConversationPrefs {
            muted: bool_of(conversation, "muted"),
            mention_only: bool_of(conversation, "mention_only"),
        },
        user: UserPrefs {
            quiet_hours,
            preview,
            multi_device,
        },
        local_time,
        other_device_attentive: bool_of(value, "other_device_attentive"),
    }
}

fn decision_json(decision: Decision) -> Value {
    match decision {
        Decision::Drop => json!({"decision": "drop"}),
        Decision::Quiet => json!({"decision": "quiet"}),
        Decision::Candidate { priority } => json!({"decision": "candidate", "priority": priority}),
        Decision::Delay { delay_ms, then } => {
            json!({"decision": "delay", "delay_ms": delay_ms, "then": tier_name(then)})
        }
        Decision::BadgeOnly => json!({"decision": "badge_only"}),
    }
}

#[test]
fn decide_vectors() {
    let write = write_mode();
    let mut vectors = load("decide.json");
    assert_eq!(vectors["format"], 1);
    let cases = vectors["cases"].as_array_mut().expect("cases");
    assert!(!cases.is_empty());
    for case in cases.iter_mut() {
        let got = decision_json(decide(&decide_input(&case["input"])));
        if write {
            case["expect"] = got;
        } else {
            assert_eq!(
                got,
                case["expect"],
                "decide case `{}`",
                str_of(case, "name")
            );
        }
    }
    if write {
        save("decide.json", &vectors);
    }
}

// -------------------------------------------------------------------------------------- gate

fn ladder(key: &str, value: &Value) -> Ladder {
    let steps: Vec<u32> = value
        .as_array()
        .unwrap_or_else(|| panic!("config `{key}` must be an array"))
        .iter()
        .map(|step| {
            step.as_u64()
                .and_then(|step| u32::try_from(step).ok())
                .unwrap_or_else(|| panic!("config `{key}` has a bad step {step}"))
        })
        .collect();
    Ladder::new(&steps).unwrap_or_else(|| panic!("config `{key}` is not a valid ladder"))
}

fn gate_config(value: &Value) -> GateConfig {
    let mut config = GateConfig::DEFAULT;
    let Some(overrides) = value.as_object() else {
        return config;
    };
    for (key, value) in overrides {
        let int = || {
            value
                .as_i64()
                .unwrap_or_else(|| panic!("config `{key}` must be an integer"))
        };
        let byte = || u8::try_from(int()).unwrap_or_else(|_| panic!("config `{key}` exceeds u8"));
        match key.as_str() {
            "quiet_ladder" => config.quiet_ladder = ladder(key, value),
            "loud_ladder_direct" => config.loud_ladder_direct = ladder(key, value),
            "loud_ladder_group" => config.loud_ladder_group = ladder(key, value),
            "digest_ladder" => config.digest_ladder = ladder(key, value),
            "slot_idle_expiry_ms" => config.slot_idle_expiry_ms = int(),
            "loud_bucket_capacity" => config.loud_bucket_capacity = byte(),
            "loud_bucket_refill_ms" => config.loud_bucket_refill_ms = int(),
            "device_min_loud_gap_ms" => config.device_min_loud_gap_ms = int(),
            "digest_enter_conversations" => config.digest_enter_conversations = byte(),
            "digest_enter_window_ms" => config.digest_enter_window_ms = int(),
            "digest_exit_window_ms" => config.digest_exit_window_ms = int(),
            "vendor_cap" => config.vendor_cap = byte(),
            "vendor_cap_window_ms" => config.vendor_cap_window_ms = int(),
            other => panic!("unknown config key `{other}`"),
        }
    }
    config.validate().expect("vector config validates");
    config
}

/// 展开 `admit_series`，再按 `at_ms` 稳定排序：同一时刻按文件中的先后。
fn expand(events: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for event in events {
        if str_of(event, "op") == "admit_series" {
            let start = i64_of(event, "start_ms");
            let every = i64_of(event, "every_ms");
            for i in 0..i64_of(event, "count") {
                out.push(json!({
                    "at_ms": start + i * every,
                    "op": "admit",
                    "conversation": event["conversation"],
                    "kind": event["kind"],
                    "tier": event["tier"],
                }));
            }
        } else {
            out.push(event.clone());
        }
    }
    out.sort_by_key(|event| i64_of(event, "at_ms"));
    out
}

/// 一台设备的回放：槽位按会话名存，到期队列每个会话至多一项（登记即覆盖）。
struct Replay {
    gate: Gate,
    device: DeviceState,
    slots: BTreeMap<String, SlotState>,
    queue: BTreeMap<String, i64>,
    log: Vec<Value>,
}

impl Replay {
    /// 按到期时刻、同一时刻按会话名的顺序处理不晚于 `limit` 的登记，在到期时刻调用。
    fn run_dues(&mut self, limit: Option<i64>) {
        // BTreeMap 按会话名遍历，min_by_key 取第一个最小值，于是同一时刻按会话名排序。
        while let Some((conversation, due)) = self
            .queue
            .iter()
            .min_by_key(|(_, due)| **due)
            .map(|(conversation, due)| (conversation.clone(), *due))
        {
            if limit.is_some_and(|limit| due > limit) {
                break;
            }
            self.queue.remove(&conversation);
            let slot = self.slots.entry(conversation.clone()).or_default();
            let action = self.gate.on_due(due, slot, &mut self.device);
            let mut entry = json!({"at_ms": due, "op": "due", "conversation": conversation});
            match action {
                DueAction::Tail { count } => {
                    entry["due"] = json!("tail");
                    entry["count"] = json!(count);
                }
                DueAction::Defer { due_ms } => {
                    entry["due"] = json!("defer");
                    entry["due_ms"] = json!(due_ms);
                    self.queue.insert(conversation, due_ms);
                }
                DueAction::Nothing => entry["due"] = json!("nothing"),
            }
            self.log.push(entry);
        }
    }

    fn apply(&mut self, event: &Value) {
        let at = i64_of(event, "at_ms");
        self.run_dues(Some(at));
        match str_of(event, "op") {
            "admit" => {
                let conversation = str_of(event, "conversation").to_owned();
                let input = GateInput {
                    kind: parse_kind(str_of(event, "kind")),
                    tier: parse_tier(str_of(event, "tier")),
                    tag: ConversationTag::of(conversation.as_bytes()),
                };
                let slot = self.slots.entry(conversation.clone()).or_default();
                let admitted = self.gate.admit(at, input, slot, &mut self.device);
                let entry = admitted_entry(at, &conversation, admitted);
                if let SlotAction::Defer { due_ms, .. } = admitted.slot {
                    self.queue.insert(conversation, due_ms);
                }
                self.log.push(entry);
            }
            "read" => {
                let conversation = str_of(event, "conversation").to_owned();
                let slot = self.slots.entry(conversation.clone()).or_default();
                let ReadOutcome { clear, cancel_tail } = self.gate.on_read(at, slot, &self.device);
                if cancel_tail {
                    self.queue.remove(&conversation);
                }
                self.log.push(json!({
                    "at_ms": at,
                    "op": "read",
                    "conversation": conversation,
                    "clear": clear,
                    "cancel_tail": cancel_tail,
                }));
            }
            "active" => {
                self.gate.on_active(&mut self.device);
                self.queue.clear();
                self.log.push(json!({"at_ms": at, "op": "active"}));
            }
            other => panic!("unknown event op `{other}`"),
        }
    }
}

/// admit 的日志条目，键序：`at_ms`、`op`、`conversation`、`slot`、[`due_ms`]、`count`、[`digest`]。
fn admitted_entry(at: i64, conversation: &str, admitted: Admitted) -> Value {
    let (slot, due_ms, count) = match admitted.slot {
        SlotAction::Loud { count } => ("loud", None, count),
        SlotAction::Quiet { count } => ("quiet", None, count),
        SlotAction::Defer { due_ms, count } => ("defer", Some(due_ms), count),
        SlotAction::CountOnly { count } => ("count_only", None, count),
    };
    let mut entry = json!({"at_ms": at, "op": "admit", "conversation": conversation, "slot": slot});
    if let Some(due_ms) = due_ms {
        entry["due_ms"] = json!(due_ms);
    }
    entry["count"] = json!(count);
    if let Some(digest) = admitted.digest {
        entry["digest"] = json!({
            "conversations": digest.conversations,
            "messages": digest.messages,
        });
    }
    entry
}

fn replay(scenario: &Value) -> Vec<Value> {
    let mut run = Replay {
        gate: Gate::new(gate_config(&scenario["config"])),
        device: DeviceState::default(),
        slots: BTreeMap::new(),
        queue: BTreeMap::new(),
        log: Vec::new(),
    };
    let events = scenario["events"].as_array().expect("events");
    for event in expand(events) {
        run.apply(&event);
    }
    run.run_dues(None);
    run.log
}

/// 回放日志的汇总，口径与 README 一致。
struct Summary {
    totals: BTreeMap<&'static str, u64>,
    ring_times: Vec<i64>,
    last_count: BTreeMap<String, Value>,
    digests: Vec<Value>,
}

/// 汇总计数的键，也是写进文件时的顺序。
const TOTAL_KEYS: [&str; 13] = [
    "admit",
    "loud",
    "quiet",
    "defer",
    "count_only",
    "digest",
    "tail",
    "due_defer",
    "due_nothing",
    "read",
    "active",
    "vendor_calls",
    "rings",
];

fn bump(totals: &mut BTreeMap<&'static str, u64>, key: &str) {
    *totals
        .get_mut(key)
        .unwrap_or_else(|| panic!("unknown total `{key}`")) += 1;
}

fn summarize(log: &[Value]) -> Summary {
    let mut summary = Summary {
        totals: TOTAL_KEYS.into_iter().map(|key| (key, 0)).collect(),
        ring_times: Vec::new(),
        last_count: BTreeMap::new(),
        digests: Vec::new(),
    };
    for entry in log {
        let at = i64_of(entry, "at_ms");
        match str_of(entry, "op") {
            "admit" => {
                bump(&mut summary.totals, "admit");
                let slot = str_of(entry, "slot");
                bump(&mut summary.totals, slot);
                if slot == "loud" || slot == "quiet" {
                    bump(&mut summary.totals, "vendor_calls");
                    summary.last_count.insert(
                        str_of(entry, "conversation").to_owned(),
                        entry["count"].clone(),
                    );
                }
                if slot == "loud" {
                    bump(&mut summary.totals, "rings");
                    summary.ring_times.push(at);
                }
                if let Some(digest) = entry.get("digest") {
                    bump(&mut summary.totals, "digest");
                    bump(&mut summary.totals, "vendor_calls");
                    bump(&mut summary.totals, "rings");
                    summary.ring_times.push(at);
                    summary.digests.push(json!({
                        "at_ms": at,
                        "conversations": digest["conversations"],
                        "messages": digest["messages"],
                    }));
                }
            }
            "due" => match str_of(entry, "due") {
                "tail" => {
                    bump(&mut summary.totals, "tail");
                    bump(&mut summary.totals, "vendor_calls");
                    summary.last_count.insert(
                        str_of(entry, "conversation").to_owned(),
                        entry["count"].clone(),
                    );
                }
                "defer" => bump(&mut summary.totals, "due_defer"),
                _ => bump(&mut summary.totals, "due_nothing"),
            },
            "read" => bump(&mut summary.totals, "read"),
            "active" => bump(&mut summary.totals, "active"),
            other => panic!("unknown log op `{other}`"),
        }
    }
    summary
}

/// 任意长 `window_ms` 的半开窗口里最多有几次响铃。
fn max_in_window(times: &[i64], window_ms: i64) -> u64 {
    let mut best = 0;
    let mut start = 0;
    for (end, &at) in times.iter().enumerate() {
        while at - times[start] >= window_ms {
            start += 1;
        }
        best = best.max(end - start + 1);
    }
    best as u64
}

/// 大场景的期望：汇总各项，按写进文件的顺序。
fn summary_json(summary: &Summary, window_ms: i64) -> Value {
    let totals: Map<String, Value> = TOTAL_KEYS
        .into_iter()
        .map(|key| (key.to_owned(), json!(summary.totals[key])))
        .collect();
    json!({
        "totals": totals,
        "ring_times_ms": summary.ring_times,
        "last_count": summary.last_count,
        "max_rings_in_window": {
            "window_ms": window_ms,
            "max": max_in_window(&summary.ring_times, window_ms),
        },
        "digests": summary.digests,
    })
}

/// 一个场景的期望。原期望写的是完整日志就生成日志，否则生成汇总（新场景写 `{"log": []}`
/// 或 `{}` 占位，回写一次即可）。
fn expected_for(scenario: &Value, log: Vec<Value>) -> Value {
    let expect = &scenario["expect"];
    if expect.get("log").is_some() {
        return json!({ "log": log });
    }
    let window_ms = expect["max_rings_in_window"]["window_ms"]
        .as_i64()
        .unwrap_or(DEFAULT_RING_WINDOW_MS);
    summary_json(&summarize(&log), window_ms)
}

fn scenario<'a>(vectors: &'a Value, name: &str) -> &'a Value {
    vectors["scenarios"]
        .as_array()
        .expect("scenarios")
        .iter()
        .find(|candidate| candidate["name"] == name)
        .unwrap_or_else(|| panic!("scenario `{name}` is missing"))
}

#[test]
fn gate_vectors() {
    let write = write_mode();
    let mut vectors = load("gate.json");
    assert_eq!(vectors["format"], 1);
    let scenarios = vectors["scenarios"].as_array_mut().expect("scenarios");
    assert!(!scenarios.is_empty());
    let mut names = BTreeSet::new();
    for scenario in scenarios.iter_mut() {
        let name = str_of(scenario, "name").to_owned();
        assert!(names.insert(name.clone()), "duplicate scenario `{name}`");
        let got = expected_for(scenario, replay(scenario));
        if write {
            scenario["expect"] = got;
            continue;
        }
        let want = scenario["expect"].as_object().expect("expect");
        let got = got.as_object().expect("generated expect");
        for (key, value) in got {
            match (value, want.get(key)) {
                (Value::Array(got), Some(Value::Array(want))) => {
                    for (i, (got, want)) in got.iter().zip(want).enumerate() {
                        assert_eq!(got, want, "scenario `{name}` {key}[{i}]");
                    }
                    assert_eq!(got.len(), want.len(), "scenario `{name}` {key} length");
                }
                (value, want) => assert_eq!(Some(value), want, "scenario `{name}` {key}"),
            }
        }
        let extra: Vec<&String> = want.keys().filter(|key| !got.contains_key(*key)).collect();
        assert!(
            extra.is_empty(),
            "scenario `{name}` expects unknown keys {extra:?}"
        );
    }
    if write {
        save("gate.json", &vectors);
    }
}

/// 设计里的两个例子单独按验收口径再核一遍，不只是与生成的数字相等；回写模式下同样执行，
/// 规则改坏了验收口径时回写不会悄悄通过。
#[test]
fn design_examples_meet_the_acceptance_bounds() {
    let vectors = load("gate.json");

    let flood = summarize(&replay(scenario(
        &vectors,
        "one_group_ten_thousand_messages_in_an_hour",
    )));
    assert_eq!(flood.totals["admit"], 10_000);
    assert_eq!(flood.totals["rings"], 8);
    assert!(flood.totals["vendor_calls"] <= 30);
    assert_eq!(flood.last_count.len(), 1, "one notification only");
    assert_eq!(flood.last_count["g"], 10_000);
    assert_eq!(
        flood.ring_times[..4]
            .iter()
            .map(|at| at / 1_000)
            .collect::<Vec<_>>(),
        [0, 30, 150, 750]
    );

    let storm = summarize(&replay(scenario(&vectors, "twenty_conversations_at_once")));
    assert!(storm.totals["digest"] >= 1, "digest mode is entered");
    assert!(max_in_window(&storm.ring_times, 60_000) <= 1);
    assert_eq!(storm.last_count.len(), 20);
}
