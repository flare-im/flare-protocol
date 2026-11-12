//! 闸门的持久化状态与定长二进制编码。
//!
//! 服务端把每个（设备，会话）的槽位和每台设备的预算各存成一个 Redis 值，读出后跑规则，
//! 再按 `version` 比较后写回。所以状态要小、编码要定长且不依赖 serde：
//! 槽位 58 字节，设备 169 字节，都是小端序，第 0 字节是布局号，`version` 固定在第 1 到 8 字节，
//! 写回脚本可以直接比较这 8 个字节而不必解析整个值。
//!
//! 「不存在」与 `default()` 等价：调用方可以给值设过期时间（不短于槽位过期时长），
//! 过期删除和规则里的惰性过期得到同样的结果。

use core::fmt;

use crate::time::NEVER;

/// 设备里「最近产生推送的会话」表的容量，决定进入摘要模式最多能数到几个会话。
pub(crate) const RECENT_CAPACITY: usize = 8;

/// 槽位标志：本槽位上一次响铃之后，有普通消息因为响铃间隔未到而没响。
pub(crate) const LOUD_SUPPRESSED: u8 = 0b01;
/// 槽位标志：本槽位上一次更新之后，有消息因为静默更新间隔未到而被吞并。
pub(crate) const QUIET_SUPPRESSED: u8 = 0b10;
const SLOT_FLAGS: u8 = LOUD_SUPPRESSED | QUIET_SUPPRESSED;

/// 设备标志：处于摘要模式。
pub(crate) const DIGEST_ON: u8 = 0b01;
/// 设备标志：上一次摘要响铃之后，有普通消息因为摘要间隔未到而没响。
pub(crate) const DIGEST_SUPPRESSED: u8 = 0b10;
const DEVICE_FLAGS: u8 = DIGEST_ON | DIGEST_SUPPRESSED;

/// 二进制布局号。布局一变就换号，旧值解码失败，调用方按「不存在」放行重建。
const LAYOUT: u8 = 1;

/// [`SlotState::encode`] 的字节数。
pub const SLOT_STATE_LEN: usize = 58;
/// [`DeviceState::encode`] 的字节数。
pub const DEVICE_STATE_LEN: usize = 169;

// 每个槽位、每台设备各占一个存储值，编码必须保持在 200 字节以内。
const _: () = assert!(SLOT_STATE_LEN < 200 && DEVICE_STATE_LEN < 200);

/// 一个（设备，会话）槽位，对应系统通知栏里这个会话的那一条通知。
///
/// 只有 `version` 对外可写：其余字段的含义随规则演进，调用方只通过 [`crate::Gate`]
/// 改动它们，通过只读方法查看。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SlotState {
    /// 比较后写入用的版本号。规则每次改动状态都加一；调用方记住读出时的值，
    /// 写回时与存储里的值比较。
    pub version: u64,
    /// 打开这个槽位时设备的代号。设备回到前台会换代，旧代的槽位随之作废，
    /// 不必逐个找出来删除。
    pub(crate) generation: u32,
    /// 最近一条进闸门的消息时刻；[`NEVER`] 表示槽位空闲。
    pub(crate) last_msg_ms: i64,
    /// 最近一次为这个槽位调用厂商（响铃、静默或尾部刷新）的时刻。
    pub(crate) last_send_ms: i64,
    /// 最近一次响铃的时刻。
    pub(crate) last_loud_ms: i64,
    /// 已登记的尾部刷新时刻；[`NEVER`] 表示没有。
    pub(crate) due_ms: i64,
    /// 槽位打开以来的消息数，即通知上的「[N 条]」。
    pub(crate) count: u32,
    /// 还没体现在已发通知里的消息数。
    pub(crate) pending: u32,
    /// 静默更新阶梯的档位。
    pub(crate) quiet_level: u8,
    /// 响铃阶梯的档位。
    pub(crate) loud_level: u8,
    /// [`LOUD_SUPPRESSED`]、[`QUIET_SUPPRESSED`]。
    pub(crate) flags: u8,
    /// 这个会话最近一次计入摘要的摘要期号，用来不重复计数会话。
    pub(crate) digest_mark: u16,
}

