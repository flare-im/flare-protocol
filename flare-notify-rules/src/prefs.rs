//! 提示等级与接收者偏好。
//!
//! 字段与协议里的 `ConversationUserSettings`、`UserNotificationSettings`、`QuietHours`
//! 一一对应，但这里不依赖协议 crate：规则要能单独编进 wasm，也不该因为协议加字段而重编。
//! 引擎与服务端各自从协议类型转换过来。

use crate::time::MinuteOfWeek;

/// 消息对某个接收者的提示等级，由服务端分类器根据内容类型和标志给出。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AlertClass {
    /// 不提示、不计数：已读回执、输入中、表情回应、编辑。
    None,
    /// 更新通知内容和角标，不响铃：`show_badge` 为真而 `play_sound` 为假的通知类消息。
    Silent,
    /// 普通消息：文本、图片、文件。
    #[default]
    Normal,
    /// 点名到这个接收者，或 @所有人。
    Mention,
    /// 绕过免打扰和预算（由发送方配额约束）：加急、来电邀请、安全告警。
    Urgent,
}

/// 消息是否点名到这个接收者。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mentioned {
    /// 没有点名。
    #[default]
    No,
    /// 点名到了（含 @所有人）。
    Yes,
    /// 解析不出。按点名处理，只在「仅 @我」的过滤里起作用：宁可多响一声，也不吞掉可能点名的消息。
    Unknown,
}

/// 会话类型，决定用哪一条响铃阶梯。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConversationKind {
    /// 单聊。
    Direct,
    /// 群聊（含频道等多人会话）。
    Group,
}

/// 接收者对这个会话的偏好。默认值即偏好查询失败时的放行取值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ConversationPrefs {
    /// 会话免打扰：只有 `urgent` 还会提示。
    pub muted: bool,
    /// 仅 @我：没点名的消息不提示。
    pub mention_only: bool,
}

/// 勿扰时段，按设备当地时间计算。
///
/// - `start_minute == end_minute`：全天；
/// - `end_minute < start_minute`：跨午夜，零点以后的部分算作前一天的时段，
///   所以「周五 22:00 到 07:00」在周六早上仍然生效，即使周六没被勾选；
/// - 区间含起点不含终点。
///
/// 不提供 `Default`：全零恰好是「每天全天勿扰」，不能当作缺省值。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QuietHours {
    /// 起始，当天的分钟（0..1440）。
    pub start_minute: u16,
    /// 结束，当天的分钟（0..1440），不含。
    pub end_minute: u16,
    /// 生效的星期：bit0 周一 … bit6 周日；为 0 表示每天。
    pub weekdays: u8,
    /// 时段内是否放行 @我。
    pub allow_mentions: bool,
}

impl QuietHours {
    /// `weekdays` 取这个值表示每天生效。
    pub const EVERY_DAY: u8 = 0;

    /// 这个当地时刻是否落在勿扰时段里。
    ///
    /// 起止分钟越界视为没有设置，返回 `false`：配置坏了时宁可照常提示，与偏好查询失败时放行一致。
    pub fn contains(&self, at: MinuteOfWeek) -> bool {
        let (start, end) = (self.start_minute, self.end_minute);
        if start >= MinuteOfWeek::MINUTES_PER_DAY || end >= MinuteOfWeek::MINUTES_PER_DAY {
            return false;
        }
        let day = at.weekday();
        let minute = at.minute_of_day();
        if start == end {
            return self.applies_on(day);
        }
        if start < end {
            return (start..end).contains(&minute) && self.applies_on(day);
        }
        if minute >= start {
            self.applies_on(day)
        } else if minute < end {
            self.applies_on((day + 6) % 7)
        } else {
            false
        }
    }

    /// 时段是否在这一天（0 = 周一）开始生效。高于 bit6 的位忽略。
    fn applies_on(&self, weekday: u8) -> bool {
        let days = self.weekdays & 0x7f;
        days == Self::EVERY_DAY || (days & (1 << weekday)) != 0
    }
}

/// 通知预览。
///
/// 变体按「遮蔽程度」从低到高排列，[`PreviewMode::strictest`] 依赖这个顺序。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PreviewMode {
    /// 显示发送者与内容。
    #[default]
    Full,
    /// 只显示发送者。
    SenderOnly,
    /// 隐藏发送者与内容，使用通用文案。
    Hidden,
}

impl PreviewMode {
    /// 取两者中更严格的一个：用户设置与租户强制隐藏同时存在时，以更严格者为准。
    pub fn strictest(self, other: Self) -> Self {
        self.max(other)
    }
}

/// 同一用户的其他设备活跃且在看时，这台设备的推送怎么处理。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MultiDevicePolicy {
    /// 推迟 [`crate::MULTI_DEVICE_DELAY_MS`]，期间在别的设备上读了就取消。
    #[default]
    Delay,
    /// 与单设备相同，立即处理。
    Immediate,
    /// 只更新角标。
    BadgeOnly,
}

/// 用户级通知设置。默认值即设置查询失败时的放行取值：无勿扰时段、完整预览、多端延迟。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct UserPrefs {
    /// 勿扰时段；`None` 表示没有设置。
    pub quiet_hours: Option<QuietHours>,
    /// 通知预览。决策不看它，渲染文案时由调用方使用。
    pub preview: PreviewMode,
    /// 多端策略。
    pub multi_device: MultiDevicePolicy,
}

