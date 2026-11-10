use crate::documents::EditorGroupId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Axis {
    Horizontal,
    Vertical,
}

impl Direction {
    pub(crate) fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Horizontal,
            Self::Up | Self::Down => Axis::Vertical,
        }
    }

    fn before(self) -> bool {
        matches!(self, Self::Left | Self::Up)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Layout {
    Group(EditorGroupId),
    Split {
        axis: Axis,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

#[derive(Clone, Copy)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Layout {
    pub(crate) fn contains(&self, group: EditorGroupId) -> bool {
        match self {
            Self::Group(id) => *id == group,
            Self::Split { first, second, .. } => first.contains(group) || second.contains(group),
        }
    }

    pub(crate) fn split(
        &mut self,
        anchor: EditorGroupId,
        id: EditorGroupId,
        direction: Direction,
    ) -> bool {
        match self {
            Self::Group(group) if *group == anchor => {
                let original = Self::Group(anchor);
                let new = Self::Group(id);
                let (first, second) = if direction.before() {
                    (new, original)
                } else {
                    (original, new)
                };
                *self = Self::Split {
                    axis: direction.axis(),
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            Self::Group(_) => false,
            Self::Split { first, second, .. } => {
                first.split(anchor, id, direction) || second.split(anchor, id, direction)
            }
        }
    }

    pub(crate) fn remove(&mut self, id: EditorGroupId) {
        if let Self::Split { first, second, .. } = self {
            if matches!(first.as_ref(), Self::Group(group) if *group == id) {
                *self = (**second).clone();
            } else if matches!(second.as_ref(), Self::Group(group) if *group == id) {
                *self = (**first).clone();
            } else {
                first.remove(id);
                second.remove(id);
            }
        }
    }

    pub(crate) fn swap(&mut self, left: EditorGroupId, right: EditorGroupId) {
        match self {
            Self::Group(id) => {
                if *id == left {
                    *id = right;
                } else if *id == right {
                    *id = left;
                }
            }
            Self::Split { first, second, .. } => {
                first.swap(left, right);
                second.swap(left, right);
            }
        }
    }

    pub(crate) fn transpose(&mut self) {
        if let Self::Split {
            axis,
            first,
            second,
        } = self
        {
            *axis = match axis {
                Axis::Horizontal => Axis::Vertical,
                Axis::Vertical => Axis::Horizontal,
            };
            first.transpose();
            second.transpose();
        }
    }

    pub(crate) fn parent_axis(&self, id: EditorGroupId) -> Option<Axis> {
        match self {
            Self::Group(_) => None,
            Self::Split {
                axis,
                first,
                second,
            } => {
                if [first.as_ref(), second.as_ref()]
                    .iter()
                    .any(|child| matches!(child, Self::Group(group) if *group == id))
                {
                    Some(*axis)
                } else {
                    first.parent_axis(id).or_else(|| second.parent_axis(id))
                }
            }
        }
    }

    fn visit(&self, rect: Rect, visit: &mut impl FnMut(EditorGroupId, Rect)) {
        match self {
            Self::Group(id) => visit(*id, rect),
            Self::Split {
                axis,
                first,
                second,
            } => {
                let (a, b) = match axis {
                    Axis::Horizontal => {
                        let w = rect.w / 2.0;
                        (
                            Rect { w, ..rect },
                            Rect {
                                x: rect.x + w,
                                w,
                                ..rect
                            },
                        )
                    }
                    Axis::Vertical => {
                        let h = rect.h / 2.0;
                        (
                            Rect { h, ..rect },
                            Rect {
                                y: rect.y + h,
                                h,
                                ..rect
                            },
                        )
                    }
                };
                first.visit(a, visit);
                second.visit(b, visit);
            }
        }
    }

    fn bounds(&self) -> Vec<(EditorGroupId, Rect)> {
        let mut leaves = Vec::new();
        self.visit(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            &mut |id, rect| leaves.push((id, rect)),
        );
        leaves
    }

    pub(crate) fn order(&self) -> Vec<EditorGroupId> {
        let mut leaves = self.bounds();
        leaves.sort_by(|(_, a), (_, b)| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
        leaves.into_iter().map(|(id, _)| id).collect()
    }

    pub(crate) fn neighbor(
        &self,
        group: EditorGroupId,
        direction: Direction,
    ) -> Option<EditorGroupId> {
        let leaves = self.bounds();
        let (_, anchor) = leaves.iter().find(|(id, _)| *id == group)?;
        let along = |rect: Rect| match direction {
            Direction::Left | Direction::Right => (rect.x, rect.w, rect.y, rect.h),
            Direction::Up | Direction::Down => (rect.y, rect.h, rect.x, rect.w),
        };
        let (start, len, cross, cross_len) = along(*anchor);
        leaves
            .into_iter()
            .filter_map(|(id, rect)| {
                let (other, other_len, other_cross, other_cross_len) = along(rect);
                let distance = if direction.before() {
                    start - other - other_len
                } else {
                    other - start - len
                };
                let overlap =
                    (cross + cross_len).min(other_cross + other_cross_len) - cross.max(other_cross);
                if id == group || distance < 0.0 || overlap <= 0.0 {
                    return None;
                }
                let shift = (cross + cross_len / 2.0 - other_cross - other_cross_len / 2.0).abs();
                Some((id, distance, shift))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)))
            .map(|(id, _, _)| id)
    }
}
