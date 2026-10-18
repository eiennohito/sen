//! Bounded history ranges and reading anchors, independent of Slint and transport.
//! Geometry is reported on layout changes, never on every scroll/frame.
use std::{collections::HashMap, ops::Range};

pub(crate) const UI_ROWS: usize = 200;
pub(crate) const RETAINED_ROWS: usize = 600;
pub(crate) const RETAINED_BYTES: usize = 4 * 1024 * 1024;
const EDGE_ROWS: usize = 40;
const SHIFT_ROWS: usize = 80;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Anchor {
    pub id: String,
    pub screen_y: f32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cursor {
    pub message: String,
    pub delivered_time: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Older,
    Newer,
}
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub cursor: Cursor,
    pub direction: Direction,
}
#[derive(Clone, Debug)]
pub(crate) struct Page {
    pub first_order: String,
    pub last_order: String,
    pub cursor: Option<Cursor>,
}
#[derive(Default, Clone)]
pub(crate) struct HistoryWindow {
    pub anchor: Option<Anchor>,
    pub return_anchor: Option<Anchor>,
    pub request: Option<Request>,
    pub failed: Option<Request>,
    pub reached_beginning: bool,
    pub newer_evicted: bool,
    start_id: Option<String>,
    measurements: HashMap<String, (f32, f32)>,
    pages: Vec<Page>,
}
impl HistoryWindow {
    pub fn range(&mut self, ids: &[String], following: bool) -> Range<usize> {
        let start = if ids.len() <= UI_ROWS {
            0
        } else if following {
            ids.len().saturating_sub(UI_ROWS)
        } else {
            self.start_id
                .as_ref()
                .and_then(|id| ids.iter().position(|x| x == id))
                .unwrap_or_else(|| ids.len().saturating_sub(UI_ROWS))
        };
        let end = (start + UI_ROWS).min(ids.len());
        self.start_id = ids.get(start).cloned();
        self.measurements
            .retain(|id, _| ids[start..end].contains(id));
        start..end
    }
    /// Select the first exposed row from cached layout coordinates.
    pub fn capture(&mut self, offset: f32) -> Option<Anchor> {
        self.anchor = self
            .measurements
            .iter()
            .filter(|(_, (y, h))| *y + *h + offset > 0.)
            .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
            .map(|(id, (y, _))| Anchor {
                id: id.clone(),
                screen_y: *y + offset,
            });
        self.anchor.clone()
    }
    /// Return a new pane offset only when the one anchored row changes layout.
    pub fn measured(&mut self, id: &str, y: f32, height: f32, following: bool) -> Option<f32> {
        self.measurements.insert(id.into(), (y, height));
        if following {
            return None;
        }
        self.anchor
            .as_ref()
            .filter(|anchor| anchor.id == id)
            .map(|anchor| anchor.screen_y - y)
    }
    pub fn exposed(&self, offset: f32, viewport: f32) -> Vec<String> {
        let mut rows: Vec<_> = self
            .measurements
            .iter()
            .filter(|(_, (y, _))| *y + offset >= 0. && *y + offset < viewport)
            .collect();
        rows.sort_by(|a, b| a.1.0.total_cmp(&b.1.0));
        rows.into_iter().map(|(id, _)| id.clone()).collect()
    }
    pub fn offset_for(&self, id: &str, screen_y: f32) -> Option<f32> {
        self.measurements.get(id).map(|(y, _)| screen_y - y)
    }
    pub fn reset_latest(&mut self) {
        self.start_id = None;
        self.newer_evicted = false;
        self.reached_beginning = false;
        self.request = None;
        self.failed = None;
        self.anchor = None;
        self.return_anchor = None;
    }
    /// Slide the rendered range while keeping the exposed identity and offset.
    pub fn slide(&mut self, ids: &[String], following: bool) -> bool {
        if following {
            return false;
        }
        let Some(anchor) = &self.anchor else {
            return false;
        };
        let Some(index) = ids.iter().position(|id| id == &anchor.id) else {
            return false;
        };
        let start = self
            .start_id
            .as_ref()
            .and_then(|id| ids.iter().position(|x| x == id))
            .unwrap_or(0);
        let next = if index < start + EDGE_ROWS {
            start.saturating_sub(SHIFT_ROWS)
        } else if index >= start + UI_ROWS - EDGE_ROWS && start + UI_ROWS < ids.len() {
            (start + SHIFT_ROWS).min(ids.len().saturating_sub(UI_ROWS))
        } else {
            start
        };
        if next == start {
            return false;
        }
        self.start_id = ids.get(next).cloned();
        true
    }
    pub fn reveal(&mut self, ids: &[String], id: &str, screen_y: f32) -> bool {
        let Some(index) = ids.iter().position(|item| item == id) else {
            return false;
        };
        self.start_id = ids.get(index.saturating_sub(EDGE_ROWS)).cloned();
        self.anchor = Some(Anchor {
            id: id.into(),
            screen_y,
        });
        true
    }
    /// Preserve the nearest surviving measured neighbour if an anchor vanishes.
    pub fn reconcile_anchor(&mut self, ids: &[String]) {
        let Some(anchor) = &self.anchor else {
            return;
        };
        if ids.contains(&anchor.id) {
            return;
        }
        let old_y = self.measurements.get(&anchor.id).map(|v| v.0).unwrap_or(0.);
        self.anchor = self
            .measurements
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .min_by(|a, b| (a.1.0 - old_y).abs().total_cmp(&(b.1.0 - old_y).abs()))
            .map(|(id, (y, _))| Anchor {
                id: id.clone(),
                screen_y: anchor.screen_y + y - old_y,
            });
    }
    pub fn begin(&mut self, cursor: Cursor, direction: Direction) -> bool {
        if self.request.is_some() || (direction == Direction::Older && self.reached_beginning) {
            return false;
        }
        self.failed = None;
        self.request = Some(Request { cursor, direction });
        true
    }
    pub fn failed(&mut self, cursor: &str) {
        if self
            .request
            .as_ref()
            .is_some_and(|request| request.cursor.message == cursor)
        {
            self.failed = self.request.take();
        }
    }
    pub fn add_page(&mut self, page: Page) {
        self.pages.retain(|old| old.first_order != page.first_order);
        self.pages.push(page);
        self.pages.sort_by(|a, b| a.first_order.cmp(&b.first_order));
        if self.pages.len() > 128 {
            self.pages.drain(..self.pages.len() - 128);
        }
    }
    /// A cursor reopens an evicted newer page from the worker's encrypted range cache.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
    pub fn next_newer(&self, last_order: &str) -> Option<Option<Cursor>> {
        self.pages
            .iter()
            .find(|page| page.last_order.as_str() > last_order)
            .map(|page| page.cursor.clone())
    }
}

