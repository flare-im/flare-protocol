//! 闸门：会话槽位（一个会话一条通知）与设备预算（跨会话的总量）。
//!
//! 每次调用读写两份状态：这个（设备，会话）的 [`SlotState`] 与这台设备的 [`DeviceState`]。
//! 槽位决定这条消息是立即更新通知、响铃，还是先吞并、到间隔结束补一次尾部刷新；
//! 设备决定还有没有响铃预算、要不要进入摘要模式、厂商调用是否到了硬上限。
//!
//! 过期一律惰性处理：槽位空闲过久、设备回到前台换了代，都在下一次碰到这个槽位时才生效。
//! 这样调用方不必扫描或批量删除，状态存储里缺失的值与 `default()` 也就是同一个意思。

use crate::config::GateConfig;
use crate::decide::Tier;
use crate::ladder::Ladder;
use crate::prefs::ConversationKind;
use crate::state::{
    DIGEST_ON, DIGEST_SUPPRESSED, DeviceState, LOUD_SUPPRESSED, QUIET_SUPPRESSED, RECENT_CAPACITY,
    RecentEntry, SlotState,
};
use crate::time::{NEVER, since};

/// 会话标签：设备里「最近产生推送的会话」表用它区分会话，只需在同一台设备上稳定。
///
/// 32 位足够：表里最多 8 项，撞车只会让摘要模式的会话计数少算一个。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ConversationTag(pub u32);

impl ConversationTag {
    /// 由会话标识算出标签（FNV-1a 32 位）。服务端与引擎各自维护设备状态，
    /// 用同一个函数只是为了让测试向量在两边得到同样的标签。
    pub const fn of(conversation_id: &[u8]) -> Self {
        let mut hash: u32 = 0x811c_9dc5;
        let mut i = 0;
        while i < conversation_id.len() {
            hash ^= conversation_id[i] as u32;
            hash = hash.wrapping_mul(0x0100_0193);
            i += 1;
        }
        Self(hash)
    }
}

/// [`Gate::admit`] 的输入。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GateInput {
    /// 会话类型，决定响铃阶梯。
    pub kind: ConversationKind,
    /// 决策给出的档位（[`crate::Decision::tier`]）。
    pub tier: Tier,
    /// 会话标签。
    pub tag: ConversationTag,
}

/// 这条消息对会话通知的处置。`count` 都是槽位打开以来的消息数（通知上的「[N 条]」）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SlotAction {
    /// 立即发出会话通知并响铃。
    Loud {
        /// 槽位打开以来的消息数。
        count: u32,
    },
    /// 立即发出会话通知（新建或替换同一条），不响铃。
    Quiet {
        /// 槽位打开以来的消息数。
        count: u32,
    },
    /// 吞并：通知暂不更新，调用方在 `due_ms` 登记一次尾部刷新，到时调用 [`Gate::on_due`]。
    /// 每个槽位同一时间只有一个登记；登记按槽位覆盖（同一会话只留一项）。
    Defer {
        /// 尾部刷新的时刻。
        due_ms: i64,
        /// 槽位打开以来的消息数。
        count: u32,
    },
    /// 吞并：尾部刷新已经登记过，只累加计数。
    CountOnly {
        /// 槽位打开以来的消息数。
        count: u32,
    },
}

impl SlotAction {
    /// 是否立即调用厂商。
    pub const fn sends(self) -> bool {
        matches!(self, Self::Loud { .. } | Self::Quiet { .. })
    }

    /// 槽位打开以来的消息数。
    pub const fn count(self) -> u32 {
        match self {
            Self::Loud { count }
            | Self::Quiet { count }
            | Self::Defer { count, .. }
            | Self::CountOnly { count } => count,
        }
    }
}

/// 摘要通知：「N 个会话有 M 条新消息」，单独一条通知，负责响铃。
///
/// 计数是本期（进入摘要模式或上一次摘要响铃以来）的数：各会话自己的通知仍然各自更新，
/// 摘要只回答「这段时间又来了多少」。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DigestAlert {
    /// 本期有消息的会话数。
    pub conversations: u16,
    /// 本期的消息数。
    pub messages: u32,
}

/// [`Gate::admit`] 的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Admitted {
    /// 会话通知怎么处理。
    pub slot: SlotAction,
    /// 摘要模式下同时要发的摘要通知（响铃）。只在普通档位、摘要模式下出现，
    /// 此时 `slot` 不会是 [`SlotAction::Loud`]。
    pub digest: Option<DigestAlert>,
}

impl Admitted {
    /// 这次要立即调用厂商的次数（0 到 2）。
    pub const fn vendor_calls(&self) -> u32 {
        self.slot.sends() as u32 + self.digest.is_some() as u32
    }
}

/// [`Gate::on_due`] 的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DueAction {
    /// 补发尾部刷新：静默替换会话通知，显示最新一条与正确计数。
    Tail {
        /// 槽位打开以来的消息数。
        count: u32,
    },
    /// 还不能发（没到时刻，或厂商调用到了上限），改登记到 `due_ms`。
    Defer {
        /// 新的登记时刻。
        due_ms: i64,
    },
    /// 没有要发的：槽位已被读、已过期作废，或者尾部内容已经随一次响铃或更新发出。
    Nothing,
}

/// [`Gate::on_read`] 的结果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ReadOutcome {
    /// 设备上很可能有这个会话的通知，需要发清除修正。
    ///
    /// 只对确实推过、槽位仍然有效的设备为真，修正的成本因此有界；过期或已换代的槽位
    /// 视同不存在，那条通知留到设备打开应用时清。
    pub clear: bool,
    /// 槽位有登记中的尾部刷新，调用方可以把它从到期队列里移除。
    /// 不移除也无妨，到期时 [`Gate::on_due`] 返回 [`DueAction::Nothing`]。
    pub cancel_tail: bool,
}

