//! Border, padding, and titles drawn around a widget's content area.

use crate::prelude::*;

/// Title placed on a [`Chrome`] border at a given edge and alignment.
pub struct ChromeTitle {
    /// Title text drawn on the border.
    pub text: StyledString,
    /// Border edge the title sits on.
    pub edge: VerticalEdge,
    /// Horizontal alignment of the title along its edge.
    pub align: Align,
}

impl ChromeTitle {
    /// Creates a title from text, edge, and alignment.
    pub fn new(text: impl Into<StyledString>, edge: VerticalEdge, align: Align) -> Self {
        Self { text: text.into(), edge, align }
    }
}

/// Border, padding, and titles drawn around a widget's content area.
pub struct Chrome {
    /// Whether a border is drawn around the content area.
    pub bordered: bool,
    /// Glyph set used for the border, or `None` to use the global default.
    pub border: Option<&'static Border>,
    /// Style layered over the global default for border glyphs. Empty fields inherit from the config.
    pub border_style: Style,
    /// Inner padding inserted between the border and the content area.
    pub padding: Spacing,
    /// Titles drawn on the border.
    pub titles: Vec<ChromeTitle>,
}

impl Default for Chrome {
    fn default() -> Self { Self::new() }
}

impl Chrome {
    /// Creates empty chrome with no border, no padding, and no titles.
    pub fn new() -> Self {
        Self {
            bordered: false,
            border: None,
            border_style: Style::new(),
            padding: Spacing::new(),
            titles: Vec::new(),
        }
    }

    /// Returns the cell width consumed by the border on each side.
    pub fn get_border_size(&self) -> u16 {
        u16::from(self.bordered)
    }

    /// Returns the configured [`Border`] or falls back to the global default.
    pub fn get_resolved_border(&self) -> &'static Border {
        self.border.unwrap_or_else(|| crate::render::border::config::get().border)
    }

    /// Returns the resolved border [`Style`] with per-instance overrides applied.
    pub fn get_resolved_border_style(&self) -> Style {
        crate::render::border::config::get().style.apply(self.border_style)
    }

    /// Returns the top-left title text, if any.
    pub fn get_title(&self) -> Option<&str> {
        self.get_title_at(VerticalEdge::Top, Align::Start)
    }

    /// Sets or clears the top-left title.
    pub fn set_title(&mut self, title: Option<StyledString>) {
        self.set_title_at(VerticalEdge::Top, Align::Start, title);
    }

    /// Returns the title text at `edge` and `align`, if any.
    pub fn get_title_at(&self, edge: VerticalEdge, align: Align) -> Option<&str> {
        self.titles.iter()
            .find(|t| t.edge == edge && t.align == align)
            .map(|t| t.text.as_str())
    }

    /// Sets or clears the title at `edge` and `align`.
    pub fn set_title_at(&mut self, edge: VerticalEdge, align: Align, text: Option<StyledString>) {
        self.titles.retain(|t| t.edge != edge || t.align != align);
        if let Some(text) = text {
            self.titles.push(ChromeTitle::new(text, edge, align));
        }
    }

    /// Draws the border and titles into `ctx`.
    pub fn render(&self, ctx: &mut crate::render::RenderContext) {
        if !self.bordered {
            return;
        }

        let border = self.get_resolved_border();
        let box_size = ctx.size;
        ctx.set_style(self.get_resolved_border_style());
        ctx.border(box_size, border);

        let separator = border.get_edge(Axis2D::Y);
        let inner_w = box_size.x.saturating_sub(2) as usize;

        for title in &self.titles {
            if title.text.is_empty() || inner_w < 3 {
                continue;
            }

            let decorated = format!("{} {} ", separator, title.text.as_str());
            let decorated_w = tuie::display_width(&decorated);
            let slack = inner_w.saturating_sub(decorated_w);

            let x: i32 = 1 + match title.align {
                Align::Start => 0,
                Align::Center => (slack / 2) as i32,
                Align::End => slack as i32,
            };
            let y: i32 = match title.edge {
                VerticalEdge::Top => 0,
                VerticalEdge::Bottom => box_size.y as i32 - 1,
            };

            ctx.move_to(Vec2::new(x, y));
            let region_w = (inner_w + 1).saturating_sub(x as usize) as u16;
            let mut region = ctx.region(Vec2::new(region_w, 1));
            write!(region, "{} ", separator);
            for (chunk, style) in title.text.iter_chunks(..) {
                region.set_style(style);
                region.write(chunk);
            }
            region.set_style(Style::new());
            region.write(" ");
        }
    }
}

/// Shared chrome accessors for widgets with an optional [`Chrome`] and a [`Spacing`] inset.
pub(crate) trait ChromeHost: WidgetMethods {
    fn get_chrome(&self) -> Option<&Chrome>;
    fn get_chrome_mut(&mut self) -> &mut Chrome;
    fn get_insets(&self) -> Spacing;

    fn get_inset_before(&self, a: Axis2D) -> u16 {
        self.get_insets().get_before(a) as u16
    }

    fn get_inset_after(&self, a: Axis2D) -> u16 {
        self.get_insets().get_after(a) as u16
    }

    fn get_border_cells(&self) -> u16 {
        match self.get_chrome() {
            Some(c) => c.get_border_size(),
            None => 0,
        }
    }

    fn get_padding_total(&self) -> Vec2<u16> {
        match self.get_chrome() {
            Some(c) => c.padding.get_total(),
            None => Vec2::of(0),
        }
    }

    fn get_padding_before(&self, a: Axis2D) -> u16 {
        match self.get_chrome() {
            Some(c) => c.padding.get_before(a) as u16,
            None => 0,
        }
    }

    fn get_padding_after(&self, a: Axis2D) -> u16 {
        match self.get_chrome() {
            Some(c) => c.padding.get_after(a) as u16,
            None => 0,
        }
    }

    fn get_border_offset(&self) -> Vec2<i32> {
        Vec2::of(self.get_border_cells() as i32)
    }

    fn get_chrome_before(&self) -> Vec2<u16> {
        let border = self.get_border_cells();
        Axis2D::map(|a| border + self.get_padding_before(a))
    }

    fn get_chrome_total(&self) -> Vec2<u16> {
        let borders = self.get_border_cells() * 2;
        let padding = self.get_padding_total();
        Axis2D::map(|a| borders + padding[a])
    }

    fn border_did_change(&mut self) {
        let c = self.get_chrome_mut();
        let want = c.border.is_some();
        if c.bordered != want {
            c.bordered = want;
            self.dirty_layout();
        } else {
            self.dirty_paint();
        }
    }
}
