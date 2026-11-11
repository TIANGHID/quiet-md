//! 数据表 (Data table): the grid UI, built to match Feishu 多维表格 as closely
//! as GPUI allows. The model is `fdl_core::datatable`; this file is the view —
//! the title bar, the view tab bar, the two-row toolbar, and the grid itself
//! (gutter with row numbers + checkbox, typed cells, add-field / add-record).
//!
//! Tokens come from `feishu_theme` (the project already carries Feishu's UD
//! palette), so colours, radii and metrics line up with the real product:
//! primary #1456F0 (B600), text #1F2329 (N900), grid line N300, header N100.
//!
//! Stage 1 covers rendering of every cell type, cell selection, inline text /
//! number editing, add-record, add-field (the full grouped type picker), and
//! the cheap in-cell interactions (checkbox, rating, single-select). Frozen
//! columns, drag-resize, filtering/grouping/sorting and multi-table bases are
//! marked in docs/design/DATA_TABLE.md as later passes.

use crate::app::{View, Workspace};
use crate::feishu_theme::{Theme, ud};
use crate::input::{TextField, TextFieldEvent};
use crate::ui::{icon, popover, ui_text};
use crate::datatable::{CellValue, Field, FieldGroup, FieldType, Record, SelectOption, Table, View as DtView};
use gpui::prelude::FluentBuilder;
use gpui::*;

// ---- metrics (match Feishu's grid) ----
const GUTTER_W: f32 = 44.0; // row-number / checkbox column
const HEADER_H: f32 = 40.0; // field header row
const ADD_FIELD_W: f32 = 44.0; // the trailing "+" column
const CELL_PAD_X: f32 = 8.0;

/// A single/multi-select colour slot: (background, text). Feishu's tag palette —
/// light pastel fill with a saturated label.
pub const SELECT_PALETTE: &[(u32, u32)] = &[
    (0xf2f3f5, 0x646a73), // gray
    (0xfde2e2, 0xd83931), // red
    (0xfff1e0, 0xd87a1e), // orange
    (0xfdf4d0, 0xa6850a), // yellow
    (0xe8f7e3, 0x2ea121), // green
    (0xdef7f4, 0x0e8a74), // teal
    (0xe1edff, 0x1456f0), // blue
    (0xeae6ff, 0x6b5ce7), // purple
    (0xfde6f3, 0xc83f9a), // pink
    (0xeceef1, 0x515960), // slate
];

pub fn select_colors(slot: u8) -> (Hsla, Hsla) {
    let (bg, fg) = SELECT_PALETTE[slot as usize % SELECT_PALETTE.len()];
    (crate::feishu_theme::c(bg), crate::feishu_theme::c(fg))
}

/// The best existing icon glyph for a field type (icon set is small; refined
/// type glyphs are a later pass).
fn field_glyph(ty: FieldType) -> &'static str {
    use FieldType::*;
    match ty {
        Text | Phone | Email | Barcode | Location | Signature => "ft-text",
        SingleSelect => "ft-select",
        MultiSelect => "ft-multi",
        Checkbox => "ft-check",
        Number | Currency | AutoNumber => "ft-number",
        Formula => "sigma",
        Rating => "star",
        Date | CreatedTime | ModifiedTime => "ft-date",
        Url => "ft-link",
        Attachment => "ft-attach",
        Person | Group | CreatedBy | ModifiedBy => "ft-person",
        Progress => "ft-number",
        _ => "ft-text",
    }
}

/// All grid-view state for the open data table.
pub struct DataTableView {
    pub table: Table,
    pub view_idx: usize,
    /// Selected cell: (record index, visible-field index).
    pub sel: Option<(usize, usize)>,
    /// The cell the inline input currently overlays.
    pub editing: Option<(usize, usize)>,
    pub input: Entity<TextField>,
    /// Search/create box inside the single-select dropdown.
    pub menu_input: Entity<TextField>,
    /// When set, the select dropdown shows the editor for this option id
    /// (recolor / delete) instead of the option list.
    pub opt_edit: Option<String>,
    /// Add-field popover: where it hangs (None = closed).
    pub add_field: Option<Point<Pixels>>,
    /// Single/multi-select dropdown: (record, field, anchor).
    pub select_menu: Option<(usize, usize, Point<Pixels>)>,
    /// 行高 menu anchor.
    pub row_height_menu: Option<Point<Pixels>>,
    /// Field-header menu: (field index, anchor).
    pub field_menu: Option<(usize, Point<Pixels>)>,
    /// Field editor (rename + change type): (field index, anchor).
    pub field_editor: Option<(usize, Point<Pixels>)>,
    /// Name box inside the field editor.
    pub field_input: Entity<TextField>,
    /// Date picker: (record, field index, anchor, shown year, shown month).
    pub date_menu: Option<(usize, usize, Point<Pixels>, i64, u32)>,
    /// Column drag-resize in progress: (field id, mouse-down x, start width).
    pub col_drag: Option<(String, f32, f32)>,
    /// Selected record indices (gutter checkboxes).
    pub row_sel: std::collections::HashSet<usize>,
    /// Shared horizontal scroll so the header and body stay aligned.
    pub hscroll: ScrollHandle,
    /// Vertical scroll for the body rows.
    pub vscroll: ScrollHandle,
}

impl DataTableView {
    pub fn active_view(&self) -> &DtView {
        &self.table.views[self.view_idx.min(self.table.views.len() - 1)]
    }
}