/// 状态读不到或写不回（存储不可用、比较后写入冲突重试用完）时的处置：
/// 按会话的第一条消息对待，普通与优先档位响铃，静默档位静默送达。
///
/// 这是「宁可多响一声」的放行：存储故障期间每个会话至少提示一次。
/// 进程内的近似限频由调用方用同一个 [`Gate`] 与内存里的状态实现，这里只给单条的兜底。
pub const fn fail_open(tier: Tier) -> SlotAction {
    match tier {
        Tier::Quiet => SlotAction::Quiet { count: 1 },
        Tier::Normal | Tier::Priority => SlotAction::Loud { count: 1 },
    }
}

/// 闸门。只持有参数，本身无状态，可以在线程间共享。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Gate {
    config: GateConfig,
}

impl Gate {
    /// 用给定参数构造。参数应先经 [`GateConfig::validate`] 检查。
    pub const fn new(config: GateConfig) -> Self {
        Self { config }
    }

    /// 参数。
    pub const fn config(&self) -> &GateConfig {
        &self.config
    }

    /// 一条消息进闸门。
    ///
    /// 判定顺序：
    ///
    /// 1. 设备：补令牌；摘要模式下若上一个退出窗口内推送少于 2 次就退出；记下这次推送，
    ///    窗口内产生推送的会话数达到阈值就进入摘要模式。
    /// 2. 槽位：空闲过久或设备已换代则作废重开；按空闲时长回落阶梯；计数加一。
    /// 3. 响铃：
    ///    - 槽位空闲时的第一条一定发出：预算允许就响铃，否则静默；
    ///    - 普通档位要同时满足槽位响铃间隔、令牌桶、设备最小间隔，摘要模式下不在槽位上响铃；
    ///    - 优先档位只看设备最小间隔；
    ///    - 静默档位从不响铃。
    /// 4. 不响铃时走静默更新：间隔已到就立即更新，否则吞并并登记尾部刷新。
    ///    厂商调用到了上限同样吞并，登记到窗口结束。
    /// 5. 摘要模式下的普通档位另外判断摘要响铃。
    ///
    /// 两份状态都会改动，`version` 各加一。
    pub fn admit(
        &self,
        now_ms: i64,
        input: GateInput,
        slot: &mut SlotState,
        device: &mut DeviceState,
    ) -> Admitted {
        let cfg = &self.config;
        let loud_ladder = cfg.loud_ladder(input.kind);

        device.refill_bucket(now_ms, cfg);
        device.settle_digest(now_ms, cfg);
        device.record_push(now_ms, input.tag);
        if !device.digest_active()
            && device.recent_conversations(now_ms, cfg.digest_enter_window_ms)
                >= usize::from(cfg.digest_enter_conversations)
        {
            device.enter_digest(now_ms);
        }

        slot.settle(now_ms, device.generation, cfg, loud_ladder);
        let first = !slot.is_open();
        if first {
            slot.close();
            slot.generation = device.generation;
        }
        slot.last_msg_ms = now_ms;
        slot.count = slot.count.saturating_add(1);
        if device.digest_active() {
            device.note_digest_message(slot);
        }

        let gap_ok = since(now_ms, device.last_ring_ms) >= cfg.device_min_loud_gap_ms;
        let want_loud = match input.tier {
            Tier::Quiet => false,
            Tier::Priority => gap_ok,
            Tier::Normal if device.digest_active() => false,
            Tier::Normal => {
                let ladder_ok =
                    first || since(now_ms, slot.last_loud_ms) >= loud_ladder.at(slot.loud_level);
                if !ladder_ok {
                    slot.flags |= LOUD_SUPPRESSED;
                }
                ladder_ok && gap_ok && device.bucket_available(cfg)
            }
        };

        let action = if want_loud && device.vendor_room(now_ms, cfg) {
            slot.mark_sent(now_ms, &cfg.quiet_ladder);
            slot.mark_loud(now_ms, loud_ladder);
            if input.tier == Tier::Normal {
                device.take_token(now_ms);
            }
            device.last_ring_ms = now_ms;
            device.vendor_take(now_ms, cfg);
            SlotAction::Loud { count: slot.count }
        } else {
            quiet_path(cfg, now_ms, first, slot, device)
        };

        let digest = if input.tier == Tier::Normal && device.digest_active() {
            device.try_digest(now_ms, cfg)
        } else {
            None
        };

        slot.version = slot.version.wrapping_add(1);
        device.version = device.version.wrapping_add(1);
        Admitted {
            slot: action,
            digest,
        }
    }

    /// 尾部刷新到期。调用方在 [`SlotAction::Defer`] 或 [`DueAction::Defer`] 给出的时刻调用。
    ///
    /// 尾部刷新总是静默的：吞并的消息当时没有资格响铃，事后补响只会让人听到一声、
    /// 看到的却是几分钟前的内容。改动了状态时相应的 `version` 加一。
    pub fn on_due(&self, now_ms: i64, slot: &mut SlotState, device: &mut DeviceState) -> DueAction {
        let cfg = &self.config;
        if !slot.is_open() || slot.due_ms == NEVER {
            return DueAction::Nothing;
        }
        if slot.generation != device.generation {
            slot.close();
            slot.version = slot.version.wrapping_add(1);
            return DueAction::Nothing;
        }
        if now_ms < slot.due_ms {
            return DueAction::Defer {
                due_ms: slot.due_ms,
            };
        }
        if slot.pending == 0 {
            slot.due_ms = NEVER;
            slot.version = slot.version.wrapping_add(1);
            return DueAction::Nothing;
        }
        if !device.vendor_room(now_ms, cfg) {
            slot.due_ms = device.vendor_free_at(cfg);
            slot.version = slot.version.wrapping_add(1);
            return DueAction::Defer {
                due_ms: slot.due_ms,
            };
        }
        slot.mark_sent(now_ms, &cfg.quiet_ladder);
        device.vendor_take(now_ms, cfg);
        slot.version = slot.version.wrapping_add(1);
        device.version = device.version.wrapping_add(1);
        DueAction::Tail { count: slot.count }
    }