impl Default for SlotState {
    fn default() -> Self {
        Self {
            version: 0,
            generation: 0,
            last_msg_ms: NEVER,
            last_send_ms: NEVER,
            last_loud_ms: NEVER,
            due_ms: NEVER,
            count: 0,
            pending: 0,
            quiet_level: 0,
            loud_level: 0,
            flags: 0,
            digest_mark: 0,
        }
    }
}

impl SlotState {
    /// 槽位是否打开（来过消息且没被读、没过期、没随设备换代作废）。
    ///
    /// 惰性过期要到下一次 [`crate::Gate::admit`] 才生效，这里只看有没有消息。
    pub fn is_open(&self) -> bool {
        self.last_msg_ms != NEVER
    }

    /// 槽位打开以来的消息数。
    pub fn count(&self) -> u32 {
        self.count
    }

    /// 还没体现在已发通知里的消息数。
    pub fn pending(&self) -> u32 {
        self.pending
    }

    /// 已登记的尾部刷新时刻。
    pub fn due_ms(&self) -> Option<i64> {
        (self.due_ms != NEVER).then_some(self.due_ms)
    }

    /// 是否已经为这个槽位调用过厂商，也就是设备上可能有这条通知。
    pub fn has_sent(&self) -> bool {
        self.last_send_ms != NEVER
    }

    /// 关闭槽位，只保留版本号，回到与 `default()` 相同的语义。
    pub(crate) fn close(&mut self) {
        *self = Self {
            version: self.version,
            ..Self::default()
        };
    }

    /// 定长编码，见模块说明与 README 里的布局表。
    pub fn encode(&self) -> [u8; SLOT_STATE_LEN] {
        let mut out = [0u8; SLOT_STATE_LEN];
        let mut w = Writer::new(&mut out);
        w.u8(LAYOUT);
        w.u64(self.version);
        w.u32(self.generation);
        w.i64(self.last_msg_ms);
        w.i64(self.last_send_ms);
        w.i64(self.last_loud_ms);
        w.i64(self.due_ms);
        w.u32(self.count);
        w.u32(self.pending);
        w.u8(self.quiet_level);
        w.u8(self.loud_level);
        w.u8(self.flags);
        w.u16(self.digest_mark);
        debug_assert_eq!(w.at, SLOT_STATE_LEN);
        out
    }

    /// 解码 [`SlotState::encode`] 的输出。长度、布局号或标志位不对时返回错误，
    /// 调用方按「不存在」处理（放行）并用新状态覆盖。
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes, SLOT_STATE_LEN)?;
        let state = Self {
            version: r.u64(),
            generation: r.u32(),
            last_msg_ms: r.i64(),
            last_send_ms: r.i64(),
            last_loud_ms: r.i64(),
            due_ms: r.i64(),
            count: r.u32(),
            pending: r.u32(),
            quiet_level: r.u8(),
            loud_level: r.u8(),
            flags: r.u8(),
            digest_mark: r.u16(),
        };
        if state.flags & !SLOT_FLAGS != 0 {
            return Err(DecodeError::Flags(state.flags));
        }
        Ok(state)
    }
}

/// 最近产生推送的一个会话。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub(crate) struct RecentEntry {
    /// 会话标签（[`crate::ConversationTag`]）。
    pub(crate) tag: u32,
    /// 这个会话最近一次推送的时刻；[`NEVER`] 表示空位。
    pub(crate) at_ms: i64,
}

impl RecentEntry {
    pub(crate) const EMPTY: Self = Self {
        tag: 0,
        at_ms: NEVER,
    };
}

