//! Session table column selection and FOLDER/TITLE widths (`table_layout`),
//! shared by the header cells, row cells, and width constraints.

/// Session table columns in display order. UPDATED, Q, and SIZE are optional and
/// dropped on narrow panes (see `table_layout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TableColumn {
    Agent,
    Folder,
    Updated,
    Title,
    Q,
    Size,
}

/// TITLE width below which the table gives up space: first FOLDER's extra width,
/// then the optional columns in the order SIZE, Q, UPDATED.
pub(super) const TITLE_MIN_W: usize = 25;
/// FOLDER width range. The column grows to the longest folder label up to
/// `FOLDER_MAX_PERCENT` of the table width (never below the minimum), but only
/// while every column is shown; it is back at the minimum before any column is
/// hidden, so narrowing the window never widens it.
pub(super) const FOLDER_MIN_W: usize = 12;
const FOLDER_MAX_PERCENT: usize = 20;

fn folder_max_w(area_width: u16) -> usize {
    (area_width as usize * FOLDER_MAX_PERCENT / 100).max(FOLDER_MIN_W)
}

pub(super) struct TableLayout {
    pub(super) columns: Vec<TableColumn>,
    pub(super) folder_w: usize,
    pub(super) title_w: usize,
}

impl TableLayout {
    fn is_last(&self, col: TableColumn) -> bool {
        self.columns.last() == Some(&col)
    }

    /// Fixed cell width of a non-TITLE column.
    /// UPDATED is 10 characters + 1 right padding before TITLE. SIZE is 5
    /// right-aligned characters + 1 right margin before the border; Q takes that
    /// margin over when SIZE is hidden.
    pub(super) fn width(&self, col: TableColumn) -> usize {
        match col {
            TableColumn::Agent => 4,
            TableColumn::Folder => self.folder_w,
            TableColumn::Updated => 11,
            TableColumn::Title => self.title_w,
            TableColumn::Q if self.is_last(col) => 5,
            TableColumn::Q => 4,
            TableColumn::Size => 6,
        }
    }

    pub(super) fn pad_right(&self, col: TableColumn, text: String) -> String {
        if col == TableColumn::Q && self.is_last(col) {
            text + " "
        } else {
            text
        }
    }

    /// TITLE width left in a table `area_width` wide. inner area = area.width -
    /// borders (2); subtract highlight_symbol (1), the fixed columns, one spacing
    /// cell per gap, and 1 cell padding that keeps a double-width character from
    /// overflowing the right border.
    fn fit_title(&mut self, area_width: u16) {
        let fixed: usize = self
            .columns
            .iter()
            .filter(|&&c| c != TableColumn::Title)
            .map(|&c| self.width(c))
            .sum();
        let gaps = self.columns.len() - 1;
        self.title_w = (area_width as usize).saturating_sub(2 + 1 + fixed + gaps + 1);
    }
}

/// Picks the visible columns and the FOLDER/TITLE widths for a table
/// `area_width` wide whose longest folder label is `folder_label_w` cells.
pub(super) fn table_layout(area_width: u16, folder_label_w: usize) -> TableLayout {
    let mut layout = TableLayout {
        columns: vec![
            TableColumn::Agent,
            TableColumn::Folder,
            TableColumn::Updated,
            TableColumn::Title,
            TableColumn::Q,
            TableColumn::Size,
        ],
        folder_w: folder_label_w.clamp(FOLDER_MIN_W, folder_max_w(area_width)),
        title_w: 0,
    };
    layout.fit_title(area_width);
    if layout.title_w >= TITLE_MIN_W {
        return layout;
    }
    // Give back FOLDER's extra width before hiding any column.
    let deficit = TITLE_MIN_W - layout.title_w;
    layout.folder_w = layout.folder_w.saturating_sub(deficit).max(FOLDER_MIN_W);
    layout.fit_title(area_width);
    for col in [TableColumn::Size, TableColumn::Q, TableColumn::Updated] {
        if layout.title_w >= TITLE_MIN_W {
            break;
        }
        layout.columns.retain(|&c| c != col);
        layout.fit_title(area_width);
    }
    layout
}

#[cfg(test)]
mod tests {
    use super::{table_layout, TableColumn::*, FOLDER_MIN_W, TITLE_MIN_W};

    #[test]
    fn table_layout_drops_size_then_q_then_updated() {
        // (table width, visible columns, TITLE width) with FOLDER at its minimum.
        let cases = [
            (71, vec![Agent, Folder, Updated, Title, Q, Size], 25),
            (70, vec![Agent, Folder, Updated, Title, Q], 30),
            (65, vec![Agent, Folder, Updated, Title, Q], 25),
            (64, vec![Agent, Folder, Updated, Title], 30),
            (59, vec![Agent, Folder, Updated, Title], 25),
            (58, vec![Agent, Folder, Title], 36),
        ];
        for (width, columns, title_w) in cases {
            let layout = table_layout(width, 0);
            assert_eq!(layout.columns, columns, "width {width}");
            assert_eq!(layout.folder_w, FOLDER_MIN_W, "width {width}");
            assert_eq!(layout.title_w, title_w, "width {width}");
        }
    }

    #[test]
    fn folder_grows_to_the_longest_label_within_bounds() {
        assert_eq!(table_layout(200, 10).folder_w, FOLDER_MIN_W);
        assert_eq!(table_layout(200, 20).folder_w, 20);
        // The maximum is 20% of the table width: 40 at 200, 20 at 100.
        assert_eq!(table_layout(200, 50).folder_w, 40);
        assert_eq!(table_layout(100, 50).folder_w, 20);
    }

    #[test]
    fn folder_shrinks_before_any_column_is_hidden() {
        // At 73 cells the 20% cap (14) still leaves TITLE 25.
        let layout = table_layout(73, 20);
        assert_eq!(layout.columns, vec![Agent, Folder, Updated, Title, Q, Size]);
        assert_eq!((layout.folder_w, layout.title_w), (14, 25));
        // At 71 the cap (14) leaves TITLE 23, so FOLDER returns to the minimum.
        let layout = table_layout(71, 20);
        assert_eq!(layout.columns, vec![Agent, Folder, Updated, Title, Q, Size]);
        assert_eq!((layout.folder_w, layout.title_w), (FOLDER_MIN_W, 25));
        // Once a column is hidden FOLDER stays at the minimum, never regrowing.
        let layout = table_layout(70, 20);
        assert_eq!(layout.columns, vec![Agent, Folder, Updated, Title, Q]);
        assert_eq!((layout.folder_w, layout.title_w), (FOLDER_MIN_W, 30));
    }

    #[test]
    fn table_layout_keeps_agent_folder_title_when_still_narrow() {
        let layout = table_layout(30, 20);
        assert_eq!(layout.columns, vec![Agent, Folder, Title]);
        assert_eq!(layout.folder_w, FOLDER_MIN_W);
        assert!(layout.title_w < TITLE_MIN_W);
    }

    #[test]
    fn q_takes_the_right_margin_when_it_is_the_last_column() {
        assert_eq!(table_layout(71, 0).width(Q), 4);
        assert_eq!(table_layout(65, 0).width(Q), 5);
    }
}