    /// 会话在任一设备上被读：关闭这台设备上的槽位。
    ///
    /// 下一条消息重新算作第一条。槽位原本打开时 `slot.version` 加一。
    pub fn on_read(&self, now_ms: i64, slot: &mut SlotState, device: &DeviceState) -> ReadOutcome {
        if !slot.is_open() {
            return ReadOutcome::default();
        }
        let live = slot.generation == device.generation
            && since(now_ms, slot.last_msg_ms) < self.config.slot_idle_expiry_ms;
        let outcome = ReadOutcome {
            clear: live && slot.has_sent(),
            cancel_tail: slot.due_ms != NEVER,
        };
        slot.close();
        slot.version = slot.version.wrapping_add(1);
        outcome
    }

    /// 设备回到前台（`Active`）：作废它的全部槽位，退出摘要模式，清空最近会话表。
    ///
    /// 槽位不逐个删除，设备换代后旧槽位在下次碰到时作废；调用方可以顺手清掉这台设备
    /// 的到期登记，不清也只是到期时得到 [`DueAction::Nothing`]。
    /// 令牌桶与厂商调用窗口保留：它们约束的是响铃与调用的频率，与用户是否看过无关。
    pub fn on_active(&self, device: &mut DeviceState) {
        device.generation = device.generation.wrapping_add(1);
        device.exit_digest();
        device.recent = [RecentEntry::EMPTY; RECENT_CAPACITY];
        device.version = device.version.wrapping_add(1);
    }
}

/// 不响铃时的静默更新路径。
fn quiet_path(
    cfg: &GateConfig,
    now_ms: i64,
    first: bool,
    slot: &mut SlotState,
    device: &mut DeviceState,
) -> SlotAction {
    let interval = cfg.quiet_ladder.at(slot.quiet_level);
    let interval_ok = first || since(now_ms, slot.last_send_ms) >= interval;
    if interval_ok && device.vendor_room(now_ms, cfg) {
        slot.mark_sent(now_ms, &cfg.quiet_ladder);
        device.vendor_take(now_ms, cfg);
        return SlotAction::Quiet { count: slot.count };
    }
    slot.pending = slot.pending.saturating_add(1);
    if !interval_ok {
        // 只有「来得比间隔快」才算升档的理由；因为厂商上限被吞并不是会话本身太密。
        slot.flags |= QUIET_SUPPRESSED;
    }
    if slot.due_ms != NEVER {
        return SlotAction::CountOnly { count: slot.count };
    }
    let due_ms = if interval_ok {
        device.vendor_free_at(cfg)
    } else {
        slot.last_send_ms.saturating_add(interval)
    };
    slot.due_ms = due_ms;
    SlotAction::Defer {
        due_ms,
        count: slot.count,
    }
}

impl SlotState {
    /// 惰性过期与阶梯回落，在记入这条消息之前调用。
    fn settle(&mut self, now_ms: i64, generation: u32, cfg: &GateConfig, loud: &Ladder) {
        if !self.is_open() {
            return;
        }
        let idle = since(now_ms, self.last_msg_ms);
        if self.generation != generation || idle >= cfg.slot_idle_expiry_ms {
            self.close();
            return;
        }
        // 安静了一个完整的当前间隔，之前的「来得太快」就不再作数，不能拿来在下次发送时升档。
        if idle >= cfg.quiet_ladder.at(self.quiet_level) {
            self.flags &= !QUIET_SUPPRESSED;
        }
        if idle >= loud.at(self.loud_level) {
            self.flags &= !LOUD_SUPPRESSED;
        }
        self.quiet_level = cfg.quiet_ladder.down_after_idle(self.quiet_level, idle);
        self.loud_level = loud.down_after_idle(self.loud_level, idle);
    }

    /// 为这个槽位调用了一次厂商（任何形式）：通知已是最新，吞并的消息清零，登记取消。
    ///
    /// 响铃也算一次更新：同一条通知被替换，内容与计数都是最新的，
    /// 所以静默更新的间隔从这一刻重新算，不在响铃后紧跟一次多余的更新。
    fn mark_sent(&mut self, now_ms: i64, quiet: &Ladder) {
        if self.flags & QUIET_SUPPRESSED != 0 {
            self.quiet_level = quiet.up(self.quiet_level);
        }
        self.flags &= !QUIET_SUPPRESSED;
        self.pending = 0;
        self.due_ms = NEVER;
        self.last_send_ms = now_ms;
    }

    /// 这个槽位响了一次铃：上一个响铃间隔里有普通消息被压住没响，就升一档。
    ///
    /// 升档放在下一次响铃时而不是被压住的当下：被压住的那一刻升档会让当前这次等待也被拉长，
    /// 「第 0、30、150、750 秒响铃」这样的节奏就变成了「第 0、120 秒」。
    fn mark_loud(&mut self, now_ms: i64, loud: &Ladder) {
        if self.flags & LOUD_SUPPRESSED != 0 {
            self.loud_level = loud.up(self.loud_level);
        }
        self.flags &= !LOUD_SUPPRESSED;
        self.last_loud_ms = now_ms;
    }
}

impl DeviceState {
    /// 按经过的时间补令牌。补满后计时停在满桶，下次取令牌时从那一刻重新计时。
    fn refill_bucket(&mut self, now_ms: i64, cfg: &GateConfig) {
        if self.bucket_debt == 0 {
            return;
        }
        let interval = cfg.loud_bucket_refill_ms.max(1);
        let elapsed = since(now_ms, self.bucket_refill_ms);
        if elapsed < interval {
            return;
        }
        let refilled = elapsed / interval;
        if refilled >= i64::from(self.bucket_debt) {
            self.bucket_debt = 0;
        } else {
            // refilled 小于 bucket_debt（u8），转换不会截断。
            self.bucket_debt -= refilled as u8;
            self.bucket_refill_ms += refilled * interval;
        }
    }

