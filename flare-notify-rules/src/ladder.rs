//! 自适应间隔阶梯。
//!
//! 间隔不是固定值：消息来得比当前间隔快（有消息被吞并）就升一档，一个完整间隔内
//! 没有新消息就降一档。稀疏的对话因此每条都能提示，刷屏的会话迅速收敛到最长间隔。
//! 档位只占一个字节，状态里存的是档位而不是间隔本身，参数调整后旧状态仍然有效。

/// 阶梯最多的档数。
const MAX_STEPS: usize = 8;

/// 一条非递减的间隔阶梯（毫秒），档位从 0 开始。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ladder {
    steps_ms: [u32; MAX_STEPS],
    len: u8,
}

impl Ladder {
    /// 阶梯最多的档数。
    pub const MAX_STEPS: usize = MAX_STEPS;

    /// 由各档间隔构造。为空、超过 [`Ladder::MAX_STEPS`] 档、含 0 或出现递减时返回 `None`：
    /// 递减的阶梯会让「升档」反而更吵，0 间隔等于不限频。
    pub const fn new(steps_ms: &[u32]) -> Option<Self> {
        if steps_ms.is_empty() || steps_ms.len() > MAX_STEPS {
            return None;
        }
        let mut steps = [0u32; MAX_STEPS];
        let mut i = 0;
        while i < steps_ms.len() {
            let step = steps_ms[i];
            if step == 0 || (i > 0 && step < steps_ms[i - 1]) {
                return None;
            }
            steps[i] = step;
            i += 1;
        }
        Some(Self {
            steps_ms: steps,
            len: steps_ms.len() as u8,
        })
    }

    /// 编译期常量用：非法阶梯直接让编译失败。
    pub(crate) const fn must(steps_ms: &[u32]) -> Self {
        match Self::new(steps_ms) {
            Some(ladder) => ladder,
            None => panic!("invalid built-in ladder"),
        }
    }

    /// 各档间隔（毫秒）。
    pub fn steps_ms(&self) -> &[u32] {
        &self.steps_ms[..usize::from(self.len)]
    }

    /// 最高档位。
    pub const fn top(&self) -> u8 {
        self.len - 1
    }

    /// 某一档的间隔（毫秒）。档位越界按最高档算：参数缩短后，旧状态里偏高的档位仍然可用。
    pub fn at(&self, level: u8) -> i64 {
        i64::from(self.steps_ms[usize::from(level.min(self.top()))])
    }

    /// 升一档，封顶。
    pub(crate) fn up(&self, level: u8) -> u8 {
        level.saturating_add(1).min(self.top())
    }

    /// 空闲 `idle_ms` 之后的档位：每经过当前档一个完整间隔就降一档，直到 0 或剩余时间不够。
    ///
    /// 逐档扣减而不是一次跳回 0：刚停下来的刷屏会话再来消息时仍然收得紧，
    /// 安静得足够久才完全放开。
    pub(crate) fn down_after_idle(&self, level: u8, idle_ms: i64) -> u8 {
        let mut level = level.min(self.top());
        let mut rest = idle_ms;
        while level > 0 {
            let step = self.at(level);
            if rest < step {
                break;
            }
            rest -= step;
            level -= 1;
        }
        level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_zero_decreasing_and_too_long_ladders() {
        assert_eq!(Ladder::new(&[]), None);
        assert_eq!(Ladder::new(&[0, 5]), None);
        assert_eq!(Ladder::new(&[5, 4]), None);
        assert_eq!(Ladder::new(&[1; 9]), None);
        assert!(Ladder::new(&[1; 8]).is_some());
        assert!(Ladder::new(&[2, 2, 3]).is_some());
    }

    #[test]
    fn at_clamps_to_the_top_step() {
        let ladder = Ladder::must(&[30_000, 120_000, 600_000]);
        assert_eq!(ladder.steps_ms(), &[30_000, 120_000, 600_000]);
        assert_eq!(ladder.top(), 2);
        assert_eq!(ladder.at(0), 30_000);
        assert_eq!(ladder.at(2), 600_000);
        assert_eq!(ladder.at(200), 600_000);
    }

    #[test]
    fn up_saturates_at_the_top() {
        let ladder = Ladder::must(&[1, 2, 3]);
        assert_eq!(ladder.up(0), 1);
        assert_eq!(ladder.up(2), 2);
        assert_eq!(ladder.up(u8::MAX), 2);
    }

    #[test]
    fn down_after_idle_spends_one_full_interval_per_step() {
        let ladder = Ladder::must(&[2_000, 5_000, 15_000, 60_000, 300_000]);
        // 不满最高档一个间隔：不降。
        assert_eq!(ladder.down_after_idle(4, 299_999), 4);
        // 满 300 秒降到 60 秒档，再满 60 秒降到 15 秒档。
        assert_eq!(ladder.down_after_idle(4, 300_000), 3);
        assert_eq!(ladder.down_after_idle(4, 359_999), 3);
        assert_eq!(ladder.down_after_idle(4, 360_000), 2);
        // 300 + 60 + 15 + 5 = 380 秒回到 0 档。
        assert_eq!(ladder.down_after_idle(4, 379_999), 1);
        assert_eq!(ladder.down_after_idle(4, 380_000), 0);
        // 负的空闲（时钟回拨）不降档。
        assert_eq!(ladder.down_after_idle(3, -1), 3);
        // 越界档位先按最高档算。
        assert_eq!(ladder.down_after_idle(9, 0), 4);
    }
}
