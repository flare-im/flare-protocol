//! 提示决策：只看消息等级、偏好、当地时间与多端状态，不看频率。
//!
//! 频率与总量交给闸门（[`crate::Gate`]）。把两者分开，是因为决策的输入在推送前就齐了
//! （偏好、时区、设备状态），而闸门要读写按设备保存的状态；服务端可以先决策，
//! 只有需要进闸门的才去碰状态存储。

use crate::prefs::{AlertClass, ConversationPrefs, Mentioned, MultiDevicePolicy, UserPrefs};
use crate::time::MinuteOfWeek;

/// 其他设备活跃且在看时，这台设备的推送默认推迟的时长。
///
/// 推迟期间会话在别的设备上被读就取消：在电脑上看了就不再打扰手机，没看则手机 15 秒后响。
pub const MULTI_DEVICE_DELAY_MS: i64 = 15_000;

/// 进闸门的响铃档位。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    /// 只静默送达：通知出现或更新，从不响铃（勿扰时段、通知类消息）。
    Quiet,
    /// 普通候选：受槽位响铃阶梯、设备令牌桶、摘要模式与设备最小间隔约束。
    Normal,
    /// 优先候选（点名、紧急）：不受槽位响铃阶梯、令牌桶与摘要模式限制，只受设备最小间隔约束。
    Priority,
}

/// [`decide`] 的输入。默认值是一条普通的、别人发来的、没有点名的消息，所有偏好取放行值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DecideInput {
    /// 消息对这个接收者的提示等级。
    pub class: AlertClass,
    /// 是否点名到这个接收者。
    pub mentioned: Mentioned,
    /// 消息是否由接收者本人（任一设备）发出。
    pub from_self: bool,
    /// 接收者对这个会话的偏好；查询失败时用默认值放行。
    pub conversation: ConversationPrefs,
    /// 接收者的用户级设置；查询失败时用默认值放行。
    pub user: UserPrefs,
    /// 这台设备的当地时间；不知道时区时为 `None`，勿扰时段不生效。
    pub local_time: Option<MinuteOfWeek>,
    /// 同一用户是否有别的设备处于活跃且在看（`Active` 加 `ATTENTIVE`）。
    pub other_device_attentive: bool,
}

/// 决策结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decision {
    /// 不提示。角标是否计入由角标规则另算，与这里无关。
    Drop,
    /// 静默送达：通知出现或更新，不响铃不亮屏。仍然进闸门（档位 [`Tier::Quiet`]），
    /// 这样勿扰时段里刷屏的会话也只有一条通知、更新有间隔。
    Quiet,
    /// 候选响铃，交给闸门按预算决定响铃、静默更新还是只计数。
    Candidate {
        /// 点名或紧急：绕过槽位响铃阶梯、令牌桶与摘要模式，只受设备最小间隔约束。
        priority: bool,
    },
    /// 推迟 `delay_ms` 再进闸门；到期前会话被读则取消。到期时按 `then` 档位进闸门。
    Delay {
        /// 推迟时长，毫秒。
        delay_ms: i64,
        /// 到期后进闸门的档位。
        then: Tier,
    },
    /// 只更新角标，不出通知。
    BadgeOnly,
}

impl Decision {
    /// 由闸门档位得到「立即进闸门」的决策。
    pub const fn from_tier(tier: Tier) -> Self {
        match tier {
            Tier::Quiet => Self::Quiet,
            Tier::Normal => Self::Candidate { priority: false },
            Tier::Priority => Self::Candidate { priority: true },
        }
    }

    /// 需要进闸门时的档位（推迟的取到期后的档位）；不提示与只更新角标为 `None`。
    pub const fn tier(self) -> Option<Tier> {
        match self {
            Self::Quiet => Some(Tier::Quiet),
            Self::Candidate { priority: false } => Some(Tier::Normal),
            Self::Candidate { priority: true } => Some(Tier::Priority),
            Self::Delay { then, .. } => Some(then),
            Self::Drop | Self::BadgeOnly => None,
        }
    }

    /// 去掉推迟，立即进闸门。状态存储不可用、无处登记到期时用：宁可早提示，不丢提示。
    pub const fn without_delay(self) -> Self {
        match self {
            Self::Delay { then, .. } => Self::from_tier(then),
            other => other,
        }
    }
}