impl Workspace {
    /// Open the data table page with a seeded demo table (stage 1: in-memory,
    /// not yet persisted through the store).
    pub(crate) fn open_datatable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let table = demo_table();
        let input = cx.new(|cx| {
            let mut t = TextField::new("", cx);
            t.size = 13.;
            t
        });
        // Commit or cancel inline edits.
        cx.subscribe_in(&input, window, |w, _, ev: &TextFieldEvent, window, cx| match ev {
            TextFieldEvent::Submit => w.commit_cell_edit(true, window, cx),
            TextFieldEvent::Cancel => w.commit_cell_edit(false, window, cx),
            _ => {}
        })
        .detach();
        let menu_input = cx.new(|cx| {
            let mut t = TextField::new("搜索选项", cx);
            t.size = 13.;
            t
        });
        // Enter in the dropdown search box creates a new option; Esc closes it.
        cx.subscribe_in(&menu_input, window, |w, _, ev: &TextFieldEvent, _window, cx| match ev {
            TextFieldEvent::Submit => w.create_select_option(cx),
            TextFieldEvent::Cancel => { if let Some(dt) = w.datatable.as_mut() { dt.select_menu = None; dt.opt_edit = None; } cx.notify(); }
            _ => cx.notify(),
        })
        .detach();
        let field_input = cx.new(|cx| {
            let mut t = TextField::new("字段名称", cx);
            t.size = 13.;
            t
        });
        // Enter / Esc in the field-editor name box commits the rename and closes.
        cx.subscribe_in(&field_input, window, |w, _, ev: &TextFieldEvent, _window, cx| match ev {
            TextFieldEvent::Submit => { w.commit_field_rename(cx); if let Some(dt) = w.datatable.as_mut() { dt.field_editor = None; } cx.notify(); }
            TextFieldEvent::Cancel => { if let Some(dt) = w.datatable.as_mut() { dt.field_editor = None; } cx.notify(); }
            _ => {}
        })
        .detach();
        self.datatable = Some(DataTableView {
            table,
            view_idx: 0,
            sel: None,
            editing: None,
            input,
            menu_input,
            opt_edit: None,
            add_field: None,
            select_menu: None,
            row_height_menu: None,
            field_menu: None,
            field_editor: None,
            field_input,
            date_menu: None,
            col_drag: None,
            row_sel: Default::default(),
            hscroll: ScrollHandle::new(),
            vscroll: ScrollHandle::new(),
        });
        self.view = View::DataTable;
        // Give the grid the keyboard so arrow/Tab/Enter navigation works.
        window.focus(&self.focus);
        cx.notify();
    }

    pub(crate) fn close_datatable(&mut self, cx: &mut Context<Self>) {
        self.datatable = None;
        self.view = View::Home;
        cx.notify();
    }

    /// Begin inline editing of a text/number cell.
    fn begin_cell_edit(&mut self, rec: usize, col: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dt) = self.datatable.as_ref() else { return };
        let fields = dt.table.visible_fields(dt.active_view());
        let Some(field) = fields.get(col) else { return };
        if !matches!(field.ty, FieldType::Text | FieldType::Number | FieldType::Currency | FieldType::Progress | FieldType::Phone | FieldType::Email | FieldType::Url) {
            return;
        }
        let rid = dt.table.records[rec].id.clone();
        let fid = field.id.clone();
        let cur = match dt.table.cell(&rid, &fid) {
            CellValue::Text(s) => s.clone(),
            CellValue::Number(n) => format!("{n}"),
            CellValue::Url { href, .. } => href.clone(),
            _ => String::new(),
        };
        let input = dt.input.clone();
        input.update(cx, |t, cx| {
            t.set_text(&cur, cx);
            t.select_all_text(cx);
        });
        window.focus(&input.read(cx).focus_handle(cx));
        let dt = self.datatable.as_mut().unwrap();
        dt.editing = Some((rec, col));
        dt.sel = Some((rec, col));
        cx.notify();
    }

    fn commit_cell_edit(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dt) = self.datatable.as_mut() else { return };
        let Some((rec, col)) = dt.editing.take() else { return };
        if save {
            let text = dt.input.read(cx).text.clone();
            let view = dt.table.views[dt.view_idx].clone();
            let fields = dt.table.visible_fields(&view);
            if let Some(field) = fields.get(col) {
                let (fid, ty) = (field.id.clone(), field.ty);
                let rid = dt.table.records[rec].id.clone();
                let value = match ty {
                    FieldType::Number | FieldType::Currency | FieldType::Progress => {
                        text.trim().parse::<f64>().map(CellValue::Number).unwrap_or(CellValue::Empty)
                    }
                    FieldType::Url => CellValue::Url { text: text.clone(), href: text.clone() },
                    _ => CellValue::Text(text),
                };
                dt.table.set_cell(&rid, &fid, value);
            }
        }
        window.focus(&self.focus);
        cx.notify();
    }

    /// Grid keyboard navigation (arrows / Tab / Enter / Space / Esc). Returns
    /// true if it consumed the key. Called from the Workspace key handler; while
    /// a cell editor or any popover is open the keyboard belongs to it.
    pub(crate) fn datatable_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.view != View::DataTable { return false; }
        let Some(dt) = self.datatable.as_ref() else { return false };
        if dt.editing.is_some() || dt.add_field.is_some() || dt.select_menu.is_some()
            || dt.date_menu.is_some() || dt.field_menu.is_some() || dt.row_height_menu.is_some()
            || dt.field_editor.is_some() {
            return false;
        }
        let km = ev.keystroke.modifiers;
        if km.secondary() || km.alt || km.control || km.function { return false; }
        // owned column (type, id) list so the &mut self action calls below don't
        // clash with the borrow of `dt`.
        let cols: Vec<(FieldType, String)> =
            dt.table.visible_fields(dt.active_view()).iter().map(|f| (f.ty, f.id.clone())).collect();
        let (ncols, nrows) = (cols.len(), dt.table.records.len());
        if ncols == 0 || nrows == 0 { return false; }
        // ignore a stale selection left over from a structural change
        let cur = dt.sel.filter(|&(r, c)| r < nrows && c < ncols);
        let key = ev.keystroke.key.clone();
        let set = |w: &mut Workspace, r: usize, c: usize, cx: &mut Context<Workspace>| {
            if let Some(dt) = w.datatable.as_mut() { dt.sel = Some((r, c)); dt.editing = None; }
            cx.notify();
        };
        match key.as_str() {
            "down" => { let (r, c) = cur.map_or((0, 0), |(r, c)| ((r + 1).min(nrows - 1), c)); set(self, r, c, cx); }
            "up" => { let (r, c) = cur.unwrap_or((0, 0)); set(self, r.saturating_sub(1), c, cx); }
            "left" => { let (r, c) = cur.unwrap_or((0, 0)); set(self, r, c.saturating_sub(1), cx); }
            "right" => { let (r, c) = cur.map_or((0, 0), |(r, c)| (r, (c + 1).min(ncols - 1))); set(self, r, c, cx); }
            "tab" => {
                let (r, c) = cur.unwrap_or((0, 0));
                let (r, c) = if km.shift {
                    if c == 0 { (r.saturating_sub(1), ncols - 1) } else { (r, c - 1) }
                } else if c + 1 >= ncols { ((r + 1).min(nrows - 1), 0) } else { (r, c + 1) };
                set(self, r, c, cx);
            }
            "enter" => match cur {
                Some((r, c)) if matches!(cols[c].0, FieldType::Checkbox) => self.toggle_checkbox(r, cols[c].1.clone(), cx),
                Some((r, c)) => self.begin_cell_edit(r, c, window, cx),
                None => return false,
            },
            "space" => match cur {
                Some((r, c)) if matches!(cols[c].0, FieldType::Checkbox) => self.toggle_checkbox(r, cols[c].1.clone(), cx),
                _ => return false,
            },
            "escape" if cur.is_some() => { if let Some(dt) = self.datatable.as_mut() { dt.sel = None; } cx.notify(); }
            _ => return false,
        }
        true
    }

    fn add_record(&mut self, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            dt.table.add_record();
            cx.notify();
        }
    }

    fn toggle_row(&mut self, ri: usize, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            if !dt.row_sel.remove(&ri) { dt.row_sel.insert(ri); }
            cx.notify();
        }
    }

    /// Header checkbox: select all rows, or clear when all are already selected.
    fn toggle_all_rows(&mut self, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            let n = dt.table.records.len();
            if dt.row_sel.len() == n { dt.row_sel.clear(); } else { dt.row_sel = (0..n).collect(); }
            cx.notify();
        }
    }

    fn delete_selected_rows(&mut self, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            let ids: Vec<String> = dt.row_sel.iter().filter_map(|&i| dt.table.records.get(i)).map(|r| r.id.clone()).collect();
            for id in ids { dt.table.remove_record(&id); }
            dt.row_sel.clear();
            dt.sel = None;
            cx.notify();
        }
    }

    fn add_field_of(&mut self, ty: FieldType, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            dt.table.add_field(ty.label(), ty);
            dt.add_field = None;
            cx.notify();
        }
    }

    /// Open the field editor (rename + change type) for a field, prefilling and
    /// focusing its name box.
    fn open_field_editor(&mut self, fidx: usize, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            let name = dt.table.fields.get(fidx).map(|f| f.name.clone()).unwrap_or_default();
            dt.field_menu = None;
            dt.field_editor = Some((fidx, pos));
            let input = dt.field_input.clone();
            input.update(cx, |t, cx| { t.set_text(&name, cx); t.select_all_text(cx); });
            window.focus(&input.read(cx).focus_handle(cx));
            cx.notify();
        }
    }

    fn commit_field_rename(&mut self, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_ref() {
            let Some((fidx, _)) = dt.field_editor else { return };
            let name = dt.field_input.read(cx).text.trim().to_string();
            if name.is_empty() { return; }
            let dt = self.datatable.as_mut().unwrap();
            if let Some(f) = dt.table.fields.get_mut(fidx) { f.name = name; }
            cx.notify();
        }
    }

    /// Change a field's type from the editor. The primary column stays text.
    fn change_field_type(&mut self, fidx: usize, ty: FieldType, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut()
            && let Some(f) = dt.table.fields.get_mut(fidx) {
            if fidx == 0 { return; } // primary is always the text title column
            f.ty = ty;
            // select options only make sense for select types
            if !matches!(ty, FieldType::SingleSelect | FieldType::MultiSelect) {
                f.config.options.clear();
            }
            cx.notify();
        }
    }

    fn insert_field(&mut self, at: usize, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            let f = crate::datatable::Field::new(FieldType::Text.label(), FieldType::Text);
            let at = at.min(dt.table.fields.len());
            dt.table.fields.insert(at.max(1), f); // never before the primary column
            dt.field_menu = None;
            dt.sel = None; // column indices shift
            cx.notify();
        }
    }

    /// Sort the records by one field (Feishu's 升序/降序 on the column menu).
    /// Empty cells sink to the bottom regardless of direction.
    fn sort_records(&mut self, fidx: usize, asc: bool, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut()
            && let Some(field) = dt.table.fields.get(fidx).cloned() {
            let fid = field.id.clone();
            // sort key: (is_empty, numeric, text) so blanks always trail
            let key = |rec: &Record| -> (bool, f64, String) {
                match rec.get(&fid) {
                    CellValue::Empty => (true, 0.0, String::new()),
                    CellValue::Number(n) => (false, *n, String::new()),
                    CellValue::Rating(n) => (false, *n as f64, String::new()),
                    CellValue::Date(ms) => (false, *ms as f64, String::new()),
                    CellValue::Checkbox(b) => (false, if *b { 1.0 } else { 0.0 }, String::new()),
                    other => (false, 0.0, other.as_plain(&field).to_lowercase()),
                }
            };
            dt.table.records.sort_by(|a, b| {
                let (ea, na, ta) = key(a);
                let (eb, nb, tb) = key(b);
                // blanks last in both directions
                if ea != eb { return ea.cmp(&eb); }
                let ord = na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal).then_with(|| ta.cmp(&tb));
                if asc { ord } else { ord.reverse() }
            });
            dt.field_menu = None;
            dt.sel = None;
            dt.row_sel.clear();
            cx.notify();
        }
    }

    fn hide_field(&mut self, fidx: usize, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            if let Some(f) = dt.table.fields.get(fidx) {
                let id = f.id.clone();
                let i = dt.view_idx;
                if !dt.table.views[i].hidden.contains(&id) { dt.table.views[i].hidden.push(id); }
            }
            dt.field_menu = None;
            dt.sel = None; // column indices shift
            cx.notify();
        }
    }

    fn delete_field(&mut self, fidx: usize, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            if let Some(f) = dt.table.fields.get(fidx) {
                let id = f.id.clone();
                dt.table.remove_field(&id);
            }
            dt.field_menu = None;
            dt.sel = None; // column indices shift
            cx.notify();
        }
    }

    fn toggle_checkbox(&mut self, rec: usize, fid: String, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            let rid = dt.table.records[rec].id.clone();
            let on = matches!(dt.table.cell(&rid, &fid), CellValue::Checkbox(true));
            dt.table.set_cell(&rid, &fid, CellValue::Checkbox(!on));
            cx.notify();
        }
    }

    fn set_rating(&mut self, rec: usize, fid: String, stars: u8, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            let rid = dt.table.records[rec].id.clone();
            let cur = matches!(dt.table.cell(&rid, &fid), CellValue::Rating(n) if *n == stars);
            dt.table.set_cell(&rid, &fid, CellValue::Rating(if cur { 0 } else { stars }));
            cx.notify();
        }
    }

    fn pick_select(&mut self, rec: usize, fid: String, opt: String, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            let rid = dt.table.records[rec].id.clone();
            dt.table.set_cell(&rid, &fid, CellValue::Select(vec![opt]));
            dt.select_menu = None;
            dt.opt_edit = None;
            cx.notify();
        }
    }

    /// Open the single-select dropdown for a cell, resetting and focusing the
    /// search/create box.
    fn open_select_menu(&mut self, rec: usize, fidx: usize, pos: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            dt.select_menu = Some((rec, fidx, pos));
            dt.opt_edit = None;
            let input = dt.menu_input.clone();
            input.update(cx, |t, cx| t.set_text("", cx));
            window.focus(&input.read(cx).focus_handle(cx));
            cx.notify();
        }
    }

    /// Create a new option from the search box text (Feishu's 新建选项), give it
    /// the next palette colour, and apply it to the record.
    fn create_select_option(&mut self, cx: &mut Context<Self>) {
        let Some(dt) = self.datatable.as_ref() else { return };
        let Some((rec, fidx, _)) = dt.select_menu else { return };
        let name = dt.menu_input.read(cx).text.trim().to_string();
        if name.is_empty() { return; }
        let dt = self.datatable.as_mut().unwrap();
        let Some(field) = dt.table.fields.get_mut(fidx) else { return };
        // reuse an existing option of the same name rather than duplicating
        let oid = if let Some(o) = field.config.options.iter().find(|o| o.name == name) {
            o.id.clone()
        } else {
            let color = (field.config.options.len() % SELECT_PALETTE.len()) as u8;
            let opt = SelectOption::new(&name, color);
            let oid = opt.id.clone();
            field.config.options.push(opt);
            oid
        };
        let is_multi = matches!(field.ty, FieldType::MultiSelect);
        let (fid, rid) = (field.id.clone(), dt.table.records[rec].id.clone());
        if is_multi {
            // append to the existing tags and keep the dropdown open
            let mut ids = match dt.table.cell(&rid, &fid) { CellValue::Select(v) => v.clone(), _ => Vec::new() };
            if !ids.contains(&oid) { ids.push(oid); }
            dt.table.set_cell(&rid, &fid, CellValue::Select(ids));
        } else {
            dt.table.set_cell(&rid, &fid, CellValue::Select(vec![oid]));
            dt.select_menu = None;
        }
        dt.menu_input.update(cx, |t, cx| t.set_text("", cx));
        cx.notify();
    }

    /// Toggle one option in a multi-select cell, leaving the dropdown open.
    fn toggle_select(&mut self, rec: usize, fid: String, oid: String, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            let rid = dt.table.records[rec].id.clone();
            let mut ids = match dt.table.cell(&rid, &fid) { CellValue::Select(v) => v.clone(), _ => Vec::new() };
            if let Some(p) = ids.iter().position(|i| *i == oid) { ids.remove(p); } else { ids.push(oid); }
            dt.table.set_cell(&rid, &fid, if ids.is_empty() { CellValue::Empty } else { CellValue::Select(ids) });
            cx.notify();
        }
    }

    fn recolor_option(&mut self, fidx: usize, oid: String, color: u8, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut()
            && let Some(field) = dt.table.fields.get_mut(fidx)
            && let Some(o) = field.config.options.iter_mut().find(|o| o.id == oid) {
            o.color = color;
            cx.notify();
        }
    }

    fn delete_option(&mut self, fidx: usize, oid: String, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut()
            && let Some(field) = dt.table.fields.get_mut(fidx) {
            field.config.options.retain(|o| o.id != oid);
            let fid = field.id.clone();
            // scrub the deleted option out of every cell
            for r in 0..dt.table.records.len() {
                let rid = dt.table.records[r].id.clone();
                if let CellValue::Select(ids) = dt.table.cell(&rid, &fid) {
                    let kept: Vec<String> = ids.iter().filter(|i| **i != oid).cloned().collect();
                    if kept.len() != ids.len() {
                        dt.table.set_cell(&rid, &fid, if kept.is_empty() { CellValue::Empty } else { CellValue::Select(kept) });
                    }
                }
            }
            dt.opt_edit = None;
            cx.notify();
        }
    }

    /// Open the calendar for a date cell, starting on the cell's month (or the
    /// current month when empty).
    fn open_date_menu(&mut self, rec: usize, fidx: usize, pos: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            let secs = match dt.table.fields.get(fidx).map(|f| dt.table.records[rec].get(&f.id)) {
                Some(CellValue::Date(ms)) => (*ms / 1000).max(0) as u64,
                _ => crate::time::now(),
            };
            let (y, m, ..) = crate::time::civil(secs);
            dt.date_menu = Some((rec, fidx, pos, y, m));
            cx.notify();
        }
    }

    fn set_date(&mut self, rec: usize, fid: String, y: i64, m: u32, d: u32, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            let rid = dt.table.records[rec].id.clone();
            let ms = crate::time::local_to_unix(y, m, d, 0, 0) * 1000;
            dt.table.set_cell(&rid, &fid, CellValue::Date(ms));
            dt.date_menu = None;
            cx.notify();
        }
    }

    // ---- rendering ----

    pub(crate) fn render_datatable(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(dt) = self.datatable.as_ref() else { return div().into_any_element() };
        let view = dt.active_view();
        let fields = dt.table.visible_fields(view);
        let row_h = view.row_height.px();
        let title = dt.table.name.clone();

        let top = self.dt_titlebar(&title, cx);
        let rail = self.dt_rail(cx);
        let tabs = self.dt_tab_bar(cx);
        let toolbar = self.dt_toolbar(cx);
        let grid = self.dt_grid(&fields, row_h, window, cx);

        let main = div().flex_1().min_w_0().h_full().flex().flex_col()
            .child(tabs).child(toolbar).child(grid);
        let body = div().flex_1().min_h_0().flex().child(rail).child(main);

        let mut page = div()
            .flex_1().min_w_0().h_full().flex().flex_col().bg(Theme::bg_body())
            .child(top)
            .child(body);
        for overlay in self.dt_overlays(cx) {
            page = page.child(overlay);
        }
        // Column drag-resize is tracked on the always-present page root (not a
        // deferred overlay) so the live resize works even within a single frame:
        // the handlers are registered every frame and only act while `col_drag`
        // is set. A grip's mouse-down (in the header) sets `col_drag`.
        let end = |w: &mut Workspace, cx: &mut Context<Workspace>| {
            if let Some(dt) = w.datatable.as_mut() && dt.col_drag.take().is_some() { cx.notify(); }
        };
        page = page
            .on_mouse_move(cx.listener(move |w, ev: &MouseMoveEvent, _, cx| {
                let Some((fid, start_x, start_w)) = w.datatable.as_ref().and_then(|dt| dt.col_drag.clone()) else { return };
                if ev.pressed_button != Some(MouseButton::Left) { return end(w, cx); }
                let nw = (start_w + f32::from(ev.position.x) - start_x).max(crate::datatable::FIELD_MIN_WIDTH);
                if let Some(dt) = w.datatable.as_mut()
                    && let Some(f) = dt.table.fields.iter_mut().find(|f| f.id == fid) { f.width = nw; }
                cx.notify();
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(move |w, _, _, cx| end(w, cx)));
        if let Some(bar) = self.dt_selection_bar(cx) {
            page = page.child(bar);
        }
        page.into_any_element()
    }

    /// Floating action bar shown while rows are selected (Feishu's 已选 N 条).
    fn dt_selection_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let n = self.datatable.as_ref().map(|dt| dt.row_sel.len()).unwrap_or(0);
        if n == 0 { return None; }
        let btn = |id: &'static str, glyph: &'static str, label: &'static str, danger: bool| {
            div().id(id).flex().items_center().gap(px(5.)).h(px(30.)).px(px(10.)).rounded(px(6.)).cursor_pointer()
                .hover(|s| s.bg(Theme::fill_hover()))
                .child(icon(glyph, 15., if danger { Theme::danger() } else { Theme::icon() }))
                .child(div().text_size(px(13.)).text_color(if danger { Theme::danger() } else { Theme::text() }).child(label))
        };
        Some(deferred(
            div().absolute().bottom(px(28.)).left_0().right_0().flex().justify_center()
                .child(
                    popover().id("dt-selbar").flex().items_center().gap(px(4.)).h(px(44.)).px(px(10.))
                        .child(div().text_size(px(13.)).text_color(Theme::text_caption()).mr(px(4.))
                            .child(format!("已选 {n} 条")))
                        .child(btn("dt-sel-del", "bin", "删除", true)
                            .on_click(cx.listener(|w, _, _, cx| w.delete_selected_rows(cx))))
                        .child(dt_divider())
                        .child(btn("dt-sel-cancel", "x", "取消", false)
                            .on_click(cx.listener(|w, _, _, cx| { if let Some(dt) = w.datatable.as_mut() { dt.row_sel.clear(); } cx.notify(); }))),
                ),
        ).with_priority(7).into_any_element())
    }

    /// Feishu's base rail: 搜索, then 数据表 / 仪表盘 / 工作流, 新建 at the foot.
    fn dt_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let item = |id: &'static str, glyph: &'static str, label: &'static str, on: bool| {
            div().id(id).flex().items_center().gap(px(8.)).h(px(32.)).px(px(10.)).mx(px(8.)).rounded(px(6.)).cursor_pointer()
                .when(on, |d| d.bg(Theme::fill_active()))
                .when(!on, |d| d.hover(|s| s.bg(Theme::fill_hover())))
                .child(icon(glyph, 16., if on { Theme::primary() } else { Theme::icon() }))
                .child(div().text_size(px(13.)).font_weight(if on { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                    .text_color(if on { Theme::primary() } else { Theme::text() }).child(label))
        };
        div()
            .flex_none().w(px(224.)).h_full().flex().flex_col()
            .bg(crate::feishu_theme::c(ud::N50))
            .border_r_1().border_color(Theme::line_border())
            .child(
                div().flex_none().h(px(44.)).flex().items_center().gap(px(8.)).px(px(16.))
                    .child(icon("search", 15., Theme::text_placeholder()))
                    .child(div().flex_1().text_size(px(13.)).text_color(Theme::text_placeholder()).child("搜索"))
                    .child(icon("plus", 15., Theme::icon())),
            )
            .child(div().h(px(4.)))
            .child(item("dt-rail-table", "view-grid", "数据表", true)
                .on_click(cx.listener(|_, _, _, cx| cx.notify())))
            .child(item("dt-rail-dash", "storage", "仪表盘", false))
            .child(item("dt-rail-flow", "list", "工作流", false))
            .child(div().flex_1())
            .child(
                div().id("dt-rail-new").flex().items_center().justify_center().gap(px(6.)).h(px(34.)).mx(px(12.)).mb(px(12.)).rounded(px(8.))
                    .border_1().border_color(Theme::line_border()).cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                    .child(icon("plus", 14., Theme::icon()))
                    .child(div().text_size(px(13.)).text_color(Theme::text()).child("新建")),
            )
            .into_any_element()
    }

    fn dt_titlebar(&self, title: &str, cx: &mut Context<Self>) -> AnyElement {
        let tool = |id: &'static str, glyph: &'static str| {
            div().id(id).flex_none().size(px(30.)).flex().items_center().justify_center().rounded(px(6.)).cursor_pointer()
                .hover(|s| s.bg(Theme::fill_hover())).child(icon(glyph, 17., Theme::icon()))
        };
        div()
            .flex_none().h(px(crate::app::HEADER_H)).flex().items_center()
            .pl(px(crate::feishu_theme::TRAFFIC_LIGHTS_W))
            .pr(px(crate::ui::header_right_inset()))
            .border_b_1().border_color(Theme::line_border())
            .child(
                div().id("dt-back").flex_none().size(px(28.)).mr(px(8.))
                    .flex().items_center().justify_center().rounded(px(8.)).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover_strong()))
                    .child(icon("arrow-left", 17., Theme::icon()))
                    .on_click(cx.listener(|w, _, _, cx| w.close_datatable(cx))),
            )
            .child(icon("storage", 18., Theme::primary()))
            .child(div().ml(px(8.)).text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD)
                .text_color(Theme::text()).child(ui_text(title.to_string())))
            .child(div().flex_1())
            .child(tool("dt-tb-undo", "restore"))
            .child(tool("dt-tb-redo", "restore"))
            .child(tool("dt-tb-search", "search"))
            .child(div().w(px(6.)))
            .child(
                div().id("dt-share").flex_none().flex().items_center().gap(px(5.)).h(px(32.)).px(px(12.)).ml(px(2.)).rounded(px(6.))
                    .bg(Theme::primary()).cursor_pointer().hover(|s| s.opacity(0.9))
                    .child(icon("export", 14., gpui::white()))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(gpui::white()).child("分享")),
            )
            .into_any_element()
    }

    fn dt_tab_bar(&self, _cx: &mut Context<Self>) -> AnyElement {
        let Some(dt) = self.datatable.as_ref() else { return div().into_any_element() };
        let view = dt.active_view();
        div()
            .flex_none().h(px(40.)).flex().items_center().px(px(12.)).gap(px(2.))
            .border_b_1().border_color(Theme::line_border())
            .child(
                div().flex().items_center().gap(px(6.)).px(px(10.)).h(px(28.)).rounded(px(6.))
                    .bg(Theme::fill_active())
                    .child(icon("view-grid", 15., Theme::primary()))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(Theme::primary())
                        .child(ui_text(view.name.clone()))),
            )
            .child(
                div().flex().items_center().gap(px(4.)).px(px(10.)).h(px(28.)).rounded(px(6.))
                    .text_color(Theme::text_caption())
                    .child(icon("plus", 14., Theme::icon()))
                    .child(div().text_size(px(13.)).child("新建视图")),
            )
            .into_any_element()
    }

    fn dt_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let item = |id: &'static str, glyph: &'static str, label: &'static str, primary: bool| {
            div().id(id).flex().items_center().gap(px(5.)).h(px(28.)).px(px(8.)).rounded(px(6.)).cursor_pointer()
                .hover(|s| s.bg(Theme::fill_hover()))
                .child(icon(glyph, 15., if primary { Theme::primary() } else { Theme::icon() }))
                .child(div().text_size(px(13.)).text_color(if primary { Theme::primary() } else { Theme::text() }).child(label))
        };
        div()
            .flex_none().h(px(42.)).flex().items_center().px(px(10.)).gap(px(2.))
            .border_b_1().border_color(Theme::line_border())
            .child(item("dt-add-record", "plus", "添加记录", true)
                .on_click(cx.listener(|w, _, _, cx| w.add_record(cx))))
            .child(dt_divider())
            .child(item("dt-fields", "view-settings", "字段配置", false))
            .child(item("dt-viewcfg", "eye", "视图配置", false))
            .child(item("dt-filter", "text-search", "筛选", false))
            .child(item("dt-group", "list", "分组", false))
            .child(item("dt-sort", "sort-down", "排序", false))
            .child(item("dt-rowh", "page-width", "行高", false)
                .on_click(cx.listener(|w, ev: &ClickEvent, _, cx| {
                    let p = ev.position();
                    if let Some(dt) = w.datatable.as_mut() { dt.row_height_menu = Some(point(p.x, p.y + px(8.))); }
                    cx.notify();
                })))
            .child(item("dt-color", "palette", "填色", false))
            .child(div().flex_1())
            .into_any_element()
    }

    fn dt_grid(&self, fields: &[&Field], row_h: f32, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(dt) = self.datatable.as_ref() else { return div().into_any_element() };
        let records = dt.table.records.clone();
        let sel = dt.sel;
        let editing = dt.editing;
        let input = dt.input.clone();
        let hscroll = dt.hscroll.clone();
        let vscroll = dt.vscroll.clone();
        let row_sel = dt.row_sel.clone();
        let all_sel = !records.is_empty() && row_sel.len() == records.len();

        // header row (white, like Feishu; only the gridlines separate it)
        let mut header = div().flex_none().flex().h(px(HEADER_H)).bg(Theme::bg_body())
            .border_b_1().border_color(Theme::table_line())
            .child(
                div().id("dt-hdr-check").flex_none().w(px(GUTTER_W)).h_full().flex().items_center().justify_center()
                    .border_r_1().border_color(Theme::table_line()).cursor_pointer()
                    .child(checkbox_box(all_sel))
                    .on_click(cx.listener(|w, _, _, cx| w.toggle_all_rows(cx))),
            );
        for (fi, f) in fields.iter().enumerate() {
            let (fid, fw) = (f.id.clone(), f.width);
            header = header.child(
                div().id(("dt-hdr", fi)).group("dt-hdr").relative().flex_none().w(px(f.width)).h_full().flex().items_center().gap(px(6.)).px(px(CELL_PAD_X))
                    .border_r_1().border_color(Theme::table_line()).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover()))
                    // the primary (标题) column carries Feishu's lock glyph
                    .when(fi == 0, |d| d.child(icon("ft-lock", 13., Theme::text_placeholder())))
                    .child(icon(field_glyph(f.ty), 14., Theme::text_caption()))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis()
                        .text_size(px(13.)).text_color(Theme::text()).child(ui_text(f.name.clone())))
                    // the field-menu chevron, shown on hover like Feishu (absolute so it
                    // never steals width from the name)
                    .child(div().absolute().right(px(4.)).top(px(12.)).flex().items_center().justify_center()
                        .size(px(16.)).rounded(px(4.)).bg(Theme::bg_body()).invisible().group_hover("dt-hdr", |s| s.visible())
                        .child(icon("chevron-down", 13., Theme::text_caption())))
                    .on_click(cx.listener(move |w, ev: &ClickEvent, _, cx| {
                        let p = ev.position();
                        if let Some(dt) = w.datatable.as_mut() { dt.field_menu = Some((fi, point(p.x, p.y + px(8.)))); }
                        cx.notify();
                    }))
                    // column drag-resize grip on the right edge (Feishu shows a blue bar)
                    .child(
                        div().id(("dt-colgrip", fi)).absolute().top_0().bottom_0().right(px(-3.)).w(px(7.)).flex().justify_center()
                            .cursor(CursorStyle::ResizeLeftRight)
                            .child(div().w(px(2.)).h_full().group_hover("dt-hdr", |s| s.bg(Theme::primary())))
                            .on_mouse_down(MouseButton::Left, cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                if let Some(dt) = w.datatable.as_mut() { dt.col_drag = Some((fid.clone(), f32::from(ev.position.x), fw)); }
                                cx.notify();
                            })),
                    ),
            );
        }
        // add-field "+"
        header = header.child(
            div().id("dt-add-field").flex_none().w(px(ADD_FIELD_W)).h_full().flex().items_center().justify_center()
                .cursor_pointer().hover(|s| s.bg(Theme::fill_hover()))
                .child(icon("plus", 15., Theme::icon()))
                .on_click(cx.listener(|w, ev: &ClickEvent, _, cx| {
                    let p = ev.position();
                    if let Some(dt) = w.datatable.as_mut() { dt.add_field = Some(point(p.x - px(160.), p.y + px(8.))); }
                    cx.notify();
                })),
        );

        // body rows — scrolls vertically; horizontal scroll is owned by the
        // outer wrapper so the header moves in lock-step.
        let mut body = div().id("dt-body").flex_1().min_h_0().overflow_y_scroll().track_scroll(&vscroll).flex().flex_col();
        for (ri, rec) in records.iter().enumerate() {
            let checked = row_sel.contains(&ri);
            let mut row = div().group("dt-row").flex_none().flex().h(px(row_h))
                .when(checked, |d| d.bg(Theme::fill_active()))
                .when(!checked, |d| d.hover(|s| s.bg(Theme::block_bg_area())))
                .border_b_1().border_color(Theme::table_line())
                .child(
                    // gutter: the row number, which becomes a checkbox on hover or
                    // while the row is selected (Feishu)
                    div().id(("dt-gutter", ri)).relative().flex_none().w(px(GUTTER_W)).h_full().flex().items_center().justify_center()
                        .border_r_1().border_color(Theme::table_line()).cursor_pointer()
                        .child(div().when(checked, |d| d.invisible()).group_hover("dt-row", |s| s.invisible())
                            .text_size(px(12.)).text_color(Theme::text_placeholder())
                            .child(format!("{}", ri + 1)))
                        .child(div().absolute().when(!checked, |d| d.invisible().group_hover("dt-row", |s| s.visible())).child(checkbox_box(checked)))
                        .on_click(cx.listener(move |w, _, _, cx| w.toggle_row(ri, cx))),
                );
            for (ci, f) in fields.iter().enumerate() {
                let on = sel == Some((ri, ci));
                let is_editing = editing == Some((ri, ci));
                let cell_body: AnyElement = if is_editing {
                    input.clone().into_any_element()
                } else {
                    self.dt_cell(rec, f, ri, ci, cx)
                };
                let (fid, ri2, ci2) = (f.id.clone(), ri, ci);
                row = row.child(
                    div().id(("dt-cell", ri * 1000 + ci)).flex_none().w(px(f.width)).h_full()
                        .flex().items_center().px(px(CELL_PAD_X)).overflow_hidden()
                        .border_r_1().border_color(Theme::table_line())
                        .when(on, |d| d.border_2().border_color(Theme::primary()).rounded(px(2.)))
                        .cursor_text()
                        .child(cell_body)
                        .on_click(cx.listener(move |w, _, _, cx| {
                            if let Some(dt) = w.datatable.as_mut() { dt.sel = Some((ri2, ci2)); dt.editing = None; }
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Left, {
                            let fid = fid.clone();
                            cx.listener(move |w, ev: &MouseDownEvent, window, cx| {
                                if ev.click_count >= 2 { w.begin_cell_edit(ri2, ci2, window, cx); let _ = &fid; }
                            })
                        }),
                );
            }
            body = body.child(row);
        }
        // add-record row
        body = body.child(
            div().id("dt-add-row").flex_none().flex().h(px(row_h)).cursor_pointer()
                .hover(|s| s.bg(Theme::fill_hover()))
                .child(div().flex_none().w(px(GUTTER_W)).h_full().flex().items_center().justify_center()
                    .child(icon("plus", 15., Theme::icon())))
                .on_click(cx.listener(|w, _, _, cx| w.add_record(cx))),
        );

        // Header (pinned at top) and body stacked in a column of intrinsic
        // width, wrapped in a single horizontal scroller so both pan together.
        let content = div().flex().flex_col().h_full().child(header).child(body);
        div().id("dt-xscroll").flex_1().min_h_0().overflow_x_scroll().track_scroll(&hscroll)
            .child(content).into_any_element()
    }

    /// Render one cell's content by field type.
    fn dt_cell(&self, rec: &Record, field: &Field, ri: usize, _ci: usize, cx: &mut Context<Self>) -> AnyElement {
        let v = rec.get(&field.id);
        match (field.ty, v) {
            (FieldType::Checkbox, _) => {
                let on = matches!(v, CellValue::Checkbox(true));
                let fid = field.id.clone();
                div().flex().items_center().justify_center().w_full()
                    .child(div().id(("dt-chk", ri)).cursor_pointer().child(checkbox_box(on))
                        .on_click(cx.listener(move |w, _, _, cx| w.toggle_checkbox(ri, fid.clone(), cx))))
                    .into_any_element()
            }
            (FieldType::Rating, _) => {
                let n = if let CellValue::Rating(n) = v { *n } else { 0 };
                let fid = field.id.clone();
                let max = field.config.rating_max.unwrap_or(5);
                let mut row = div().flex().items_center().gap(px(2.));
                for s in 1..=max {
                    let on = s <= n;
                    let fid = fid.clone();
                    row = row.child(div().id(("dt-star", ri * 10 + s as usize)).cursor_pointer()
                        .child(icon("star", 15., if on { crate::feishu_theme::c(ud::O350) } else { Theme::block_placeholder() }))
                        .on_click(cx.listener(move |w, _, _, cx| w.set_rating(ri, fid.clone(), s, cx))));
                }
                row.into_any_element()
            }
            (FieldType::SingleSelect | FieldType::MultiSelect, CellValue::Select(ids)) => {
                let fidx = self.datatable.as_ref().and_then(|dt| dt.table.fields.iter().position(|f| f.id == field.id)).unwrap_or(0);
                let mut row = div().id(("dt-sel", ri)).flex().items_center().gap(px(4.)).w_full().h_full().cursor_pointer();
                for id in ids {
                    if let Some(opt) = field.option(id) {
                        row = row.child(select_pill(opt));
                    }
                }
                row.on_click(cx.listener(move |w, ev: &ClickEvent, window, cx| {
                    let p = ev.position();
                    w.open_select_menu(ri, fidx, point(p.x, p.y + px(6.)), window, cx);
                })).into_any_element()
            }
            (FieldType::SingleSelect | FieldType::MultiSelect, _) => {
                // empty select: clicking opens the option menu
                let fidx = self.datatable.as_ref().and_then(|dt| dt.table.fields.iter().position(|f| f.id == field.id)).unwrap_or(0);
                div().id(("dt-sel-empty", ri)).w_full().h_full().cursor_pointer()
                    .on_click(cx.listener(move |w, ev: &ClickEvent, window, cx| {
                        let p = ev.position();
                        w.open_select_menu(ri, fidx, point(p.x, p.y + px(6.)), window, cx);
                    })).into_any_element()
            }
            (_, CellValue::Url { text, href }) => {
                let label = if text.is_empty() { href } else { text };
                div().text_size(px(13.)).text_color(Theme::link()).underline().overflow_hidden().whitespace_nowrap().text_ellipsis()
                    .child(ui_text(label.clone())).into_any_element()
            }
            (FieldType::Number | FieldType::Currency | FieldType::Progress, _) => {
                div().w_full().flex().justify_end().text_size(px(13.)).text_color(Theme::text())
                    .child(ui_text(v.as_plain(field))).into_any_element()
            }
            (FieldType::Date, _) => {
                let fidx = self.datatable.as_ref().and_then(|dt| dt.table.fields.iter().position(|f| f.id == field.id)).unwrap_or(0);
                let label = if let CellValue::Date(ms) = v { fmt_date(*ms) } else { String::new() };
                div().id(("dt-date", ri)).w_full().h_full().flex().items_center().cursor_pointer()
                    .text_size(px(13.)).text_color(Theme::text()).child(ui_text(label))
                    .on_click(cx.listener(move |w, ev: &ClickEvent, _, cx| {
                        let p = ev.position();
                        w.open_date_menu(ri, fidx, point(p.x, p.y + px(6.)), cx);
                    })).into_any_element()
            }
            _ => {
                let s = v.as_plain(field);
                div().text_size(px(13.)).text_color(Theme::text()).overflow_hidden().whitespace_nowrap().text_ellipsis()
                    .child(ui_text(s)).into_any_element()
            }
        }
    }

    /// Popovers that float over the grid: add-field picker, select dropdown,
    /// row-height menu.
    fn dt_overlays(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(dt) = self.datatable.as_ref() else { return vec![] };
        let mut out = Vec::new();

        if let Some(pos) = dt.add_field {
            out.push(deferred(anchored().anchor(Corner::TopLeft).position(pos).snap_to_window_with_margin(px(8.)).child(
                popover().id("dt-field-types").occlude().w(px(224.)).max_h(px(440.)).overflow_hidden().flex().flex_col()
                    .on_mouse_down_out(cx.listener(|w, _, _, cx| { if let Some(dt) = w.datatable.as_mut() { dt.add_field = None; } cx.notify(); }))
                    .child(div().flex_none().m(px(8.)).h(px(30.)).px(px(8.)).flex().items_center().gap(px(6.))
                        .rounded(px(6.)).bg(crate::feishu_theme::c(ud::N100))
                        .child(icon("search", 13., Theme::text_placeholder()))
                        .child(div().text_size(px(13.)).text_color(Theme::text_placeholder()).child("搜索")))
                    .child(div().id("dt-type-scroll").flex_1().min_h_0().overflow_y_scroll().pb(px(4.))
                        .children(self.dt_type_list(cx))),
            )).with_priority(5).into_any_element());
        }

        if let Some((rec, fidx, pos)) = dt.select_menu {
            out.push(self.dt_select_menu(rec, fidx, pos, cx));
        }

        if let Some(pos) = dt.row_height_menu {
            use crate::datatable::RowHeight::*;
            let cur = dt.active_view().row_height;
            out.push(deferred(anchored().anchor(Corner::TopLeft).position(pos).snap_to_window_with_margin(px(8.)).child(
                popover().id("dt-rowh-menu").occlude().w(px(120.)).py(px(4.))
                    .on_mouse_down_out(cx.listener(|w, _, _, cx| { if let Some(dt) = w.datatable.as_mut() { dt.row_height_menu = None; } cx.notify(); }))
                    .children([Short, Medium, Tall, ExtraTall].into_iter().map(|h| {
                        let on = h == cur;
                        div().id(SharedString::from(format!("dt-rh-{}", h.label()))).flex().items_center().h(px(32.)).px(px(12.)).cursor_pointer()
                            .hover(|s| s.bg(Theme::fill_hover()))
                            .when(on, |d| d.text_color(Theme::primary()))
                            .child(div().flex_1().text_size(px(13.)).child(h.label()))
                            .when(on, |d| d.child(icon("check", 14., Theme::primary())))
                            .on_click(cx.listener(move |w, _, _, cx| {
                                if let Some(dt) = w.datatable.as_mut() { let i = dt.view_idx; dt.table.views[i].row_height = h; dt.row_height_menu = None; }
                                cx.notify();
                            }))
                            .into_any_element()
                    })),
            )).with_priority(5).into_any_element());
        }

        if let Some((fidx, pos)) = dt.field_menu {
            let is_primary = fidx == 0;
            let row = |id: SharedString, glyph: &'static str, label: &'static str, danger: bool, disabled: bool| {
                div().id(id).flex().items_center().gap(px(8.)).h(px(32.)).mx(px(4.)).px(px(8.)).rounded(px(6.))
                    .when(!disabled, |d| d.cursor_pointer().hover(|s| s.bg(Theme::fill_hover())))
                    .child(icon(glyph, 15., if disabled { Theme::text_disabled() } else if danger { Theme::danger() } else { Theme::icon() }))
                    .child(div().text_size(px(13.)).text_color(if disabled { Theme::text_disabled() } else if danger { Theme::danger() } else { Theme::text() }).child(label))
            };
            out.push(deferred(anchored().anchor(Corner::TopLeft).position(pos).snap_to_window_with_margin(px(8.)).child(
                popover().id("dt-field-menu").occlude().w(px(176.)).py(px(4.))
                    .on_mouse_down_out(cx.listener(|w, _, _, cx| { if let Some(dt) = w.datatable.as_mut() { dt.field_menu = None; } cx.notify(); }))
                    .child(row("dt-fm-edit".into(), "square-pen", "编辑字段", false, false)
                        .on_click(cx.listener(move |w, _, window, cx| w.open_field_editor(fidx, pos, window, cx))))
                    .child(row("dt-fm-insl".into(), "indent-decrease", "向左插入", false, false)
                        .on_click(cx.listener(move |w, _, _, cx| w.insert_field(fidx, cx))))
                    .child(row("dt-fm-insr".into(), "indent-increase", "向右插入", false, false)
                        .on_click(cx.listener(move |w, _, _, cx| w.insert_field(fidx + 1, cx))))
                    .child(dt_menu_divider())
                    .child(row("dt-fm-asc".into(), "sort-down", "升序", false, false)
                        .on_click(cx.listener(move |w, _, _, cx| w.sort_records(fidx, true, cx))))
                    .child(row("dt-fm-desc".into(), "sort-down", "降序", false, false)
                        .on_click(cx.listener(move |w, _, _, cx| w.sort_records(fidx, false, cx))))
                    .child(row("dt-fm-hide".into(), "eye", "隐藏字段", false, is_primary)
                        .when(!is_primary, |d| d.on_click(cx.listener(move |w, _, _, cx| w.hide_field(fidx, cx)))))
                    .child(dt_menu_divider())
                    .child(row("dt-fm-del".into(), "bin", "删除字段", true, is_primary)
                        .when(!is_primary, |d| d.on_click(cx.listener(move |w, _, _, cx| w.delete_field(fidx, cx))))),
            )).with_priority(5).into_any_element());
        }

        if let Some((fidx, pos)) = dt.field_editor {
            out.push(self.dt_field_editor(fidx, pos, cx));
        }

        if let Some((rec, fidx, pos, y, m)) = dt.date_menu {
            out.push(self.dt_date_picker(rec, fidx, pos, y, m, cx));
        }
        out
    }

    /// Field editor: a name box plus a grouped type list with the current type
    /// ticked (Feishu's 编辑字段). The primary column's type is locked.
    fn dt_field_editor(&self, fidx: usize, pos: Point<Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let Some(dt) = self.datatable.as_ref() else { return div().into_any_element() };
        let Some(field) = dt.table.fields.get(fidx).cloned() else { return div().into_any_element() };
        let input = dt.field_input.clone();
        let is_primary = fidx == 0;

        let mut types = div().id("dt-fe-types").flex_none().max_h(px(300.)).overflow_y_scroll().pb(px(4.));
        let mut group: Option<FieldGroup> = None;
        for &ty in FieldType::all() {
            if group != Some(ty.group()) {
                group = Some(ty.group());
                types = types.child(div().px(px(12.)).pt(px(8.)).pb(px(4.)).text_size(px(11.)).text_color(Theme::text_placeholder())
                    .child(ty.group().label()));
            }
            let on = ty == field.ty;
            let mut r = div().id(SharedString::from(format!("dt-fe-{}", ty.label()))).flex().items_center().gap(px(8.)).h(px(32.)).px(px(12.)).mx(px(4.)).rounded(px(6.))
                .when(!is_primary, |d| d.cursor_pointer().hover(|s| s.bg(Theme::fill_hover())))
                .child(icon(field_glyph(ty), 15., if on { Theme::primary() } else { Theme::text_caption() }))
                .child(div().flex_1().text_size(px(13.)).text_color(if is_primary && !on { Theme::text_disabled() } else { Theme::text() }).child(ty.label()))
                .when(on, |d| d.child(icon("check", 15., Theme::primary())));
            if !is_primary {
                r = r.on_click(cx.listener(move |w, _, _, cx| w.change_field_type(fidx, ty, cx)));
            }
            types = types.child(r);
        }

        deferred(anchored().anchor(Corner::TopLeft).position(pos).snap_to_window_with_margin(px(8.)).child(
            popover().id("dt-field-editor").occlude().w(px(240.)).flex().flex_col()
                .on_mouse_down_out(cx.listener(|w, _, _, cx| { w.commit_field_rename(cx); if let Some(dt) = w.datatable.as_mut() { dt.field_editor = None; } cx.notify(); }))
                .child(div().flex_none().m(px(8.)).h(px(32.)).px(px(8.)).flex().items_center()
                    .rounded(px(6.)).border_1().border_color(Theme::line_border())
                    .child(div().flex_1().child(input)))
                .child(div().px(px(12.)).pb(px(2.)).text_size(px(11.)).text_color(Theme::text_placeholder()).child("字段类型"))
                .child(types),
        )).with_priority(6).into_any_element()
    }

    /// Feishu-style month calendar for a date cell.
    fn dt_date_picker(&self, rec: usize, fidx: usize, pos: Point<Pixels>, y: i64, m: u32, cx: &mut Context<Self>) -> AnyElement {
        use crate::time::{civil, days_in_month, now, weekday};
        let dt = self.datatable.as_ref();
        let fid = dt.and_then(|dt| dt.table.fields.get(fidx)).map(|f| f.id.clone()).unwrap_or_default();
        // the day currently stored in this cell (for the selected ring)
        let sel = dt.and_then(|dt| match dt.table.records[rec].get(&fid) {
            CellValue::Date(ms) => { let (yy, mm, dd, ..) = civil((*ms / 1000).max(0) as u64); Some((yy, mm, dd)) }
            _ => None,
        });
        let (ty, tm, td, ..) = civil(now()); // today
        let lead = weekday(y, m, 1) as usize; // 0=Mon leading blanks
        let ndays = days_in_month(y, m);
        let prev = move || if m == 1 { (y - 1, 12) } else { (y, m - 1) };
        let next = move || if m == 12 { (y + 1, 1) } else { (y, m + 1) };

        let (pv, nx) = (prev(), next());
        let nav_prev = div().id("dt-date-prev").size(px(24.)).flex().items_center().justify_center().rounded(px(4.)).cursor_pointer()
            .hover(|s| s.bg(Theme::fill_hover())).child(icon("chevron-left", 15., Theme::icon()))
            .on_click(cx.listener(move |w, _, _, cx| {
                if let Some(dt) = w.datatable.as_mut() && let Some(dm) = dt.date_menu.as_mut() { dm.3 = pv.0; dm.4 = pv.1; }
                cx.notify();
            }));
        let nav_next = div().id("dt-date-next").size(px(24.)).flex().items_center().justify_center().rounded(px(4.)).cursor_pointer()
            .hover(|s| s.bg(Theme::fill_hover())).child(icon("chevron-right", 15., Theme::icon()))
            .on_click(cx.listener(move |w, _, _, cx| {
                if let Some(dt) = w.datatable.as_mut() && let Some(dm) = dt.date_menu.as_mut() { dm.3 = nx.0; dm.4 = nx.1; }
                cx.notify();
            }));
        let mut grid = div().flex().flex_wrap().px(px(8.));
        for _ in 0..lead {
            grid = grid.child(div().w(px(32.)).h(px(30.)));
        }
        for d in 1..=ndays {
            let is_sel = sel == Some((y, m, d));
            let is_today = (y, m, d) == (ty, tm, td);
            let (fid2, yy, mm) = (fid.clone(), y, m);
            grid = grid.child(
                div().id(("dt-day", d as usize)).w(px(32.)).h(px(30.)).flex().items_center().justify_center()
                    .child(div().size(px(26.)).flex().items_center().justify_center().rounded(px(13.)).cursor_pointer()
                        .when(is_sel, |e| e.bg(Theme::primary()))
                        .when(!is_sel && is_today, |e| e.border_1().border_color(Theme::primary()))
                        .when(!is_sel, |e| e.hover(|s| s.bg(Theme::fill_hover())))
                        .text_size(px(13.)).text_color(if is_sel { gpui::white() } else if is_today { Theme::primary() } else { Theme::text() })
                        .child(format!("{d}")))
                    .on_click(cx.listener(move |w, _, _, cx| w.set_date(rec, fid2.clone(), yy, mm, d, cx))),
            );
        }
        let week = ["一", "二", "三", "四", "五", "六", "日"];
        deferred(anchored().anchor(Corner::TopLeft).position(pos).snap_to_window_with_margin(px(8.)).child(
            popover().id("dt-date-menu").occlude().w(px(252.)).py(px(8.))
                .on_mouse_down_out(cx.listener(|w, _, _, cx| { if let Some(dt) = w.datatable.as_mut() { dt.date_menu = None; } cx.notify(); }))
                .child(div().flex().items_center().justify_between().px(px(12.)).pb(px(6.))
                    .child(nav_prev)
                    .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(Theme::text()).child(format!("{y}年{m}月")))
                    .child(nav_next))
                .child(div().flex().px(px(8.)).pb(px(2.)).children(week.into_iter().map(|w| {
                    div().w(px(32.)).h(px(24.)).flex().items_center().justify_center()
                        .text_size(px(12.)).text_color(Theme::text_placeholder()).child(w)
                })))
                .child(grid),
        )).with_priority(5).into_any_element()
    }

    /// The single-select dropdown: a search/create box, the (filtered) options
    /// with a per-option editor entry, and — when an option is being edited —
    /// an in-place recolor/delete panel.
    fn dt_select_menu(&self, rec: usize, fidx: usize, pos: Point<Pixels>, cx: &mut Context<Self>) -> AnyElement {
        let Some(dt) = self.datatable.as_ref() else { return div().into_any_element() };
        let Some(field) = dt.table.fields.get(fidx).cloned() else { return div().into_any_element() };
        let fid = field.id.clone();
        let current: Vec<String> = match dt.table.records.get(rec).map(|r| r.get(&field.id)) {
            Some(CellValue::Select(ids)) => ids.clone(),
            _ => Vec::new(),
        };
        let close = cx.listener(|w, _, _, cx| { if let Some(dt) = w.datatable.as_mut() { dt.select_menu = None; dt.opt_edit = None; } cx.notify(); });

        // ---- option editor (recolor / delete) ----
        if let Some(oid) = dt.opt_edit.clone() {
            let opt = field.config.options.iter().find(|o| o.id == oid).cloned();
            let mut panel = div().flex().flex_col();
            panel = panel
                .child(div().flex().items_center().gap(px(6.)).h(px(36.)).px(px(8.))
                    .child(div().id("dt-opt-back").size(px(24.)).flex().items_center().justify_center().rounded(px(4.)).cursor_pointer()
                        .hover(|s| s.bg(Theme::fill_hover())).child(icon("chevron-left", 15., Theme::icon()))
                        .on_click(cx.listener(|w, _, _, cx| { if let Some(dt) = w.datatable.as_mut() { dt.opt_edit = None; } cx.notify(); })))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(Theme::text()).child("编辑选项")));
            if let Some(opt) = opt {
                panel = panel.child(div().flex().items_center().justify_center().py(px(6.)).child(select_pill(&opt)));
                let mut swatches = div().flex().flex_wrap().gap(px(10.)).px(px(14.)).py(px(6.));
                for slot in 0..SELECT_PALETTE.len() as u8 {
                    let (bg, _) = select_colors(slot);
                    let is = opt.color == slot;
                    let oid2 = oid.clone();
                    swatches = swatches.child(
                        div().id(("dt-sw", slot as usize)).size(px(22.)).rounded(px(11.)).bg(bg).cursor_pointer()
                            .when(is, |d| d.border_2().border_color(Theme::primary()))
                            .on_click(cx.listener(move |w, _, _, cx| w.recolor_option(fidx, oid2.clone(), slot, cx))),
                    );
                }
                let oid3 = oid.clone();
                panel = panel.child(swatches).child(dt_menu_divider())
                    .child(div().id("dt-opt-del").flex().items_center().gap(px(8.)).h(px(32.)).mx(px(4.)).px(px(8.)).rounded(px(6.)).cursor_pointer()
                        .hover(|s| s.bg(Theme::fill_hover())).child(icon("bin", 15., Theme::danger()))
                        .child(div().text_size(px(13.)).text_color(Theme::danger()).child("删除选项"))
                        .on_click(cx.listener(move |w, _, _, cx| w.delete_option(fidx, oid3.clone(), cx))));
            }
            return deferred(anchored().anchor(Corner::TopLeft).position(pos).snap_to_window_with_margin(px(8.)).child(
                popover().id("dt-opt-editor").occlude().w(px(220.)).flex().flex_col().pb(px(6.))
                    .on_mouse_down_out(close).child(panel),
            )).with_priority(6).into_any_element();
        }

        // ---- option list + search/create ----
        let is_multi = matches!(field.ty, FieldType::MultiSelect);
        let query = dt.menu_input.read(cx).text.trim().to_lowercase();
        let create_label = dt.menu_input.read(cx).text.trim().to_string();
        let exact = field.config.options.iter().any(|o| o.name.to_lowercase() == query);
        let show_create = !query.is_empty() && !exact;
        let input = dt.menu_input.clone();
        let filtered: Vec<SelectOption> = field.config.options.iter()
            .filter(|o| query.is_empty() || o.name.to_lowercase().contains(&query)).cloned().collect();

        let mut list = div().id("dt-sel-scroll").flex_none().max_h(px(240.)).overflow_y_scroll().pb(px(4.));
        for o in &filtered {
            let (fid2, oid) = (fid.clone(), o.id.clone());
            let on = current.contains(&o.id);
            let oid_edit = o.id.clone();
            list = list.child(
                div().id(SharedString::from(format!("dt-opt-{}", o.id))).group("dt-optrow").flex().items_center().gap(px(6.)).h(px(34.)).mx(px(4.)).px(px(8.)).rounded(px(6.)).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover()))
                    .child(div().flex_1().min_w_0().child(select_pill(o)))
                    .when(on, |d| d.child(icon("check", 15., Theme::primary())))
                    .child(div().id(SharedString::from(format!("dt-opte-{}", o.id))).invisible().group_hover("dt-optrow", |s| s.visible())
                        .size(px(22.)).flex().items_center().justify_center().rounded(px(4.)).hover(|s| s.bg(Theme::fill_active()))
                        .child(icon("ellipsis", 15., Theme::icon()))
                        .on_mouse_down(MouseButton::Left, cx.listener(move |w, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(dt) = w.datatable.as_mut() { dt.opt_edit = Some(oid_edit.clone()); }
                            cx.notify();
                        })))
                    .on_click(cx.listener(move |w, _, _, cx| {
                        if is_multi { w.toggle_select(rec, fid2.clone(), oid.clone(), cx); }
                        else { w.pick_select(rec, fid2.clone(), oid.clone(), cx); }
                    }))
                    .into_any_element(),
            );
        }
        if show_create {
            list = list.child(
                div().id("dt-opt-create").flex().items_center().gap(px(6.)).h(px(34.)).mx(px(4.)).px(px(8.)).rounded(px(6.)).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover()))
                    .child(icon("plus", 14., Theme::primary()))
                    .child(div().text_size(px(13.)).text_color(Theme::text_caption()).child("新建"))
                    .child(select_pill(&SelectOption::new(&create_label, (field.config.options.len() % SELECT_PALETTE.len()) as u8)))
                    .on_click(cx.listener(|w, _, _, cx| w.create_select_option(cx)))
                    .into_any_element(),
            );
        }

        deferred(anchored().anchor(Corner::TopLeft).position(pos).snap_to_window_with_margin(px(8.)).child(
            popover().id("dt-select-menu").occlude().w(px(224.)).flex().flex_col()
                .on_mouse_down_out(close)
                .child(div().flex_none().m(px(8.)).h(px(32.)).px(px(8.)).flex().items_center().gap(px(6.))
                    .rounded(px(6.)).bg(crate::feishu_theme::c(ud::N100))
                    .child(icon("search", 13., Theme::text_placeholder()))
                    .child(div().flex_1().child(input)))
                .child(list),
        )).with_priority(5).into_any_element()
    }

    fn dt_type_list(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let mut group: Option<FieldGroup> = None;
        for &ty in FieldType::all() {
            if group != Some(ty.group()) {
                group = Some(ty.group());
                out.push(div().px(px(12.)).pt(px(8.)).pb(px(4.)).text_size(px(11.)).text_color(Theme::text_placeholder())
                    .child(ty.group().label()).into_any_element());
            }
            out.push(
                div().id(SharedString::from(format!("dt-ft-{}", ty.label()))).flex().items_center().gap(px(8.)).h(px(32.)).px(px(12.)).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover()))
                    .child(icon(field_glyph(ty), 15., Theme::text_caption()))
                    .child(div().text_size(px(13.)).text_color(Theme::text()).child(ty.label()))
                    .on_click(cx.listener(move |w, _, _, cx| w.add_field_of(ty, cx)))
                    .into_any_element(),
            );
        }
        out
    }
}