    fn bucket_available(&self, cfg: &GateConfig) -> bool {
        self.bucket_debt < cfg.loud_bucket_capacity
    }

    fn take_token(&mut self, now_ms: i64) {
        if self.bucket_debt == 0 {
            self.bucket_refill_ms = now_ms;
        }
        self.bucket_debt = self.bucket_debt.saturating_add(1);
    }

    /// 厂商调用是否还有余量。固定窗口从窗口内第一次调用起算，窗口过去即视为清零。
    fn vendor_room(&self, now_ms: i64, cfg: &GateConfig) -> bool {
        since(now_ms, self.vendor_window_ms) >= cfg.vendor_cap_window_ms
            || self.vendor_count < cfg.vendor_cap
    }

    fn vendor_take(&mut self, now_ms: i64, cfg: &GateConfig) {
        if since(now_ms, self.vendor_window_ms) >= cfg.vendor_cap_window_ms {
            self.vendor_window_ms = now_ms;
            self.vendor_count = 0;
        }
        self.vendor_count = self.vendor_count.saturating_add(1);
    }

    /// 当前窗口结束、厂商调用重新有余量的时刻。
    fn vendor_free_at(&self, cfg: &GateConfig) -> i64 {
        self.vendor_window_ms
            .saturating_add(cfg.vendor_cap_window_ms)
    }

    /// 记下一次推送：设备级的最近两次时刻，以及这个会话在最近会话表里的时刻。
    fn record_push(&mut self, now_ms: i64, tag: ConversationTag) {
        self.prev_admit_ms = self.last_admit_ms;
        self.last_admit_ms = now_ms;
        let index = match self
            .recent
            .iter()
            .position(|entry| entry.at_ms != NEVER && entry.tag == tag.0)
        {
            Some(index) => index,
            // 空位的时刻是 NEVER，最小，所以先用空位，满了再替换最久没推送的会话；
            // 时刻相同取靠前的一项，保证两边实现得到同样的表。
            None => self
                .recent
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.at_ms)
                .map_or(0, |(index, _)| index),
        };
        self.recent[index] = RecentEntry {
            tag: tag.0,
            at_ms: now_ms,
        };
    }

    /// 窗口内产生过推送的会话数。
    fn recent_conversations(&self, now_ms: i64, window_ms: i64) -> usize {
        self.recent
            .iter()
            .filter(|entry| entry.at_ms != NEVER && since(now_ms, entry.at_ms) < window_ms)
            .count()
    }

    /// 摘要模式的惰性维护：退出判断与阶梯回落，在记下这次推送之前调用。
    fn settle_digest(&mut self, now_ms: i64, cfg: &GateConfig) {
        if !self.digest_active() {
            return;
        }
        // 不算这一次，退出窗口内的推送少于 2 次：上上次推送已在窗口之外。
        if since(now_ms, self.prev_admit_ms) >= cfg.digest_exit_window_ms {
            self.exit_digest();
            return;
        }
        let idle = since(now_ms, self.last_admit_ms);
        if idle >= cfg.digest_ladder.at(self.digest_level) {
            self.digest_flags &= !DIGEST_SUPPRESSED;
        }
        self.digest_level = cfg.digest_ladder.down_after_idle(self.digest_level, idle);
    }

    /// 进入摘要模式，开始新的一期。
    ///
    /// 本期从此刻算起，第一次摘要响铃要等满一个摘要间隔（同时距上次任何响铃也满一个间隔）：
    /// 刚触发进入的那批会话大多已经各自提示过，立刻再响一次摘要只会是「1 个会话 1 条」的噪声。
    fn enter_digest(&mut self, now_ms: i64) {
        self.digest_flags = DIGEST_ON;
        self.digest_level = 0;
        self.digest_seq = next_digest_seq(self.digest_seq);
        self.digest_from_ms = now_ms;
        self.digest_conversations = 0;
        self.digest_messages = 0;
    }

    fn exit_digest(&mut self) {
        self.digest_flags = 0;
        self.digest_level = 0;
        self.digest_from_ms = NEVER;
        self.digest_conversations = 0;
        self.digest_messages = 0;
    }

    /// 摘要模式下记入一条消息；会话按期去重，靠槽位上记的期号，不需要在设备里存会话集合。
    fn note_digest_message(&mut self, slot: &mut SlotState) {
        self.digest_messages = self.digest_messages.saturating_add(1);
        if slot.digest_mark != self.digest_seq {
            slot.digest_mark = self.digest_seq;
            self.digest_conversations = self.digest_conversations.saturating_add(1);
        }
    }

    /// 摘要响铃：距本期起点与上次任何响铃都满一个摘要间隔，且厂商调用有余量。
    fn try_digest(&mut self, now_ms: i64, cfg: &GateConfig) -> Option<DigestAlert> {
        let interval = cfg
            .digest_ladder
            .at(self.digest_level)
            .max(cfg.device_min_loud_gap_ms);
        if since(now_ms, self.digest_from_ms.max(self.last_ring_ms)) < interval {
            self.digest_flags |= DIGEST_SUPPRESSED;
            return None;
        }
        if !self.vendor_room(now_ms, cfg) {
            return None;
        }
        let alert = DigestAlert {
            conversations: self.digest_conversations,
            messages: self.digest_messages,
        };
        if self.digest_flags & DIGEST_SUPPRESSED != 0 {
            self.digest_level = cfg.digest_ladder.up(self.digest_level);
        }
        self.digest_flags &= !DIGEST_SUPPRESSED;
        self.last_ring_ms = now_ms;
        self.vendor_take(now_ms, cfg);
        self.digest_seq = next_digest_seq(self.digest_seq);
        self.digest_from_ms = now_ms;
        self.digest_conversations = 0;
        self.digest_messages = 0;
        Some(alert)
    }
}