/// 按决策顺序给出处置。
///
/// 1. 等级 `none`，或本人发出：不提示。
/// 2. 会话免打扰：`urgent` 作为优先候选，其余不提示。
/// 3. 会话仅 @我 且没点名：不提示。解析不出是否点名按点名处理；`urgent` 不受这一条限制，
///    否则「仅 @我」反而比免打扰更严。
/// 4. 勿扰时段：`urgent`、以及设置放行时的点名，作为优先候选；其余静默送达。
/// 5. 别的设备活跃且在看（`urgent` 除外）：按多端策略推迟、立即或只更新角标；
///    点名跟随策略，推迟到期后仍按点名进闸门。
/// 6. `silent`：静默送达。
/// 7. 点名与 `urgent` 为优先候选，其余为普通候选。
///
/// 点名的认定：等级为 `mention`，或等级为 `normal` 而 `mentioned` 为 `Yes`。
/// `Unknown` 只用于第 3 条的放行，不升级为点名：解析失败时让消息照常进闸门，
/// 而不是让整群的消息都绕过预算。
pub fn decide(input: &DecideInput) -> Decision {
    if input.from_self || input.class == AlertClass::None {
        return Decision::Drop;
    }
    let urgent = input.class == AlertClass::Urgent;
    let mention = input.class == AlertClass::Mention
        || (input.class == AlertClass::Normal && input.mentioned == Mentioned::Yes);

    let ConversationPrefs {
        muted,
        mention_only,
    } = input.conversation;
    if muted {
        return if urgent {
            Decision::Candidate { priority: true }
        } else {
            Decision::Drop
        };
    }
    if mention_only && !urgent && !mention && input.mentioned != Mentioned::Unknown {
        return Decision::Drop;
    }

    if let Some(quiet) = input.user.active_quiet_hours(input.local_time) {
        return if urgent || (mention && quiet.allow_mentions) {
            Decision::Candidate { priority: true }
        } else {
            Decision::Quiet
        };
    }

    let tier = if urgent || mention {
        Tier::Priority
    } else if input.class == AlertClass::Silent {
        Tier::Quiet
    } else {
        Tier::Normal
    };

    if input.other_device_attentive && !urgent {
        match input.user.multi_device {
            MultiDevicePolicy::Delay => {
                return Decision::Delay {
                    delay_ms: MULTI_DEVICE_DELAY_MS,
                    then: tier,
                };
            }
            MultiDevicePolicy::BadgeOnly => return Decision::BadgeOnly,
            MultiDevicePolicy::Immediate => {}
        }
    }

    Decision::from_tier(tier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::QuietHours;

    const NIGHT: QuietHours = QuietHours {
        start_minute: 22 * 60,
        end_minute: 7 * 60,
        weekdays: QuietHours::EVERY_DAY,
        allow_mentions: true,
    };

    fn input(class: AlertClass) -> DecideInput {
        DecideInput {
            class,
            ..DecideInput::default()
        }
    }

    fn at_night() -> Option<MinuteOfWeek> {
        MinuteOfWeek::new(2, 23 * 60)
    }

    fn at_noon() -> Option<MinuteOfWeek> {
        MinuteOfWeek::new(2, 12 * 60)
    }

    const NORMAL: Decision = Decision::Candidate { priority: false };
    const PRIORITY: Decision = Decision::Candidate { priority: true };

    #[test]
    fn none_class_and_own_messages_drop() {
        assert_eq!(decide(&input(AlertClass::None)), Decision::Drop);
        let own = DecideInput {
            from_self: true,
            ..input(AlertClass::Urgent)
        };
        assert_eq!(decide(&own), Decision::Drop);
    }

    #[test]
    fn plain_classes_map_to_tiers() {
        assert_eq!(decide(&input(AlertClass::Silent)), Decision::Quiet);
        assert_eq!(decide(&input(AlertClass::Normal)), NORMAL);
        assert_eq!(decide(&input(AlertClass::Mention)), PRIORITY);
        assert_eq!(decide(&input(AlertClass::Urgent)), PRIORITY);
        let named = DecideInput {
            mentioned: Mentioned::Yes,
            ..input(AlertClass::Normal)
        };
        assert_eq!(decide(&named), PRIORITY);
        // 解析不出不升级为点名。
        let unknown = DecideInput {
            mentioned: Mentioned::Unknown,
            ..input(AlertClass::Normal)
        };
        assert_eq!(decide(&unknown), NORMAL);
        // 通知类消息被点名也仍然静默。
        let silent_named = DecideInput {
            mentioned: Mentioned::Yes,
            ..input(AlertClass::Silent)
        };
        assert_eq!(decide(&silent_named), Decision::Quiet);
    }

    #[test]
    fn muted_lets_only_urgent_through() {
        let muted = ConversationPrefs {
            muted: true,
            mention_only: false,
        };
        for class in [AlertClass::Silent, AlertClass::Normal, AlertClass::Mention] {
            let case = DecideInput {
                conversation: muted,
                ..input(class)
            };
            assert_eq!(decide(&case), Decision::Drop, "{class:?}");
        }
        let urgent = DecideInput {
            conversation: muted,
            other_device_attentive: true,
            local_time: at_night(),
            user: UserPrefs {
                quiet_hours: Some(NIGHT),
                multi_device: MultiDevicePolicy::BadgeOnly,
                ..UserPrefs::default()
            },
            ..input(AlertClass::Urgent)
        };
        assert_eq!(decide(&urgent), PRIORITY);
    }

    #[test]
    fn mention_only_drops_unnamed_messages_but_keeps_unknown_and_urgent() {
        let prefs = ConversationPrefs {
            muted: false,
            mention_only: true,
        };
        let case = |class, mentioned| DecideInput {
            conversation: prefs,
            mentioned,
            ..input(class)
        };
        assert_eq!(
            decide(&case(AlertClass::Normal, Mentioned::No)),
            Decision::Drop
        );
        assert_eq!(
            decide(&case(AlertClass::Silent, Mentioned::No)),
            Decision::Drop
        );
        assert_eq!(
            decide(&case(AlertClass::Normal, Mentioned::Unknown)),
            NORMAL
        );
        assert_eq!(
            decide(&case(AlertClass::Silent, Mentioned::Unknown)),
            Decision::Quiet
        );
        assert_eq!(decide(&case(AlertClass::Normal, Mentioned::Yes)), PRIORITY);
        assert_eq!(decide(&case(AlertClass::Mention, Mentioned::No)), PRIORITY);
        assert_eq!(decide(&case(AlertClass::Urgent, Mentioned::No)), PRIORITY);
    }

    #[test]
    fn quiet_hours_deliver_quietly_except_urgent_and_allowed_mentions() {
        let night = |class, allow_mentions| DecideInput {
            user: UserPrefs {
                quiet_hours: Some(QuietHours {
                    allow_mentions,
                    ..NIGHT
                }),
                ..UserPrefs::default()
            },
            local_time: at_night(),
            ..input(class)
        };
        assert_eq!(decide(&night(AlertClass::Normal, true)), Decision::Quiet);
        assert_eq!(decide(&night(AlertClass::Silent, true)), Decision::Quiet);
        assert_eq!(decide(&night(AlertClass::Mention, true)), PRIORITY);
        assert_eq!(decide(&night(AlertClass::Mention, false)), Decision::Quiet);
        assert_eq!(decide(&night(AlertClass::Urgent, false)), PRIORITY);
        // 勿扰时段先于多端策略：时段内不推迟，直接静默或放行。
        let attentive = DecideInput {
            other_device_attentive: true,
            ..night(AlertClass::Normal, true)
        };
        assert_eq!(decide(&attentive), Decision::Quiet);
        // 时段外照常。
        let noon = DecideInput {
            local_time: at_noon(),
            ..night(AlertClass::Normal, true)
        };
        assert_eq!(decide(&noon), NORMAL);
        // 不知道当地时间：不判勿扰。
        let unknown_zone = DecideInput {
            local_time: None,
            ..night(AlertClass::Normal, true)
        };
        assert_eq!(decide(&unknown_zone), NORMAL);
    }

    #[test]
    fn other_device_attentive_follows_the_policy() {
        let attentive = |class, policy| DecideInput {
            other_device_attentive: true,
            user: UserPrefs {
                multi_device: policy,
                ..UserPrefs::default()
            },
            ..input(class)
        };
        let delayed = |then| Decision::Delay {
            delay_ms: MULTI_DEVICE_DELAY_MS,
            then,
        };
        assert_eq!(
            decide(&attentive(AlertClass::Normal, MultiDevicePolicy::Delay)),
            delayed(Tier::Normal)
        );
        assert_eq!(
            decide(&attentive(AlertClass::Mention, MultiDevicePolicy::Delay)),
            delayed(Tier::Priority)
        );
        assert_eq!(
            decide(&attentive(AlertClass::Silent, MultiDevicePolicy::Delay)),
            delayed(Tier::Quiet)
        );
        assert_eq!(
            decide(&attentive(AlertClass::Urgent, MultiDevicePolicy::Delay)),
            PRIORITY
        );
        assert_eq!(
            decide(&attentive(AlertClass::Normal, MultiDevicePolicy::Immediate)),
            NORMAL
        );
        assert_eq!(
            decide(&attentive(
                AlertClass::Mention,
                MultiDevicePolicy::Immediate
            )),
            PRIORITY
        );
        assert_eq!(
            decide(&attentive(AlertClass::Normal, MultiDevicePolicy::BadgeOnly)),
            Decision::BadgeOnly
        );
        assert_eq!(
            decide(&attentive(
                AlertClass::Mention,
                MultiDevicePolicy::BadgeOnly
            )),
            Decision::BadgeOnly
        );
        assert_eq!(
            decide(&attentive(AlertClass::Urgent, MultiDevicePolicy::BadgeOnly)),
            PRIORITY
        );
        assert_eq!(MULTI_DEVICE_DELAY_MS, 15_000);
    }

    #[test]
    fn decision_helpers() {
        assert_eq!(Decision::Drop.tier(), None);
        assert_eq!(Decision::BadgeOnly.tier(), None);
        assert_eq!(Decision::Quiet.tier(), Some(Tier::Quiet));
        assert_eq!(NORMAL.tier(), Some(Tier::Normal));
        assert_eq!(PRIORITY.tier(), Some(Tier::Priority));
        let delay = Decision::Delay {
            delay_ms: 15_000,
            then: Tier::Priority,
        };
        assert_eq!(delay.tier(), Some(Tier::Priority));
        assert_eq!(delay.without_delay(), PRIORITY);
        assert_eq!(Decision::Drop.without_delay(), Decision::Drop);
        for tier in [Tier::Quiet, Tier::Normal, Tier::Priority] {
            assert_eq!(Decision::from_tier(tier).tier(), Some(tier));
        }
    }
}
