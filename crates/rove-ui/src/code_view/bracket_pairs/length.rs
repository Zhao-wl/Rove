//! 括号对树用的「长度」（#57）。
//!
//! 照搬 VS Code `bracketPairsTextModelPart/length.ts` 的核心性质：**节点存长度而不是绝对偏移**，
//! 这样一次编辑只会让覆盖编辑位置的那条根到叶路径上的节点重建，旁边的子树原样复用，
//! 不需要整体平移偏移量。
//!
//! 长度是 (行数差, 列数)，列是行内的 UTF-8 字节数。绝对位置也用同一个类型表示
//! （`lines` 是 0 起行号，`columns` 是行内字节列），两者的加减法规则相同：
//! 跨行时列数**重置**为后一段的列数，不跨行时列数相加。

/// 文档中的一段长度，或一个绝对位置。
///
/// 按 (行, 列) 字典序比较，所以 `Ord` 直接 derive 即可。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Length {
    /// 行数差（作为绝对位置时是 0 起行号）。
    pub lines: u64,
    /// 行内 UTF-8 字节列。
    pub columns: u32,
}

impl Length {
    pub const ZERO: Self = Self {
        lines: 0,
        columns: 0,
    };

    pub const fn new(lines: u64, columns: u32) -> Self {
        Self { lines, columns }
    }

    pub const fn is_zero(self) -> bool {
        self.lines == 0 && self.columns == 0
    }

    /// 首尾相接：走完 `self` 之后再走 `other` 这么长。
    #[must_use]
    pub const fn add(self, other: Self) -> Self {
        if other.lines == 0 {
            Self {
                lines: self.lines,
                columns: self.columns.saturating_add(other.columns),
            }
        } else {
            Self {
                lines: self.lines.saturating_add(other.lines),
                columns: other.columns,
            }
        }
    }

    /// 从 `self` 走到 `bigger` 的长度；`bigger < self` 时返回 [`Length::ZERO`]。
    #[must_use]
    pub const fn to(self, bigger: Self) -> Self {
        if self.lines == bigger.lines {
            if bigger.columns < self.columns {
                Self::ZERO
            } else {
                Self {
                    lines: 0,
                    columns: bigger.columns - self.columns,
                }
            }
        } else if bigger.lines < self.lines {
            Self::ZERO
        } else {
            Self {
                lines: bigger.lines - self.lines,
                columns: bigger.columns,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_resets_columns_across_lines() {
        let a = Length::new(0, 5);
        assert_eq!(a.add(Length::new(0, 3)), Length::new(0, 8));
        // 跨行：列数由后一段决定。
        assert_eq!(a.add(Length::new(2, 3)), Length::new(2, 3));
        assert_eq!(Length::new(4, 7).add(Length::new(1, 0)), Length::new(5, 0));
        assert_eq!(Length::ZERO.add(Length::ZERO), Length::ZERO);
    }

    #[test]
    fn difference_is_the_inverse_of_add() {
        for (a, b) in [
            (Length::new(0, 2), Length::new(0, 9)),
            (Length::new(3, 4), Length::new(7, 1)),
            (Length::new(3, 4), Length::new(3, 4)),
        ] {
            assert_eq!(a.add(a.to(b)), b, "{a:?} → {b:?}");
        }
        // 反向取差为零，不会下溢。
        assert_eq!(Length::new(5, 0).to(Length::new(1, 2)), Length::ZERO);
        assert_eq!(Length::new(5, 9).to(Length::new(5, 2)), Length::ZERO);
    }

    #[test]
    fn ordering_is_lexicographic() {
        assert!(Length::new(0, 9) < Length::new(1, 0));
        assert!(Length::new(1, 0) < Length::new(1, 1));
        assert!(Length::ZERO.is_zero());
    }
}
