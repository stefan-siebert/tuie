//! Clipboard access and providers.

use std::cell::RefCell;

thread_local! {
    static CLIPBOARD: RefCell<Box<dyn Clipboard>> =
        RefCell::new(Box::new(LocalClipboard::new()));
}

/// Installs `provider` as the active clipboard.
pub fn set_provider(provider: impl Clipboard + 'static) {
    CLIPBOARD.with_borrow_mut(|c| *c = Box::new(provider))
}

/// Writes `items` to the clipboard.
pub fn write(items: impl Into<Box<[ClipboardItem]>>) {
    CLIPBOARD.with_borrow_mut(|c| c.write(items.into()))
}

/// Writes `text` to the clipboard as a single [`ClipboardItem::Text`].
pub fn write_string(text: impl Into<String>) {
    write(ClipboardItem::Text(text.into()))
}

/// Returns a clone of the current clipboard contents.
pub fn read() -> Box<[ClipboardItem]> {
    CLIPBOARD.with_borrow(|c| c.read().to_vec().into_boxed_slice())
}

/// Reads the clipboard as a UTF-8 string when available.
pub fn read_string() -> Option<String> {
    CLIPBOARD.with_borrow(|c| c.read_string())
}

/// Clears the active clipboard.
pub fn clear() {
    CLIPBOARD.with_borrow_mut(|c| c.clear())
}

/// Single entry stored on a [`Clipboard`].
#[non_exhaustive]
#[derive(Clone)]
pub enum ClipboardItem {
    /// Plain-text item.
    Text(String),
}

impl From<ClipboardItem> for Box<[ClipboardItem]> {
    fn from(item: ClipboardItem) -> Self {
        Box::new([item])
    }
}

/// Backing store for cut, copy, and paste operations.
pub trait Clipboard {
    /// Returns the current clipboard contents.
    fn read(&self) -> &[ClipboardItem];
    /// Replaces the clipboard contents with `items`.
    fn write(&mut self, items: Box<[ClipboardItem]>);

    /// Writes `text` as a single [`ClipboardItem::Text`].
    fn write_string(&mut self, text: String) {
        self.write(ClipboardItem::Text(text).into());
    }

    /// Returns clipboard text items joined by newlines, or `None` when empty.
    fn read_string(&self) -> Option<String> {
        let items = self.read();
        if items.is_empty() {
            return None;
        }
        let total_text_len = items
            .iter()
            .map(|item| match item {
                ClipboardItem::Text(s) => s.len(),
            })
            .sum::<usize>();
        let mut joined = String::with_capacity(total_text_len + items.len() - 1);
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                joined.push('\n');
            }
            match item {
                ClipboardItem::Text(s) => joined.push_str(s),
            }
        }
        Some(joined)
    }

    /// Removes all items.
    fn clear(&mut self) {
        self.write(Box::new([]));
    }
}

/// In-memory [`Clipboard`] implementation.
pub struct LocalClipboard(Box<[ClipboardItem]>);

impl Default for LocalClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalClipboard {
    /// Creates an empty in-memory clipboard.
    pub fn new() -> Self {
        Self(Box::new([]))
    }
}

impl Clipboard for LocalClipboard {
    fn read(&self) -> &[ClipboardItem] {
        &self.0
    }
    fn write(&mut self, items: Box<[ClipboardItem]>) {
        self.0 = items;
    }
}