/// 下一个摘要期号，跳过 0：新开的槽位期号为 0，不能与任何一期相同。
const fn next_digest_seq(seq: u16) -> u16 {
    match seq.wrapping_add(1) {
        0 => 1,
        next => next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUP: ConversationKind = ConversationKind::Group;
    const DIRECT: ConversationKind = ConversationKind::Direct;
    const SIX: [&str; 6] = ["c0", "c1", "c2", "c3", "c4", "c5"];

    fn input(kind: ConversationKind, tier: Tier, name: &str) -> GateInput {
        GateInput {
            kind,
            tier,
            tag: ConversationTag::of(name.as_bytes()),
        }
    }

    /// 一台设备加若干会话槽位的小夹具，按会话名取槽位。
    struct Rig {
        gate: Gate,
        device: DeviceState,
        slots: std::vec::Vec<(&'static str, SlotState)>,
    }

    impl Rig {
        fn new() -> Self {
            Self::with(GateConfig::DEFAULT)
        }

        fn with(config: GateConfig) -> Self {
            Self {
                gate: Gate::new(config),
                device: DeviceState::default(),
                slots: std::vec::Vec::new(),
            }
        }

        fn slot(&mut self, name: &'static str) -> &mut SlotState {
            let index = match self.slots.iter().position(|(n, _)| *n == name) {
                Some(index) => index,
                None => {
                    self.slots.push((name, SlotState::default()));
                    self.slots.len() - 1
                }
            };
            &mut self.slots[index].1
        }

        fn admit(
            &mut self,
            now: i64,
            kind: ConversationKind,
            tier: Tier,
            name: &'static str,
        ) -> Admitted {
            let gate = self.gate;
            let mut device = self.device;
            let slot = self.slot(name);
            let out = gate.admit(now, input(kind, tier, name), slot, &mut device);
            self.device = device;
            out
        }

        fn due(&mut self, now: i64, name: &'static str) -> DueAction {
            let gate = self.gate;
            let mut device = self.device;
            let slot = self.slot(name);
            let out = gate.on_due(now, slot, &mut device);
            self.device = device;
            out
        }

        fn read(&mut self, now: i64, name: &'static str) -> ReadOutcome {
            let gate = self.gate;
            let device = self.device;
            gate.on_read(now, self.slot(name), &device)
        }
    }

    fn loud(count: u32) -> SlotAction {
        SlotAction::Loud { count }
    }

    fn quiet(count: u32) -> SlotAction {
        SlotAction::Quiet { count }
    }

    #[test]
    fn first_message_of_an_idle_slot_rings() {
        let mut rig = Rig::new();
        let out = rig.admit(0, GROUP, Tier::Normal, "g");
        assert_eq!(
            out,
            Admitted {
                slot: loud(1),
                digest: None
            }
        );
        assert_eq!(out.vendor_calls(), 1);
        assert_eq!(rig.slot("g").version, 1);
        assert_eq!(rig.device.version, 1);
    }

    #[test]
    fn sparse_direct_chat_rings_every_message() {
        let mut rig = Rig::new();
        for (i, at) in [0, 20_000, 45_000, 100_000, 200_000]
            .into_iter()
            .enumerate()
        {
            let out = rig.admit(at, DIRECT, Tier::Normal, "d");
            assert_eq!(out.slot, loud(i as u32 + 1), "message at {at}");
        }
    }

    #[test]
    fn burst_is_coalesced_and_finished_by_a_quiet_tail() {
        let mut rig = Rig::new();
        assert_eq!(rig.admit(0, GROUP, Tier::Normal, "g").slot, loud(1));
        // 2 秒静默更新间隔内的消息被吞并，第一次吞并登记尾部刷新。
        assert_eq!(
            rig.admit(500, GROUP, Tier::Normal, "g").slot,
            SlotAction::Defer {
                due_ms: 2_000,
                count: 2
            }
        );
        assert_eq!(
            rig.admit(900, GROUP, Tier::Normal, "g").slot,
            SlotAction::CountOnly { count: 3 }
        );
        // 提前领取：不发，原样改登记。
        assert_eq!(rig.due(1_999, "g"), DueAction::Defer { due_ms: 2_000 });
        assert_eq!(rig.due(2_000, "g"), DueAction::Tail { count: 3 });
        assert_eq!(rig.slot("g").pending(), 0);
        assert_eq!(rig.slot("g").due_ms(), None);
        // 再次领取：没有要发的。
        assert_eq!(rig.due(2_001, "g"), DueAction::Nothing);
        // 尾部刷新因为有吞并而升到 5 秒档：3 秒后的消息被吞并到 7 秒。
        assert_eq!(
            rig.admit(5_000, GROUP, Tier::Normal, "g").slot,
            SlotAction::Defer {
                due_ms: 7_000,
                count: 4
            }
        );
    }

    #[test]
    fn loud_ladder_steps_up_on_suppression_and_down_after_quiet() {
        let mut rig = Rig::new();
        assert_eq!(rig.admit(0, GROUP, Tier::Normal, "g").slot, loud(1));
        // 10 秒时被群聊 30 秒响铃间隔压住，静默更新。
        assert_eq!(rig.admit(10_000, GROUP, Tier::Normal, "g").slot, quiet(2));
        // 30 秒响铃；上个间隔有压住的消息，升到 120 秒档。
        assert_eq!(rig.admit(30_000, GROUP, Tier::Normal, "g").slot, loud(3));
        assert_eq!(rig.admit(60_000, GROUP, Tier::Normal, "g").slot, quiet(4));
        assert_eq!(rig.admit(149_000, GROUP, Tier::Normal, "g").slot, quiet(5));
        // 150 秒响铃；又有压住的消息，升到 600 秒档。
        assert_eq!(rig.admit(150_000, GROUP, Tier::Normal, "g").slot, loud(6));
        assert_eq!(rig.slot("g").loud_level, 2);
        // 安静 720 秒 = 600 秒档一个间隔加 120 秒档一个间隔：逐档降回 30 秒档，压住标志作废。
        assert_eq!(rig.admit(870_000, GROUP, Tier::Normal, "g").slot, loud(7));
        assert_eq!(rig.slot("g").loud_level, 0);
        assert_eq!(rig.admit(890_000, GROUP, Tier::Normal, "g").slot, quiet(8));
        assert_eq!(rig.admit(900_000, GROUP, Tier::Normal, "g").slot, loud(9));
        assert_eq!(rig.slot("g").loud_level, 1);
    }

    #[test]
    fn partial_quiet_steps_down_one_level_at_a_time() {
        let mut rig = Rig::new();
        rig.admit(0, GROUP, Tier::Normal, "g");
        rig.admit(10_000, GROUP, Tier::Normal, "g");
        rig.admit(30_000, GROUP, Tier::Normal, "g");
        rig.admit(60_000, GROUP, Tier::Normal, "g");
        rig.admit(150_000, GROUP, Tier::Normal, "g");
        assert_eq!(rig.slot("g").loud_level, 2);
        // 只安静了 600 秒档一个间隔：降到 120 秒档，不是一下回到 30 秒档。
        assert_eq!(rig.admit(750_000, GROUP, Tier::Normal, "g").slot, loud(6));
        assert_eq!(rig.slot("g").loud_level, 1);
        assert_eq!(rig.admit(780_000, GROUP, Tier::Normal, "g").slot, quiet(7));
    }

    #[test]
    fn device_bucket_limits_rings_across_conversations() {
        let mut rig = Rig::new();
        let names = ["a", "b", "c", "d"];
        let mut slots = std::vec::Vec::new();
        for (i, name) in names.into_iter().enumerate() {
            slots.push(rig.admit(i as i64 * 2_000, DIRECT, Tier::Normal, name).slot);
        }
        // 桶里 3 个令牌：前三个会话的第一条响铃，第四个静默但照常出现。
        assert_eq!(slots, [loud(1), loud(1), loud(1), quiet(1)]);
        // 第一个令牌在 0 秒取出，15 秒后补回一个，d 的下一条可以响铃。
        assert_eq!(rig.admit(14_999, DIRECT, Tier::Normal, "d").slot, quiet(2));
        assert_eq!(rig.admit(17_000, DIRECT, Tier::Normal, "d").slot, loud(3));
    }

    #[test]
    fn device_min_gap_applies_to_every_ring() {
        let mut rig = Rig::new();
        assert_eq!(rig.admit(0, DIRECT, Tier::Normal, "a").slot, loud(1));
        assert_eq!(rig.admit(999, DIRECT, Tier::Normal, "b").slot, quiet(1));
        assert_eq!(rig.admit(999, DIRECT, Tier::Priority, "c").slot, quiet(1));
        assert_eq!(rig.admit(1_000, DIRECT, Tier::Priority, "c").slot, loud(2));
    }

    #[test]
    fn priority_bypasses_slot_ladder_and_bucket() {
        let mut rig = Rig::new();
        // 用掉全部令牌。
        for (i, name) in ["a", "b", "c"].into_iter().enumerate() {
            assert_eq!(
                rig.admit(i as i64 * 1_000, GROUP, Tier::Normal, name).slot,
                loud(1)
            );
        }
        // 普通消息：令牌桶空且槽位间隔未到，静默。
        assert_eq!(rig.admit(5_000, GROUP, Tier::Normal, "a").slot, quiet(2));
        // 点名：不看槽位阶梯与令牌桶。
        assert_eq!(rig.admit(6_000, GROUP, Tier::Priority, "a").slot, loud(3));
        assert_eq!(rig.admit(7_000, GROUP, Tier::Priority, "a").slot, loud(4));
        // 点名响铃也重置槽位的响铃计时：之后的普通消息照常等间隔。
        assert_eq!(rig.admit(20_000, GROUP, Tier::Normal, "a").slot, quiet(5));
    }

    #[test]
    fn quiet_tier_never_rings_but_is_still_coalesced() {
        let mut rig = Rig::new();
        assert_eq!(rig.admit(0, GROUP, Tier::Quiet, "q").slot, quiet(1));
        assert_eq!(
            rig.admit(100, GROUP, Tier::Quiet, "q").slot,
            SlotAction::Defer {
                due_ms: 2_000,
                count: 2
            }
        );
        assert_eq!(rig.due(2_000, "q"), DueAction::Tail { count: 2 });
        assert_eq!(rig.admit(3_600_000, GROUP, Tier::Quiet, "q").slot, quiet(1));
        assert_eq!(rig.device.bucket_debt, 0);
    }

    #[test]
    fn vendor_cap_defers_to_the_end_of_the_window() {
        let mut config = GateConfig::DEFAULT;
        config.vendor_cap = 2;
        let mut rig = Rig::with(config);
        assert_eq!(rig.admit(0, DIRECT, Tier::Normal, "a").slot, loud(1));
        assert_eq!(rig.admit(1_000, DIRECT, Tier::Normal, "b").slot, loud(1));
        // 第三次调用超出上限：即使是空闲槽位的第一条也只能推迟到窗口结束。
        assert_eq!(
            rig.admit(2_000, DIRECT, Tier::Normal, "c").slot,
            SlotAction::Defer {
                due_ms: 60_000,
                count: 1
            }
        );
        assert!(!rig.slot("c").has_sent());
        assert_eq!(rig.due(60_000, "c"), DueAction::Tail { count: 1 });
        // 推迟不是因为会话太密，不升静默档。
        assert_eq!(rig.slot("c").quiet_level, 0);
    }

    #[test]
    fn vendor_cap_redefers_tails_that_meet_a_full_window() {
        let mut config = GateConfig::DEFAULT;
        config.vendor_cap = 1;
        let mut rig = Rig::with(config);
        assert_eq!(rig.admit(0, DIRECT, Tier::Normal, "a").slot, loud(1));
        assert_eq!(
            rig.admit(500, DIRECT, Tier::Normal, "a").slot,
            SlotAction::Defer {
                due_ms: 2_000,
                count: 2
            }
        );
        assert_eq!(rig.due(2_000, "a"), DueAction::Defer { due_ms: 60_000 });
        assert_eq!(rig.due(60_000, "a"), DueAction::Tail { count: 2 });
    }

    #[test]
    fn digest_mode_enters_at_six_conversations_and_rings_on_its_own_ladder() {
        let mut rig = Rig::new();
        let names = ["c0", "c1", "c2", "c3", "c4", "c5", "c6"];
        assert_eq!(rig.admit(0, GROUP, Tier::Normal, names[0]).slot, loud(1));
        for (i, &name) in names[1..5].iter().enumerate() {
            let out = rig.admit(100 * (i as i64 + 1), GROUP, Tier::Normal, name);
            // 设备最小间隔内：静默出现。
            assert_eq!(
                out,
                Admitted {
                    slot: quiet(1),
                    digest: None
                }
            );
        }
        assert!(!rig.device.digest_active());
        // 第六个会话：进入摘要模式；摘要要等满 60 秒。
        let out = rig.admit(500, GROUP, Tier::Normal, names[5]);
        assert_eq!(
            out,
            Admitted {
                slot: quiet(1),
                digest: None
            }
        );
        assert!(rig.device.digest_active());
        // 摘要模式下槽位不响铃，即使令牌还有、槽位间隔已到。
        let out = rig.admit(40_000, GROUP, Tier::Normal, names[6]);
        assert_eq!(
            out,
            Admitted {
                slot: quiet(1),
                digest: None
            }
        );
        let out = rig.admit(40_500, GROUP, Tier::Normal, "c0");
        assert_eq!(out.slot, quiet(2));
        assert_eq!(out.digest, None);
        // 60.5 秒：本期从 0.5 秒算起满 60 秒，摘要响铃；本期 c5、c6、c0、c1 共 4 个会话 5 条。
        rig.admit(50_000, GROUP, Tier::Normal, "c1");
        let out = rig.admit(60_500, GROUP, Tier::Normal, "c1");
        assert_eq!(
            out.digest,
            Some(DigestAlert {
                conversations: 4,
                messages: 5
            })
        );
        assert_eq!(out.slot, quiet(3));
        assert_eq!(out.vendor_calls(), 2);
        // 上一期有被压住的消息：升到 120 秒档，下一次摘要要到 180.5 秒。
        let out = rig.admit(120_000, GROUP, Tier::Normal, "c2");
        assert_eq!(out.digest, None);
        let out = rig.admit(150_000, GROUP, Tier::Normal, "c4");
        assert_eq!(out.digest, None);
        let out = rig.admit(180_500, GROUP, Tier::Normal, "c3");
        assert_eq!(
            out.digest,
            Some(DigestAlert {
                conversations: 3,
                messages: 3
            })
        );
        assert!(rig.device.digest_active());
    }

    #[test]
    fn digest_mode_exits_after_a_quiet_exit_window() {
        let mut rig = Rig::new();
        for (i, name) in SIX.into_iter().enumerate() {
            rig.admit(i as i64 * 1_000, GROUP, Tier::Normal, name);
        }
        assert!(rig.device.digest_active());
        // 5 秒之后只有一次推送；再过 120 秒，窗口内（不算这一次）只剩 1 次，退出。
        rig.admit(10_000, GROUP, Tier::Normal, "c0");
        let out = rig.admit(130_000, DIRECT, Tier::Normal, "fresh");
        assert!(!rig.device.digest_active());
        assert_eq!(out.slot, loud(1));
    }

    #[test]
    fn priority_still_rings_in_digest_mode() {
        let mut rig = Rig::new();
        for (i, name) in SIX.into_iter().enumerate() {
            rig.admit(i as i64 * 100, GROUP, Tier::Normal, name);
        }
        assert!(rig.device.digest_active());
        let out = rig.admit(2_000, GROUP, Tier::Priority, "c3");
        assert_eq!(
            out,
            Admitted {
                slot: loud(2),
                digest: None
            }
        );
    }

    #[test]
    fn read_clears_only_visible_live_slots() {
        let mut rig = Rig::new();
        rig.admit(0, GROUP, Tier::Normal, "g");
        rig.admit(100, GROUP, Tier::Normal, "g");
        let version = rig.slot("g").version;
        assert_eq!(
            rig.read(1_000, "g"),
            ReadOutcome {
                clear: true,
                cancel_tail: true
            }
        );
        assert_eq!(rig.slot("g").version, version + 1);
        assert!(!rig.slot("g").is_open());
        // 读过之后的下一条重新算第一条（令牌还有、设备间隔已过）。
        assert_eq!(rig.admit(5_000, GROUP, Tier::Normal, "g").slot, loud(1));
        // 没打开的槽位：没有修正，版本不变。
        assert_eq!(rig.read(6_000, "never"), ReadOutcome::default());
        assert_eq!(rig.slot("never").version, 0);
        // 过期的槽位：视同不存在，不发修正。
        rig.admit(10_000, DIRECT, Tier::Normal, "old");
        assert_eq!(
            rig.read(10_000 + 1_800_000, "old"),
            ReadOutcome {
                clear: false,
                cancel_tail: false
            }
        );
    }

    #[test]
    fn read_of_a_never_sent_slot_needs_no_clear() {
        let mut config = GateConfig::DEFAULT;
        config.vendor_cap = 1;
        let mut rig = Rig::with(config);
        rig.admit(0, DIRECT, Tier::Normal, "a");
        assert!(matches!(
            rig.admit(1_000, DIRECT, Tier::Normal, "b").slot,
            SlotAction::Defer { .. }
        ));
        assert_eq!(
            rig.read(2_000, "b"),
            ReadOutcome {
                clear: false,
                cancel_tail: true
            }
        );
        assert_eq!(rig.due(60_000, "b"), DueAction::Nothing);
    }

    #[test]
    fn active_device_invalidates_slots_and_digest() {
        let mut rig = Rig::new();
        for (i, name) in SIX.into_iter().enumerate() {
            rig.admit(i as i64 * 100, GROUP, Tier::Normal, name);
        }
        rig.admit(800, GROUP, Tier::Normal, "c0");
        assert!(rig.slot("c0").due_ms().is_some());
        assert!(rig.device.digest_active());
        let generation = rig.device.generation();
        let version = rig.device.version;
        let gate = rig.gate;
        gate.on_active(&mut rig.device);
        assert_eq!(rig.device.generation(), generation + 1);
        assert_eq!(rig.device.version, version + 1);
        assert!(!rig.device.digest_active());
        // 旧代槽位的尾部刷新作废。
        assert_eq!(rig.due(2_000, "c0"), DueAction::Nothing);
        assert!(!rig.slot("c0").is_open());
        // 旧代槽位上来的新消息算第一条，且旧代的已读修正不再需要。
        assert_eq!(
            rig.read(2_000, "c1"),
            ReadOutcome {
                clear: false,
                cancel_tail: false
            }
        );
        assert_eq!(rig.admit(3_000, GROUP, Tier::Normal, "c2").slot, loud(1));
        assert_eq!(rig.slot("c2").count(), 1);
    }

    #[test]
    fn idle_slot_expires_lazily() {
        let mut rig = Rig::new();
        rig.admit(0, GROUP, Tier::Normal, "g");
        rig.admit(1_000, GROUP, Tier::Normal, "g");
        assert_eq!(rig.slot("g").count(), 2);
        // 未满 30 分钟：仍是同一个槽位，计数接着累加。
        assert_eq!(
            rig.admit(1_000 + 1_799_999, GROUP, Tier::Normal, "g").slot,
            loud(3)
        );
        // 再空闲满 30 分钟：重新开始。
        assert_eq!(
            rig.admit(1_000 + 1_799_999 + 1_800_000, GROUP, Tier::Normal, "g")
                .slot,
            loud(1)
        );
    }

    #[test]
    fn tails_stay_quiet_even_when_the_loud_interval_has_passed() {
        let mut rig = Rig::new();
        rig.admit(0, DIRECT, Tier::Normal, "d");
        rig.admit(500, DIRECT, Tier::Normal, "d");
        // 到期时单聊响铃间隔（10 秒）早已过去，尾部刷新依然静默。
        assert_eq!(rig.due(20_000, "d"), DueAction::Tail { count: 2 });
    }

    #[test]
    fn clock_going_backwards_never_sends_twice() {
        let mut rig = Rig::new();
        assert_eq!(rig.admit(10_000, DIRECT, Tier::Normal, "d").slot, loud(1));
        assert_eq!(
            rig.admit(5_000, DIRECT, Tier::Normal, "d").slot,
            SlotAction::Defer {
                due_ms: 12_000,
                count: 2
            }
        );
    }

    #[test]
    fn fail_open_alerts_like_a_first_message() {
        assert_eq!(fail_open(Tier::Normal), loud(1));
        assert_eq!(fail_open(Tier::Priority), loud(1));
        assert_eq!(fail_open(Tier::Quiet), quiet(1));
    }

    #[test]
    fn conversation_tag_is_fnv1a() {
        assert_eq!(ConversationTag::of(b""), ConversationTag(0x811c_9dc5));
        assert_eq!(ConversationTag::of(b"a"), ConversationTag(0xe40c_292c));
        assert_eq!(ConversationTag::of(b"foobar"), ConversationTag(0xbf9c_f968));
    }

    #[test]
    fn slot_action_helpers() {
        assert!(loud(1).sends());
        assert!(quiet(2).sends());
        assert!(
            !SlotAction::Defer {
                due_ms: 1,
                count: 3
            }
            .sends()
        );
        assert!(!SlotAction::CountOnly { count: 4 }.sends());
        assert_eq!(
            SlotAction::Defer {
                due_ms: 1,
                count: 3
            }
            .count(),
            3
        );
        assert_eq!(SlotAction::CountOnly { count: 4 }.count(), 4);
        assert_eq!(next_digest_seq(u16::MAX), 1);
        assert_eq!(next_digest_seq(0), 1);
        assert_eq!(next_digest_seq(7), 8);
    }

    /// 设计里的例子：一个群一小时一万条。精确的时刻与次数由向量文件给出，这里只守住上界。
    #[test]
    fn ten_thousand_messages_in_one_group_hour() {
        let mut rig = Rig::new();
        let mut loud_count = 0;
        let mut vendor_calls = 0;
        let mut last_count = 0;
        for i in 0..10_000_i64 {
            let now = i * 360;
            if let Some(due) = rig.slot("g").due_ms()
                && due <= now
                && let DueAction::Tail { count } = rig.due(due, "g")
            {
                vendor_calls += 1;
                last_count = count;
            }
            let out = rig.admit(now, GROUP, Tier::Normal, "g");
            match out.slot {
                SlotAction::Loud { count } => {
                    loud_count += 1;
                    last_count = count;
                }
                SlotAction::Quiet { count } => last_count = count,
                SlotAction::Defer { .. } | SlotAction::CountOnly { .. } => {}
            }
            vendor_calls += out.vendor_calls();
        }
        if let Some(due) = rig.slot("g").due_ms()
            && let DueAction::Tail { count } = rig.due(due, "g")
        {
            vendor_calls += 1;
            last_count = count;
        }
        assert_eq!(loud_count, 8);
        assert!(vendor_calls <= 30, "{vendor_calls} vendor calls");
        assert_eq!(last_count, 10_000);
    }
}
