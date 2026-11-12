//! 闸门参数。
//!
//! 默认值就是设计给出的阶梯与上限。租户可以在 [`GateConfig::validate`] 允许的范围内调整；
//! 用户不能调（不暴露「提示频率」档位）。

use core::fmt;

use crate::ladder::Ladder;
use crate::prefs::ConversationKind;
use crate::state::RECENT_CAPACITY;

/// 闸门参数。时长均为毫秒。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GateConfig {
    /// 会话通知静默更新的最小间隔阶梯。默认 2s、5s、15s、60s、300s。
    pub quiet_ladder: Ladder,
    /// 单聊的响铃最小间隔阶梯。默认 10s、30s、120s、300s。
    pub loud_ladder_direct: Ladder,
    /// 群聊的响铃最小间隔阶梯。默认 30s、120s、600s。
    pub loud_ladder_group: Ladder,
    /// 槽位空闲多久过期（下一条消息重新算作「第一条」）。默认 30 分钟。
    pub slot_idle_expiry_ms: i64,
    /// 设备响铃令牌桶容量。默认 3。
    pub loud_bucket_capacity: u8,
    /// 令牌桶每补一个令牌的时长。默认 15s。
    pub loud_bucket_refill_ms: i64,
    /// 设备上任意两次响铃的最小间隔（含点名、紧急与摘要）。默认 1s。
    pub device_min_loud_gap_ms: i64,
    /// 进入摘要模式的会话数：窗口内有这么多（或更多）个会话产生推送。默认 6。
    pub digest_enter_conversations: u8,
    /// 进入摘要模式的统计窗口。默认 60s。
    pub digest_enter_window_ms: i64,
    /// 退出摘要模式的窗口：窗口内推送少于 2 次即退出。默认 120s。
    pub digest_exit_window_ms: i64,
    /// 摘要通知的响铃间隔阶梯。默认 60s、120s、300s。
    pub digest_ladder: Ladder,
    /// 每台设备每个窗口最多的厂商调用次数（硬上限）。默认 30。
    pub vendor_cap: u8,
    /// 厂商调用上限的窗口。默认 60s。
    pub vendor_cap_window_ms: i64,
}

impl GateConfig {
    /// 设计给出的默认参数。
    pub const DEFAULT: Self = Self {
        quiet_ladder: Ladder::must(&[2_000, 5_000, 15_000, 60_000, 300_000]),
        loud_ladder_direct: Ladder::must(&[10_000, 30_000, 120_000, 300_000]),
        loud_ladder_group: Ladder::must(&[30_000, 120_000, 600_000]),
        slot_idle_expiry_ms: 30 * 60_000,
        loud_bucket_capacity: 3,
        loud_bucket_refill_ms: 15_000,
        device_min_loud_gap_ms: 1_000,
        digest_enter_conversations: 6,
        digest_enter_window_ms: 60_000,
        digest_exit_window_ms: 120_000,
        digest_ladder: Ladder::must(&[60_000, 120_000, 300_000]),
        vendor_cap: 30,
        vendor_cap_window_ms: 60_000,
    };

    /// 这类会话的响铃阶梯。
    pub const fn loud_ladder(&self, kind: ConversationKind) -> &Ladder {
        match kind {
            ConversationKind::Direct => &self.loud_ladder_direct,
            ConversationKind::Group => &self.loud_ladder_group,
        }
    }