impl UserPrefs {
    /// 当前生效的勿扰时段。
    ///
    /// 不知道设备当地时间（时区未登记）时返回 `None`：无法判断就照常提示。
    pub fn active_quiet_hours(&self, local_time: Option<MinuteOfWeek>) -> Option<QuietHours> {
        let quiet = self.quiet_hours?;
        let at = local_time?;
        quiet.contains(at).then_some(quiet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(weekday: u8, hour: u16, minute: u16) -> MinuteOfWeek {
        MinuteOfWeek::new(weekday, hour * 60 + minute).unwrap()
    }

    fn window(start: (u16, u16), end: (u16, u16), weekdays: u8) -> QuietHours {
        QuietHours {
            start_minute: start.0 * 60 + start.1,
            end_minute: end.0 * 60 + end.1,
            weekdays,
            allow_mentions: false,
        }
    }

    #[test]
    fn same_day_window_includes_start_and_excludes_end() {
        let lunch = window((12, 0), (13, 30), QuietHours::EVERY_DAY);
        assert!(!lunch.contains(at(0, 11, 59)));
        assert!(lunch.contains(at(0, 12, 0)));
        assert!(lunch.contains(at(4, 13, 29)));
        assert!(!lunch.contains(at(4, 13, 30)));
    }

    #[test]
    fn overnight_window_belongs_to_the_day_it_starts() {
        // 只在工作日（周一到周五）的夜里勿扰。
        let weeknights = window((22, 0), (7, 0), 0b0001_1111);
        // 周五 23:00 在时段内；周六 06:59 仍属于周五开始的时段。
        assert!(weeknights.contains(at(4, 23, 0)));
        assert!(weeknights.contains(at(5, 6, 59)));
        assert!(!weeknights.contains(at(5, 7, 0)));
        // 周六 23:00 不在（周六没勾选）；周日 06:00 不在（属于周六开始的时段）。
        assert!(!weeknights.contains(at(5, 23, 0)));
        assert!(!weeknights.contains(at(6, 6, 0)));
        // 周一 06:00 属于周日开始的时段，周日没勾选，所以不在。
        assert!(!weeknights.contains(at(0, 6, 0)));
        // 白天不在。
        assert!(!weeknights.contains(at(2, 12, 0)));
    }

    #[test]
    fn equal_start_and_end_means_all_day_on_selected_days() {
        let sunday = window((0, 0), (0, 0), 0b0100_0000);
        assert!(sunday.contains(at(6, 0, 0)));
        assert!(sunday.contains(at(6, 23, 59)));
        assert!(!sunday.contains(at(0, 0, 0)));
        let always = window((9, 0), (9, 0), QuietHours::EVERY_DAY);
        assert!(always.contains(at(3, 15, 0)));
    }

    #[test]
    fn bits_above_sunday_are_ignored() {
        // 只有 bit7：有效位为零，按每天处理。
        let every_day = window((1, 0), (2, 0), 0b1000_0000);
        assert!(every_day.contains(at(2, 1, 30)));
        // bit7 加周一：只在周一。
        let monday = window((1, 0), (2, 0), 0b1000_0001);
        assert!(monday.contains(at(0, 1, 30)));
        assert!(!monday.contains(at(1, 1, 30)));
    }

    #[test]
    fn out_of_range_minutes_disable_the_window() {
        assert!(!window((24, 0), (7, 0), 0).contains(at(0, 3, 0)));
        assert!(!window((22, 0), (24, 1), 0).contains(at(0, 23, 0)));
    }

    #[test]
    fn active_quiet_hours_needs_both_settings_and_local_time() {
        let prefs = UserPrefs {
            quiet_hours: Some(window((22, 0), (7, 0), 0)),
            ..UserPrefs::default()
        };
        assert!(prefs.active_quiet_hours(Some(at(1, 23, 0))).is_some());
        assert!(prefs.active_quiet_hours(Some(at(1, 12, 0))).is_none());
        assert!(prefs.active_quiet_hours(None).is_none());
        assert!(
            UserPrefs::default()
                .active_quiet_hours(Some(at(1, 23, 0)))
                .is_none()
        );
    }

    #[test]
    fn strictest_preview_wins() {
        use PreviewMode::*;
        assert_eq!(Full.strictest(Hidden), Hidden);
        assert_eq!(SenderOnly.strictest(Full), SenderOnly);
        assert_eq!(Hidden.strictest(SenderOnly), Hidden);
        assert_eq!(Full.strictest(Full), Full);
    }

    #[test]
    fn defaults_are_the_fail_open_values() {
        let user = UserPrefs::default();
        assert_eq!(user.quiet_hours, None);
        assert_eq!(user.preview, PreviewMode::Full);
        assert_eq!(user.multi_device, MultiDevicePolicy::Delay);
        let conversation = ConversationPrefs::default();
        assert!(!conversation.muted && !conversation.mention_only);
        assert_eq!(AlertClass::default(), AlertClass::Normal);
        assert_eq!(Mentioned::default(), Mentioned::No);
    }
}
