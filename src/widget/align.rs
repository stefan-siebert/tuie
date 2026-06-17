//! Alignment primitives shared across layout-aware widgets.

use axis2d::Axis2D;

/// Content alignment along an axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Aligns to the start of the axis.
    Start,
    /// Aligns to the center of the axis.
    Center,
    /// Aligns to the end of the axis.
    End,
}

impl Align {
    /// Returns the alignment mirrored across the center.
    pub const fn flip(&self) -> Align {
        match self {
            Self::Start => Self::End,
            Self::End => Self::Start,
            Self::Center => Self::Center,
        }
    }
}

impl std::fmt::Display for Align {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start => write!(f, "Start"),
            Self::Center => write!(f, "Center"),
            Self::End => write!(f, "End"),
        }
    }
}

/// Per-axis sizing and positioning mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexAlign {
    /// Aligns the item to the start of the axis.
    Start,
    /// Aligns the item to the center of the axis.
    Center,
    /// Aligns the item to the end of the axis.
    End,
    /// Stretches the item to fill the axis.
    Stretch,
}

impl std::fmt::Display for FlexAlign {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start => write!(f, "Start"),
            Self::Center => write!(f, "Center"),
            Self::End => write!(f, "End"),
            Self::Stretch => write!(f, "Stretch"),
        }
    }
}

impl From<Align> for FlexAlign {
    fn from(a: Align) -> Self {
        match a {
            Align::Start => FlexAlign::Start,
            Align::Center => FlexAlign::Center,
            Align::End => FlexAlign::End,
        }
    }
}

/// How items overflow onto additional lines along the main axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlexWrap {
    /// Packs as many items as fit on each line before starting the next.
    #[default]
    Greedy,
    /// Distributes items across lines so each holds a similar amount.
    Balanced,
}

impl std::fmt::Display for FlexWrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Greedy => write!(f, "Greedy"),
            Self::Balanced => write!(f, "Balanced"),
        }
    }
}

/// Parent-side positioning and slack distribution along an axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[non_exhaustive]
pub enum Place {
    /// Pack items at the start of the axis.
    Start = 0,
    /// Pack items at the center of the axis.
    Center = 1,
    /// Pack items at the end of the axis.
    End = 2,
    /// Stretch each item to fill the axis.
    Stretch = 3,
    /// Distribute slack evenly around and between items.
    Evenly = 4,
    /// Push slack to the outsides so items spread apart.
    Apart = 5,
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start => write!(f, "Start"),
            Self::Center => write!(f, "Center"),
            Self::End => write!(f, "End"),
            Self::Stretch => write!(f, "Stretch"),
            Self::Evenly => write!(f, "Evenly"),
            Self::Apart => write!(f, "Apart"),
        }
    }
}

/// Error returned when an alignment conversion has no matching target variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlignConversionError;

impl std::fmt::Display for AlignConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "alignment value has no corresponding variant in target type")
    }
}

impl std::error::Error for AlignConversionError {}

impl TryFrom<Place> for FlexAlign {
    type Error = AlignConversionError;

    fn try_from(p: Place) -> Result<Self, Self::Error> {
        match p {
            Place::Start => Ok(FlexAlign::Start),
            Place::Center => Ok(FlexAlign::Center),
            Place::End => Ok(FlexAlign::End),
            Place::Stretch => Ok(FlexAlign::Stretch),
            Place::Evenly | Place::Apart => Err(AlignConversionError),
        }
    }
}

impl TryFrom<FlexAlign> for Align {
    type Error = AlignConversionError;

    fn try_from(f: FlexAlign) -> Result<Self, Self::Error> {
        match f {
            FlexAlign::Start => Ok(Align::Start),
            FlexAlign::Center => Ok(Align::Center),
            FlexAlign::End => Ok(Align::End),
            FlexAlign::Stretch => Err(AlignConversionError),
        }
    }
}

impl TryFrom<Place> for Align {
    type Error = AlignConversionError;

    fn try_from(p: Place) -> Result<Self, Self::Error> {
        match p {
            Place::Start => Ok(Align::Start),
            Place::Center => Ok(Align::Center),
            Place::End => Ok(Align::End),
            Place::Stretch | Place::Evenly | Place::Apart => Err(AlignConversionError),
        }
    }
}

