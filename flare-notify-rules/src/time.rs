//! 时间工具：「从未发生」的哨兵时刻、饱和差值、设备当地的「一周中的分钟」。
//!
//! 规则不读时钟。所有时刻都是调用方传入的 Unix 毫秒；勿扰时段要的当地时间也由调用方
//! 换算好再传进来，这样服务端（按绑定登记的时区）与引擎（按系统时区）得到同样的判断，
//! 而且规则本身不需要时区库。

/// 「从未发生」的时刻。
///
/// 用 `i64::MIN` 而不是 `Option`：与任何时刻的差值经 [`since`] 饱和到 `i64::MAX`，
/// 「距上次已过多久」的判断对从未发生的事件天然成立，判断处不必再分支，编码也只占 8 字节。
pub(crate) const NEVER: i64 = i64::MIN;

/// `now` 距 `then` 过了多久（毫秒）。`then` 为 [`NEVER`] 时为 `i64::MAX`。
///
/// 时钟回拨时结果为负，各处都按「间隔还没到」处理：宁可少发一次，也不在回拨时连发。
pub(crate) const fn since(now: i64, then: i64) -> i64 {
    now.saturating_sub(then)
}

const MINUTES_PER_DAY: u16 = 1_440;
const DAY_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;

/// 设备当地时间在一周中的位置：周一 00:00 为 0，周日 23:59 为 10079。
///
/// 星期从周一起算，与协议 `QuietHours.weekdays` 的位序（bit0 周一 … bit6 周日）一致。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MinuteOfWeek(u16);

impl MinuteOfWeek {
    /// 一天的分钟数。
    pub const MINUTES_PER_DAY: u16 = MINUTES_PER_DAY;
    /// 一周的分钟数。
    pub const MINUTES_PER_WEEK: u16 = 7 * MINUTES_PER_DAY;

    /// 由星期（0 = 周一 … 6 = 周日）和当天的分钟（0..1440）构造；越界返回 `None`。
    pub const fn new(weekday: u8, minute_of_day: u16) -> Option<Self> {
        if weekday > 6 || minute_of_day >= MINUTES_PER_DAY {
            return None;
        }
        Some(Self(weekday as u16 * MINUTES_PER_DAY + minute_of_day))
    }

    /// 由一周中的分钟序号（0..10080）构造；越界返回 `None`。
    pub const fn from_index(minute_of_week: u16) -> Option<Self> {
        if minute_of_week >= Self::MINUTES_PER_WEEK {
            return None;
        }
        Some(Self(minute_of_week))
    }

    /// 由「当地墙上时间」构造：把当地日期时间当作 UTC 写成的 Unix 毫秒。
    ///
    /// 1970-01-01 是周四，所以天数加 3 再对 7 取模得到以周一为 0 的星期。
    /// 用欧几里得除法，1970 年以前的时刻也落在正确的星期上。
    pub const fn from_local_ms(local_ms: i64) -> Self {
        let day = local_ms.div_euclid(DAY_MS);
        let weekday = (day + 3).rem_euclid(7);
        let minute = local_ms.rem_euclid(DAY_MS) / MINUTE_MS;
        Self((weekday * MINUTES_PER_DAY as i64 + minute) as u16)
    }

    /// 由 UTC 时刻与该时刻的 UTC 偏移（分钟，东八区为 480）构造。
    ///
    /// 偏移由调用方按时区规则（含夏令时）算出；规则只做加法，不认识时区名。
    pub const fn from_utc_ms(utc_ms: i64, utc_offset_minutes: i32) -> Self {
        Self::from_local_ms(utc_ms.saturating_add(utc_offset_minutes as i64 * MINUTE_MS))
    }

    /// 一周中的分钟序号（0..10080）。
    pub const fn get(self) -> u16 {
        self.0
    }

    /// 星期：0 = 周一 … 6 = 周日。
    pub const fn weekday(self) -> u8 {
        (self.0 / MINUTES_PER_DAY) as u8
    }

    /// 当天的分钟（0..1440）。
    pub const fn minute_of_day(self) -> u16 {
        self.0 % MINUTES_PER_DAY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_saturates_for_never_and_goes_negative_on_clock_skew() {
        assert_eq!(since(0, NEVER), i64::MAX);
        assert_eq!(since(-5, NEVER), i64::MAX - 4);
        assert_eq!(since(1_000, 400), 600);
        assert_eq!(since(400, 1_000), -600);
    }

    #[test]
    fn epoch_is_thursday_midnight() {
        let at = MinuteOfWeek::from_local_ms(0);
        assert_eq!(at.weekday(), 3);
        assert_eq!(at.minute_of_day(), 0);
        assert_eq!(at.get(), 3 * 1_440);
    }

    #[test]
    fn first_monday_after_epoch_is_zero() {
        // 1970-01-05 00:00 是周一。
        assert_eq!(MinuteOfWeek::from_local_ms(4 * DAY_MS).get(), 0);
        // 1970-01-11 23:59 是周日最后一分钟。
        let sunday_end = 10 * DAY_MS + 23 * 3_600_000 + 59 * MINUTE_MS + 59_999;
        let at = MinuteOfWeek::from_local_ms(sunday_end);
        assert_eq!(at.weekday(), 6);
        assert_eq!(at.minute_of_day(), 1_439);
        assert_eq!(at.get(), MinuteOfWeek::MINUTES_PER_WEEK - 1);
    }

    #[test]
    fn negative_local_times_use_euclidean_division() {
        // 1969-12-31 23:59 是周三。
        let at = MinuteOfWeek::from_local_ms(-MINUTE_MS);
        assert_eq!(at.weekday(), 2);
        assert_eq!(at.minute_of_day(), 1_439);
    }

    #[test]
    fn utc_offset_shifts_into_local_day() {
        // 2023-11-14T16:30:00Z 是周二；东八区当地为周三 00:30。
        let utc_ms = 1_699_979_400_000;
        assert_eq!(MinuteOfWeek::from_utc_ms(utc_ms, 0).weekday(), 1);
        let local = MinuteOfWeek::from_utc_ms(utc_ms, 480);
        assert_eq!(local.weekday(), 2);
        assert_eq!(local.minute_of_day(), 30);
        // 西五区当地为周二 11:30。
        let west = MinuteOfWeek::from_utc_ms(utc_ms, -300);
        assert_eq!(west.weekday(), 1);
        assert_eq!(west.minute_of_day(), 11 * 60 + 30);
    }

    #[test]
    fn constructors_reject_out_of_range_values() {
        assert_eq!(MinuteOfWeek::new(7, 0), None);
        assert_eq!(MinuteOfWeek::new(0, 1_440), None);
        assert_eq!(
            MinuteOfWeek::new(6, 1_439).map(MinuteOfWeek::get),
            Some(10_079)
        );
        assert_eq!(MinuteOfWeek::from_index(10_080), None);
        assert_eq!(
            MinuteOfWeek::from_index(10_079),
            MinuteOfWeek::new(6, 1_439)
        );
    }
}