    /// 检查参数是否在规则能保证语义的范围内。租户覆盖值加载后先过这一关，不合法就用默认值。
    ///
    /// - 槽位过期必须长于静默更新的最长间隔，否则尾部刷新还没发出槽位就过期了，
    ///   计数会悄悄归零；
    /// - 进入摘要的会话数在 2 到设备里最近会话表的容量之间，超过容量就永远数不到；
    /// - 其余数量与时长都必须为正，0 等于关掉限频或永远不放行。
    pub fn validate(&self) -> Result<(), ConfigError> {
        let quiet_top = self.quiet_ladder.at(self.quiet_ladder.top());
        let checks: [(bool, &'static str); 9] = [
            (self.slot_idle_expiry_ms > quiet_top, "slot_idle_expiry_ms"),
            (self.loud_bucket_capacity >= 1, "loud_bucket_capacity"),
            (self.loud_bucket_refill_ms > 0, "loud_bucket_refill_ms"),
            (self.device_min_loud_gap_ms >= 0, "device_min_loud_gap_ms"),
            (
                self.digest_enter_conversations >= 2
                    && usize::from(self.digest_enter_conversations) <= RECENT_CAPACITY,
                "digest_enter_conversations",
            ),
            (self.digest_enter_window_ms > 0, "digest_enter_window_ms"),
            (self.digest_exit_window_ms > 0, "digest_exit_window_ms"),
            (self.vendor_cap >= 1, "vendor_cap"),
            (self.vendor_cap_window_ms > 0, "vendor_cap_window_ms"),
        ];
        match checks.iter().find(|(ok, _)| !ok) {
            Some(&(_, field)) => Err(ConfigError { field }),
            None => Ok(()),
        }
    }
}

impl Default for GateConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 参数超出允许范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConfigError {
    /// 不合法的字段名，与 [`GateConfig`] 的字段同名。
    pub field: &'static str,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gate config field `{}` is out of range", self.field)
    }
}

impl core::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_design_and_validate() {
        let config = GateConfig::default();
        assert_eq!(config, GateConfig::DEFAULT);
        assert_eq!(config.validate(), Ok(()));
        assert_eq!(
            config.quiet_ladder.steps_ms(),
            &[2_000, 5_000, 15_000, 60_000, 300_000]
        );
        assert_eq!(
            config.loud_ladder(ConversationKind::Direct).steps_ms(),
            &[10_000, 30_000, 120_000, 300_000]
        );
        assert_eq!(
            config.loud_ladder(ConversationKind::Group).steps_ms(),
            &[30_000, 120_000, 600_000]
        );
        assert_eq!(config.digest_ladder.steps_ms(), &[60_000, 120_000, 300_000]);
        assert_eq!(config.slot_idle_expiry_ms, 1_800_000);
        assert_eq!(config.loud_bucket_capacity, 3);
        assert_eq!(config.loud_bucket_refill_ms, 15_000);
        assert_eq!(config.device_min_loud_gap_ms, 1_000);
        assert_eq!(config.digest_enter_conversations, 6);
        assert_eq!(config.digest_enter_window_ms, 60_000);
        assert_eq!(config.digest_exit_window_ms, 120_000);
        assert_eq!(config.vendor_cap, 30);
        assert_eq!(config.vendor_cap_window_ms, 60_000);
    }

    #[test]
    fn validate_names_the_first_bad_field() {
        let bad = |edit: fn(&mut GateConfig)| {
            let mut config = GateConfig::DEFAULT;
            edit(&mut config);
            config.validate().unwrap_err().field
        };
        assert_eq!(
            bad(|c| c.slot_idle_expiry_ms = 300_000),
            "slot_idle_expiry_ms"
        );
        assert_eq!(bad(|c| c.loud_bucket_capacity = 0), "loud_bucket_capacity");
        assert_eq!(
            bad(|c| c.loud_bucket_refill_ms = 0),
            "loud_bucket_refill_ms"
        );
        assert_eq!(
            bad(|c| c.device_min_loud_gap_ms = -1),
            "device_min_loud_gap_ms"
        );
        assert_eq!(
            bad(|c| c.digest_enter_conversations = 1),
            "digest_enter_conversations"
        );
        assert_eq!(
            bad(|c| c.digest_enter_conversations = 9),
            "digest_enter_conversations"
        );
        assert_eq!(
            bad(|c| c.digest_enter_window_ms = 0),
            "digest_enter_window_ms"
        );
        assert_eq!(
            bad(|c| c.digest_exit_window_ms = 0),
            "digest_exit_window_ms"
        );
        assert_eq!(bad(|c| c.vendor_cap = 0), "vendor_cap");
        assert_eq!(bad(|c| c.vendor_cap_window_ms = 0), "vendor_cap_window_ms");
    }

    #[test]
    fn zero_gap_and_full_recent_table_are_allowed() {
        let mut config = GateConfig::DEFAULT;
        config.device_min_loud_gap_ms = 0;
        config.digest_enter_conversations = RECENT_CAPACITY as u8;
        assert_eq!(config.validate(), Ok(()));
    }
}