/// Keep a contiguous range around the reader, bounded by both rows and text bytes.
/// Returns whether newer rows were evicted. Older evictions are always reloadable
/// using the first retained row's server cursor.
pub(crate) fn bound<T>(
    rows: &mut Vec<T>,
    anchor: Option<&str>,
    id: impl Fn(&T) -> &str,
    cost: impl Fn(&T) -> usize,
) -> bool {
    let anchor_index = anchor
        .and_then(|id_value| rows.iter().position(|row| id(row) == id_value))
        .unwrap_or(rows.len().saturating_sub(1));
    let mut start = anchor_index
        .saturating_sub(RETAINED_ROWS / 2)
        .min(rows.len().saturating_sub(RETAINED_ROWS));
    let mut end = (start + RETAINED_ROWS).min(rows.len());
    let mut bytes: usize = rows[start..end].iter().map(&cost).sum();
    while bytes > RETAINED_BYTES && start + 1 < end {
        if anchor_index.saturating_sub(start) > end.saturating_sub(anchor_index + 1) {
            bytes -= cost(&rows[start]);
            start += 1;
        } else {
            end -= 1;
            bytes -= cost(&rows[end]);
        }
    }
    let newer_evicted = end < rows.len();
    rows.truncate(end);
    rows.drain(..start);
    newer_evicted
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thousand_messages_keep_bounded_ranges_and_reloadable_cursors() {
        let mut window = HistoryWindow::default();
        let mut rows: Vec<String> = (950..1000).map(|n| format!("{n:04}")).collect();
        let mut maximum = 0;
        for page in (0..19).rev() {
            let cursor = Cursor {
                message: rows[0].clone(),
                delivered_time: page,
            };
            assert!(window.begin(cursor.clone(), Direction::Older));
            assert!(!window.begin(cursor.clone(), Direction::Older));
            let mut older: Vec<String> = (page * 50..page * 50 + 50)
                .map(|n| format!("{n:04}"))
                .collect();
            older.append(&mut rows);
            rows = older;
            window.request.take();
            window.add_page(Page {
                first_order: rows[0].clone(),
                last_order: rows[49].clone(),
                cursor: Some(cursor),
            });
            let anchored = rows[50].clone();
            window.reveal(&rows, &anchored, 20.);
            window.newer_evicted |= bound(&mut rows, Some(&anchored), String::as_str, String::len);
            let rendered = window.range(&rows, false);
            maximum = maximum.max(rows.len());
            assert!(rendered.len() <= UI_ROWS);
            assert!(rows.contains(&anchored));
        }
        assert_eq!(maximum, RETAINED_ROWS);
        assert!(window.newer_evicted);
        assert_eq!(rows[0], "0000");
        assert!(window.next_newer(rows.last().unwrap()).is_some());
    }
    #[test]
    fn prepend_and_failure_preserve_one_anchor_without_scroll_callbacks() {
        let mut window = HistoryWindow::default();
        window.measured("anchor", 100., 40., false);
        assert_eq!(
            window.capture(-105.),
            Some(Anchor {
                id: "anchor".into(),
                screen_y: -5.
            })
        );
        assert!(window.begin(
            Cursor {
                message: "first".into(),
                delivered_time: 1
            },
            Direction::Older
        ));
        window.failed("first");
        assert!(window.failed.is_some());
        assert_eq!(window.measured("anchor", 100., 40., false), Some(-105.));
        let cursor = window.failed.take().unwrap().cursor;
        assert!(window.begin(cursor, Direction::Older));
        assert_eq!(window.measured("anchor", 4100., 40., false), Some(-4105.));
    }
    #[test]
    fn removed_anchor_falls_back_to_surviving_neighbour() {
        let mut window = HistoryWindow::default();
        window.measured("a", 0., 40., false);
        window.measured("b", 45., 40., false);
        window.capture(-10.);
        window.reconcile_anchor(&["b".into()]);
        assert_eq!(window.anchor.as_ref().unwrap().id, "b");
        assert_eq!(window.measured("b", 0., 40., false), Some(35.));
    }
    #[test]
    fn byte_budget_and_latest_cap_hold_for_untrusted_arrivals() {
        let mut rows: Vec<String> = (0..1000).map(|_| "x".repeat(16 * 1024)).collect();
        bound(&mut rows, None, String::as_str, String::len);
        assert!(rows.iter().map(String::len).sum::<usize>() <= RETAINED_BYTES);
        assert!(rows.len() <= RETAINED_ROWS);
        let ids: Vec<String> = (0..1000).map(|n| n.to_string()).collect();
        assert_eq!(HistoryWindow::default().range(&ids, true), 800..1000);
    }
}