fn dt_divider() -> Div {
    div().flex_none().w(px(1.)).h(px(16.)).mx(px(4.)).bg(Theme::line_border())
}

fn dt_menu_divider() -> Div {
    div().flex_none().h(px(1.)).my(px(4.)).mx(px(8.)).bg(Theme::line_border())
}

fn checkbox_box(on: bool) -> impl IntoElement {
    div().size(px(16.)).rounded(px(4.)).flex().items_center().justify_center()
        .when(on, |d| d.bg(Theme::primary()))
        .when(!on, |d| d.border_1().border_color(Theme::block_placeholder()))
        .when(on, |d| d.child(icon("check", 12., gpui::white())))
}

fn select_pill(opt: &SelectOption) -> impl IntoElement {
    let (bg, fg) = select_colors(opt.color);
    div().flex_none().px(px(8.)).py(px(2.)).rounded(px(4.)).bg(bg)
        .text_size(px(12.)).line_height(px(18.)).text_color(fg)
        .child(ui_text(opt.name.clone()))
}

fn fmt_date(ms: i64) -> String {
    let secs = (ms / 1000).max(0) as u64;
    let (y, m, d, _, _) = crate::time::civil(secs);
    format!("{y}/{m:02}/{d:02}")
}

// ---- demo data (stage 1) ----

