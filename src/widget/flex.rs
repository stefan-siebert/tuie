//! Main-axis flex grow/shrink solver.

#[derive(Clone, Copy, PartialEq, Eq)]
struct Flexibility(u8);

impl Flexibility {
    const RIGID:      Self = Self(0);
    const GROWABLE:   Self = Self(1);
    const SHRINKABLE: Self = Self(2);
    const FLEXIBLE:   Self = Self(Self::GROWABLE.0 | Self::SHRINKABLE.0);

    const fn is_locked(self) -> bool {
        self.0 == Self::RIGID.0
    }
}

/// Per-item input and output for [`resolve`].
#[derive(Clone, Copy)]
pub struct FlexItem {
    /// Preferred size along the main axis before flex distribution.
    pub basis: u16,
    /// Minimum size along the main axis.
    pub min: u16,
    /// Maximum size along the main axis.
    pub max: u16,
    /// Relative weight used when distributing extra space.
    pub grow: u8,
    /// Resolved size along the main axis, written by [`resolve`].
    pub target: u16,
    flex: Flexibility,
}

impl FlexItem {
    const FLEX_SHRINK: u32 = 1;

    /// Creates an item ready for [`resolve`] with `target` initialized to zero.
    pub const fn new(basis: u16, min: u16, max: u16, grow: u8) -> Self {
        Self {
            basis,
            min,
            max,
            grow,
            target: 0,
            flex: Flexibility::FLEXIBLE,
        }
    }
}

/// Resolves main-axis sizes for a slice of flex items.
pub fn resolve(items: &mut [FlexItem], container_main: u16, gap_total: u32) {
    let n = items.len();
    if n == 0 {
        return;
    }

    let mut total_hyp: u32 = 0;
    for it in items.iter() {
        total_hyp = total_hyp.saturating_add(it.basis.clamp(it.min, it.max) as u32);
    }
    let grow = total_hyp.saturating_add(gap_total) < container_main as u32;

    for it in items.iter_mut() {
        let hypothetical = it.basis.clamp(it.min, it.max);
        let freeze_initial = if grow {
            it.grow == 0 || it.basis > hypothetical
        } else {
            it.basis < hypothetical
        };
        if freeze_initial {
            it.target = hypothetical;
            it.flex = Flexibility::RIGID;
        } else {
            it.target = it.basis;
            it.flex = Flexibility::FLEXIBLE;
        }
    }

    loop {
        let mut locked_total: i64 = 0;
        let mut open_basis_total: i64 = 0;
        let mut total_weight: i64 = 0;
        let mut any_open = false;
        for it in items.iter() {
            if it.flex.is_locked() {
                locked_total += it.target as i64;
            } else {
                any_open = true;
                open_basis_total += it.basis as i64;
                total_weight += if grow {
                    it.grow as i64
                } else {
                    FlexItem::FLEX_SHRINK as i64 * it.basis as i64
                };
            }
        }
        if !any_open {
            break;
        }

        let free = container_main as i64 - locked_total - open_basis_total - gap_total as i64;
        let dir: i64 = if grow {
            1
        } else {
            -1
        };
        let amount = free * dir;

        if total_weight <= 0 || amount <= 0 {
            for it in items.iter_mut() {
                if !it.flex.is_locked() {
                    it.target = it.basis;
                    it.flex = Flexibility::RIGID;
                }
            }
            continue;
        }

        let mut assigned: i64 = 0;
        let mut cum_w: i64 = 0;
        let mut violation: i64 = 0;
        for it in items.iter_mut() {
            if it.flex.is_locked() {
                continue;
            }
            let w = if grow {
                it.grow as i64
            } else {
                FlexItem::FLEX_SHRINK as i64 * it.basis as i64
            };
            cum_w += w;
            let running_total = (amount * cum_w + total_weight / 2) / total_weight;
            let share = running_total - assigned;
            assigned = running_total;

            let unclamped: i32 = it.basis as i32 + (dir * share) as i32;
            let clamped = unclamped.clamp(it.min as i32, it.max as i32);
            let delta = (clamped - unclamped) as i64;
            it.target = clamped as u16;
            it.flex = if delta > 0 {
                Flexibility::GROWABLE
            } else if delta < 0 {
                Flexibility::SHRINKABLE
            } else {
                Flexibility::FLEXIBLE
            };
            violation += delta;
        }

        if violation == 0 {
            for it in items.iter_mut() {
                it.flex = Flexibility::RIGID;
            }
        } else {
            let dominant = if violation > 0 {
                Flexibility::GROWABLE
            } else {
                Flexibility::SHRINKABLE
            };
            for it in items.iter_mut() {
                if it.flex == dominant {
                    it.flex = Flexibility::RIGID;
                }
            }
        }
    }
}