/// 一台设备的跨会话预算：响铃令牌桶、最小响铃间隔、厂商调用上限与摘要模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DeviceState {
    /// 比较后写入用的版本号，规则同 [`SlotState::version`]。
    pub version: u64,
    /// 设备代号，每次回到前台加一。
    pub(crate) generation: u32,
    /// 最近一次响铃（任何会话、任何等级，含摘要）的时刻。
    pub(crate) last_ring_ms: i64,
    /// 令牌桶里已用掉、还没补回的令牌数。存「欠账」而不是余量，默认值 0 就是满桶，
    /// 不必在构造时知道容量。
    pub(crate) bucket_debt: u8,
    /// 令牌补充的计时起点。
    pub(crate) bucket_refill_ms: i64,
    /// 厂商调用上限当前窗口的起点。
    pub(crate) vendor_window_ms: i64,
    /// 当前窗口内的厂商调用次数。
    pub(crate) vendor_count: u8,
    /// 最近一次进闸门的时刻。
    pub(crate) last_admit_ms: i64,
    /// 再往前一次进闸门的时刻；用来判断「窗口内推送少于 2 次」。
    pub(crate) prev_admit_ms: i64,
    /// [`DIGEST_ON`]、[`DIGEST_SUPPRESSED`]。
    pub(crate) digest_flags: u8,
    /// 摘要响铃阶梯的档位。
    pub(crate) digest_level: u8,
    /// 摘要期号：进入摘要模式和每次摘要响铃都换一期，从不为 0。
    pub(crate) digest_seq: u16,
    /// 本期起点（进入摘要模式或上一次摘要响铃）。
    pub(crate) digest_from_ms: i64,
    /// 本期有消息的会话数。
    pub(crate) digest_conversations: u16,
    /// 本期的消息数。
    pub(crate) digest_messages: u32,
    /// 最近产生推送的会话，每个会话一项，满了替换最久没推送的那一项。
    pub(crate) recent: [RecentEntry; RECENT_CAPACITY],
}

impl Default for DeviceState {
    fn default() -> Self {
        Self {
            version: 0,
            generation: 0,
            last_ring_ms: NEVER,
            bucket_debt: 0,
            bucket_refill_ms: NEVER,
            vendor_window_ms: NEVER,
            vendor_count: 0,
            last_admit_ms: NEVER,
            prev_admit_ms: NEVER,
            digest_flags: 0,
            digest_level: 0,
            digest_seq: 0,
            digest_from_ms: NEVER,
            digest_conversations: 0,
            digest_messages: 0,
            recent: [RecentEntry::EMPTY; RECENT_CAPACITY],
        }
    }
}

impl DeviceState {
    /// 是否处于摘要模式。退出条件在下一次 [`crate::Gate::admit`] 时惰性判断。
    pub fn digest_active(&self) -> bool {
        self.digest_flags & DIGEST_ON != 0
    }

    /// 设备代号。
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// 定长编码，见模块说明与 README 里的布局表。
    pub fn encode(&self) -> [u8; DEVICE_STATE_LEN] {
        let mut out = [0u8; DEVICE_STATE_LEN];
        let mut w = Writer::new(&mut out);
        w.u8(LAYOUT);
        w.u64(self.version);
        w.u32(self.generation);
        w.i64(self.last_ring_ms);
        w.u8(self.bucket_debt);
        w.i64(self.bucket_refill_ms);
        w.i64(self.vendor_window_ms);
        w.u8(self.vendor_count);
        w.i64(self.last_admit_ms);
        w.i64(self.prev_admit_ms);
        w.u8(self.digest_flags);
        w.u8(self.digest_level);
        w.u16(self.digest_seq);
        w.i64(self.digest_from_ms);
        w.u16(self.digest_conversations);
        w.u32(self.digest_messages);
        for entry in &self.recent {
            w.u32(entry.tag);
            w.i64(entry.at_ms);
        }
        debug_assert_eq!(w.at, DEVICE_STATE_LEN);
        out
    }