fn demo_table() -> Table {
    let mut t = Table::blank("任务管理");
    // primary becomes 任务名称
    if let Some(p) = t.fields.first_mut() {
        p.name = "任务名称".into();
    }
    let pid = t.fields[0].id.clone();

    let status = t.add_field("状态", FieldType::SingleSelect);
    let (s_todo, s_doing, s_done);
    {
        let f = t.field_mut(&status).unwrap();
        let a = SelectOption::new("未开始", 0);
        let b = SelectOption::new("进行中", 6);
        let c = SelectOption::new("已完成", 4);
        (s_todo, s_doing, s_done) = (a.id.clone(), b.id.clone(), c.id.clone());
        f.config.options = vec![a, b, c];
        f.width = 120.;
    }
    let owner = t.add_field("负责人", FieldType::Person);
    let due = t.add_field("截止日期", FieldType::Date);
    let tags = t.add_field("标签", FieldType::MultiSelect);
    let (g_fe, g_be, g_design, g_doc);
    {
        let f = t.field_mut(&tags).unwrap();
        let a = SelectOption::new("前端", 6);
        let b = SelectOption::new("后端", 4);
        let c = SelectOption::new("设计", 8);
        let d = SelectOption::new("文档", 2);
        (g_fe, g_be, g_design, g_doc) = (a.id.clone(), b.id.clone(), c.id.clone(), d.id.clone());
        f.config.options = vec![a, b, c, d];
        f.width = 160.;
    }
    let done = t.add_field("已验收", FieldType::Checkbox);
    t.field_mut(&done).unwrap().width = 90.;
    let prio = t.add_field("优先级", FieldType::Rating);
    t.field_mut(&prio).unwrap().width = 120.;

    let rows = [
        ("接入版本历史后端", &s_done, "舒斯达", 1_762_900_000_000i64, vec![&g_be], true, 3u8),
        ("还原飞书多维表格 UI", &s_doing, "舒斯达", 1_763_200_000_000, vec![&g_fe, &g_design], false, 5),
        ("数据表字段类型体系", &s_done, "林道琰", 1_762_700_000_000, vec![&g_fe, &g_be], true, 4),
        ("内联 diff 高亮 (stage 2)", &s_todo, "刘心怡", 1_763_500_000_000, vec![&g_fe], false, 2),
        ("store 持久化接入", &s_todo, "舒斯达", 1_763_700_000_000, vec![&g_be, &g_doc], false, 4),
    ];
    for (i, (name, st, who, date, tg, ok, stars)) in rows.into_iter().enumerate() {
        let rid = t.records[i].id.clone();
        t.set_cell(&rid, &pid, CellValue::Text(name.into()));
        t.set_cell(&rid, &status, CellValue::Select(vec![st.clone()]));
        t.set_cell(&rid, &owner, CellValue::Person(vec![who.into()]));
        t.set_cell(&rid, &due, CellValue::Date(date));
        t.set_cell(&rid, &tags, CellValue::Select(tg.into_iter().cloned().collect()));
        t.set_cell(&rid, &done, CellValue::Checkbox(ok));
        t.set_cell(&rid, &prio, CellValue::Rating(stars));
    }
    t
}
