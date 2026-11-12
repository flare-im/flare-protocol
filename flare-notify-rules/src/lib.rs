//! Flare IM 的提示规则：一条消息对某台设备是响铃、静默送达、合并还是延迟。
//!
//! 推送服务端（设备进程不在时）与客户端引擎（进程在时）共用这一份实现，同一条消息
//! 在两边得到同样的处置。为此这里只做纯计算：
//!
//! - 不读时钟：时刻一律由调用方以 `now_ms`（Unix 毫秒）传入；勿扰时段用的当地时间
//!   也由调用方换算好再传入（[`MinuteOfWeek`]）。
//! - 不做 IO、不起异步、不用随机数，热路径上不分配内存，可以编进 `no_std` 与 wasm。
//! - 状态由调用方保存：[`SlotState`]（每台设备每个会话一份）与 [`DeviceState`]
//!   （每台设备一份），各带一个 `version` 供调用方比较后写入；定长二进制编码见
//!   [`SlotState::encode`]。
//!
//! 两步使用：
//!
//! 1. [`decide`] 按会话与用户偏好给出处置（[`Decision`]）：不提示、静默、候选、延迟或只更新角标。
//! 2. 需要进闸门的交给 [`Gate::admit`]：由会话槽位与设备预算决定立即响铃、静默更新、
//!    推迟到某时刻补一次尾部刷新，还是只累加计数；到期由 [`Gate::on_due`] 补发。
//!    会话被读调用 [`Gate::on_read`]，设备回到前台调用 [`Gate::on_active`]。
//!
//! 通知的兜底文案（通用标题、占位、条数前缀、摘要）在 [`text`] 模块，推送服务端与引擎共用。
//! 规则本身不分配内存；只有 [`text`] 里拼数字的两个函数用到 `alloc`。
//!
//! ```
//! use flare_notify_rules::{
//!     AlertClass, ConversationKind, ConversationTag, DecideInput, DeviceState, Gate, GateInput,
//!     SlotAction, SlotState, decide,
//! };
//!
//! let decision = decide(&DecideInput {
//!     class: AlertClass::Normal,
//!     ..DecideInput::default()
//! });
//! let tier = decision.tier().expect("普通消息进闸门");
//!
//! let gate = Gate::default();
//! let mut slot = SlotState::default();
//! let mut device = DeviceState::default();
//! let input = GateInput {
//!     kind: ConversationKind::Group,
//!     tier,
//!     tag: ConversationTag::of(b"conversation-1"),
//! };
//! let admitted = gate.admit(1_700_000_000_000, input, &mut slot, &mut device);
//! assert_eq!(admitted.slot, SlotAction::Loud { count: 1 });
//! assert_eq!(slot.version, 1);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate alloc;

mod config;
mod decide;
mod gate;
mod ladder;
mod prefs;
mod state;
pub mod text;
mod time;

pub use config::{ConfigError, GateConfig};
pub use decide::{DecideInput, Decision, MULTI_DEVICE_DELAY_MS, Tier, decide};
pub use gate::{
    Admitted, ConversationTag, DigestAlert, DueAction, Gate, GateInput, ReadOutcome, SlotAction,
    fail_open,
};
pub use ladder::Ladder;
pub use prefs::{
    AlertClass, ConversationKind, ConversationPrefs, Mentioned, MultiDevicePolicy, PreviewMode,
    QuietHours, UserPrefs,
};
pub use state::{DEVICE_STATE_LEN, DecodeError, DeviceState, SLOT_STATE_LEN, SlotState};
pub use time::MinuteOfWeek;