    /// 解码 [`DeviceState::encode`] 的输出，错误处理同 [`SlotState::decode`]。
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes, DEVICE_STATE_LEN)?;
        let mut state = Self {
            version: r.u64(),
            generation: r.u32(),
            last_ring_ms: r.i64(),
            bucket_debt: r.u8(),
            bucket_refill_ms: r.i64(),
            vendor_window_ms: r.i64(),
            vendor_count: r.u8(),
            last_admit_ms: r.i64(),
            prev_admit_ms: r.i64(),
            digest_flags: r.u8(),
            digest_level: r.u8(),
            digest_seq: r.u16(),
            digest_from_ms: r.i64(),
            digest_conversations: r.u16(),
            digest_messages: r.u32(),
            recent: [RecentEntry::EMPTY; RECENT_CAPACITY],
        };
        for entry in &mut state.recent {
            entry.tag = r.u32();
            entry.at_ms = r.i64();
        }
        if state.digest_flags & !DEVICE_FLAGS != 0 {
            return Err(DecodeError::Flags(state.digest_flags));
        }
        Ok(state)
    }
}

/// 状态解码失败。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecodeError {
    /// 长度不对。
    Length {
        /// 应有的字节数。
        expected: usize,
        /// 实际的字节数。
        actual: usize,
    },
    /// 布局号不认识。
    Layout(u8),
    /// 标志字节含未定义的位。
    Flags(u8),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { expected, actual } => {
                write!(f, "gate state has {actual} bytes, expected {expected}")
            }
            Self::Layout(layout) => write!(f, "unknown gate state layout {layout}"),
            Self::Flags(flags) => write!(f, "gate state has undefined flag bits {flags:#04x}"),
        }
    }
}

impl core::error::Error for DecodeError {}

/// 写入定长缓冲区的游标。长度在编译期由常量保证，越界即是编码表写错，直接 panic。
struct Writer<'a> {
    buf: &'a mut [u8],
    at: usize,
}

impl<'a> Writer<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, at: 0 }
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.buf[self.at..self.at + bytes.len()].copy_from_slice(bytes);
        self.at += bytes.len();
    }

    fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.bytes(&value.to_le_bytes());
    }
}