impl From<FlexAlign> for Place {
    fn from(f: FlexAlign) -> Self {
        match f {
            FlexAlign::Start => Place::Start,
            FlexAlign::Center => Place::Center,
            FlexAlign::End => Place::End,
            FlexAlign::Stretch => Place::Stretch,
        }
    }
}

impl From<Align> for Place {
    fn from(a: Align) -> Self {
        match a {
            Align::Start => Place::Start,
            Align::Center => Place::Center,
            Align::End => Place::End,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AlignSpec(u8);

impl Default for AlignSpec {
    fn default() -> Self {
        Self::new()
            .place_x(Place::Stretch)
            .place_y(Place::Stretch)
    }
}

impl AlignSpec {
    const fn place_from_bits(b: u8) -> Place {
        match b {
            0 => Place::Start,
            1 => Place::Center,
            2 => Place::End,
            3 => Place::Stretch,
            4 => Place::Evenly,
            5 => Place::Apart,
            _ => Place::Start,
        }
    }

    /// Creates an empty spec with both axes set to [`Place::Start`].
    pub const fn new() -> Self {
        Self(0)
    }

    /// Returns the place along the x axis.
    pub const fn get_place_x(self) -> Place {
        Self::place_from_bits(self.0 & 0b111)
    }

    /// Returns the place along the y axis.
    pub const fn get_place_y(self) -> Place {
        Self::place_from_bits((self.0 >> 3) & 0b111)
    }

    /// Returns the place along `axis`.
    pub const fn get_place(self, axis: Axis2D) -> Place {
        match axis {
            Axis2D::X => self.get_place_x(),
            Axis2D::Y => self.get_place_y(),
        }
    }

    /// Returns a copy with the x place set to `p`.
    pub const fn place_x(self, p: Place) -> Self {
        Self((self.0 & !0b111) | (p as u8 & 0b111))
    }

    /// Returns a copy with the y place set to `p`.
    pub const fn place_y(self, p: Place) -> Self {
        Self((self.0 & !(0b111 << 3)) | ((p as u8 & 0b111) << 3))
    }
}

/// Child-side per-axis alignment override, where unset axes inherit from the parent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AlignOverride(u8);

impl AlignOverride {
    const fn mode_from_bits(b: u8) -> Option<FlexAlign> {
        match b {
            0 => None,
            1 => Some(FlexAlign::Start),
            2 => Some(FlexAlign::Center),
            3 => Some(FlexAlign::End),
            4 => Some(FlexAlign::Stretch),
            _ => None,
        }
    }

    const fn mode_to_bits(m: Option<FlexAlign>) -> u8 {
        match m {
            None => 0,
            Some(FlexAlign::Start) => 1,
            Some(FlexAlign::Center) => 2,
            Some(FlexAlign::End) => 3,
            Some(FlexAlign::Stretch) => 4,
        }
    }

    /// Creates an empty override that inherits both axes from the parent.
    pub const fn new() -> Self {
        Self(0)
    }

    /// Returns the x mode override, or `None` to inherit from the parent.
    pub const fn get_mode_x(self) -> Option<FlexAlign> {
        Self::mode_from_bits(self.0 & 0b111)
    }

    /// Returns the y mode override, or `None` to inherit from the parent.
    pub const fn get_mode_y(self) -> Option<FlexAlign> {
        Self::mode_from_bits((self.0 >> 3) & 0b111)
    }

    /// Returns the mode override along `axis`, or `None` to inherit.
    pub const fn get_mode(self, axis: Axis2D) -> Option<FlexAlign> {
        match axis {
            Axis2D::X => self.get_mode_x(),
            Axis2D::Y => self.get_mode_y(),
        }
    }

    /// Returns a copy with the x mode override set.
    pub const fn mode_x(self, m: Option<FlexAlign>) -> Self {
        Self((self.0 & !0b111) | Self::mode_to_bits(m))
    }

    /// Returns a copy with the y mode override set.
    pub const fn mode_y(self, m: Option<FlexAlign>) -> Self {
        Self((self.0 & !(0b111 << 3)) | (Self::mode_to_bits(m) << 3))
    }

    /// Returns a copy with the mode override along `axis` set.
    pub const fn mode(self, axis: Axis2D, m: Option<FlexAlign>) -> Self {
        match axis {
            Axis2D::X => self.mode_x(m),
            Axis2D::Y => self.mode_y(m),
        }
    }
}