/// 读取定长缓冲区的游标。构造时已核对长度，之后的读取不会越界。
struct Reader<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8], expected: usize) -> Result<Self, DecodeError> {
        if buf.len() != expected {
            return Err(DecodeError::Length {
                expected,
                actual: buf.len(),
            });
        }
        if buf[0] != LAYOUT {
            return Err(DecodeError::Layout(buf[0]));
        }
        Ok(Self { buf, at: 1 })
    }

    fn array<const N: usize>(&mut self) -> [u8; N] {
        let mut out = [0u8; N];
        out.copy_from_slice(&self.buf[self.at..self.at + N]);
        self.at += N;
        out
    }

    fn u8(&mut self) -> u8 {
        let [value] = self.array::<1>();
        value
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.array())
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.array())
    }

    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.array())
    }

    fn i64(&mut self) -> i64 {
        i64::from_le_bytes(self.array())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn busy_slot() -> SlotState {
        SlotState {
            version: 0x0102_0304_0506_0708,
            generation: 7,
            last_msg_ms: 1_700_000_123_456,
            last_send_ms: 1_700_000_120_000,
            last_loud_ms: 1_700_000_000_000,
            due_ms: NEVER,
            count: 10_000,
            pending: 37,
            quiet_level: 4,
            loud_level: 2,
            flags: LOUD_SUPPRESSED | QUIET_SUPPRESSED,
            digest_mark: 513,
        }
    }

    fn busy_device() -> DeviceState {
        let mut device = DeviceState {
            version: 42,
            generation: u32::MAX,
            last_ring_ms: 1_700_000_000_000,
            bucket_debt: 2,
            bucket_refill_ms: 1_699_999_990_000,
            vendor_window_ms: 1_699_999_950_000,
            vendor_count: 29,
            last_admit_ms: 1_700_000_001_000,
            prev_admit_ms: 1_700_000_000_500,
            digest_flags: DIGEST_ON | DIGEST_SUPPRESSED,
            digest_level: 1,
            digest_seq: u16::MAX,
            digest_from_ms: 1_699_999_940_000,
            digest_conversations: 20,
            digest_messages: 3_456,
            ..DeviceState::default()
        };
        for (i, entry) in device.recent.iter_mut().enumerate() {
            entry.tag = 0xdead_0000 + i as u32;
            entry.at_ms = if i == 3 {
                NEVER
            } else {
                1_700_000_000_000 + i as i64
            };
        }
        device
    }

    #[test]
    fn encoded_sizes_are_fixed() {
        assert_eq!(SlotState::default().encode().len(), 58);
        assert_eq!(DeviceState::default().encode().len(), 169);
    }

    #[test]
    fn slot_round_trips() {
        for slot in [SlotState::default(), busy_slot()] {
            assert_eq!(SlotState::decode(&slot.encode()), Ok(slot));
        }
    }

    #[test]
    fn device_round_trips() {
        for device in [DeviceState::default(), busy_device()] {
            assert_eq!(DeviceState::decode(&device.encode()), Ok(device));
        }
    }

    #[test]
    fn version_sits_at_bytes_one_to_eight_little_endian() {
        let slot = busy_slot();
        assert_eq!(slot.encode()[1..9], slot.version.to_le_bytes());
        let device = busy_device();
        assert_eq!(device.encode()[1..9], device.version.to_le_bytes());
        assert_eq!(slot.encode()[0], LAYOUT);
    }

    #[test]
    fn decode_rejects_bad_length_layout_and_flags() {
        let bytes = busy_slot().encode();
        assert_eq!(
            SlotState::decode(&bytes[..57]),
            Err(DecodeError::Length {
                expected: 58,
                actual: 57
            })
        );
        assert!(matches!(
            DeviceState::decode(&bytes),
            Err(DecodeError::Length { expected: 169, .. })
        ));
        let mut wrong_layout = bytes;
        wrong_layout[0] = 9;
        assert_eq!(
            SlotState::decode(&wrong_layout),
            Err(DecodeError::Layout(9))
        );
        let mut wrong_flags = bytes;
        wrong_flags[55] = 0b100;
        assert_eq!(
            SlotState::decode(&wrong_flags),
            Err(DecodeError::Flags(0b100))
        );
        let mut device = busy_device().encode();
        device[55] = 0b1000_0001;
        assert_eq!(
            DeviceState::decode(&device),
            Err(DecodeError::Flags(0b1000_0001))
        );
    }

    #[test]
    fn close_keeps_only_the_version() {
        let mut slot = busy_slot();
        slot.close();
        assert_eq!(
            slot,
            SlotState {
                version: busy_slot().version,
                ..SlotState::default()
            }
        );
        assert!(!slot.is_open());
        assert!(!slot.has_sent());
        assert_eq!(slot.due_ms(), None);
    }

    #[test]
    fn accessors_reflect_fields() {
        let mut slot = busy_slot();
        assert!(slot.is_open());
        assert!(slot.has_sent());
        assert_eq!(slot.count(), 10_000);
        assert_eq!(slot.pending(), 37);
        assert_eq!(slot.due_ms(), None);
        slot.due_ms = 5;
        assert_eq!(slot.due_ms(), Some(5));
        let device = busy_device();
        assert!(device.digest_active());
        assert_eq!(device.generation(), u32::MAX);
        assert!(!DeviceState::default().digest_active());
    }

    #[test]
    fn errors_render_readable_messages() {
        // 测试构建不是 no_std，`to_string` 来自 std 的预导入。
        assert_eq!(
            DecodeError::Length {
                expected: 58,
                actual: 3
            }
            .to_string(),
            "gate state has 3 bytes, expected 58"
        );
        assert_eq!(
            DecodeError::Layout(2).to_string(),
            "unknown gate state layout 2"
        );
        assert_eq!(
            DecodeError::Flags(4).to_string(),
            "gate state has undefined flag bits 0x04"
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trips() {
        let slot = busy_slot();
        let json = serde_json::to_string(&slot).unwrap();
        assert_eq!(serde_json::from_str::<SlotState>(&json).unwrap(), slot);
        let device = busy_device();
        let json = serde_json::to_string(&device).unwrap();
        assert_eq!(serde_json::from_str::<DeviceState>(&json).unwrap(), device);
    }
}
