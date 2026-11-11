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
        Text | Phone | Email | Barcode | Location | Signature => "file-text",
        SingleSelect => "chevron-down",
        MultiSelect => "list",
        Checkbox => "todo-check",
        Number | Currency | AutoNumber => "sigma",
        Formula => "sigma",
        Rating => "star",
        Date | CreatedTime | ModifiedTime => "info",
        Url => "export",
        Person | Group | CreatedBy | ModifiedBy => "home",
        Progress => "view-list",
        _ => "file-text",
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
    /// Add-field popover: where it hangs (None = closed).
    pub add_field: Option<Point<Pixels>>,
    /// Single/multi-select dropdown: (record, field, anchor).
    pub select_menu: Option<(usize, usize, Point<Pixels>)>,
    /// 行高 menu anchor.
    pub row_height_menu: Option<Point<Pixels>>,
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
        self.datatable = Some(DataTableView {
            table,
            view_idx: 0,
            sel: None,
            editing: None,
            input,
            add_field: None,
            select_menu: None,
            row_height_menu: None,
        });
        self.view = View::DataTable;
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

    fn add_record(&mut self, cx: &mut Context<Self>) {
        if let Some(dt) = self.datatable.as_mut() {
            dt.table.add_record();
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
        let tabs = self.dt_tab_bar(cx);
        let toolbar = self.dt_toolbar(cx);
        let grid = self.dt_grid(&fields, row_h, window, cx);

        let mut page = div()
            .flex_1().min_w_0().h_full().flex().flex_col().bg(Theme::bg_body())
            .child(top)
            .child(tabs)
            .child(toolbar)
            .child(grid);
        for overlay in self.dt_overlays(cx) {
            page = page.child(overlay);
        }
        page.into_any_element()
    }

    fn dt_titlebar(&self, title: &str, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_none().h(px(crate::app::HEADER_H)).flex().items_center()
            .pl(px(if self.sidebar_open { 16. } else { crate::feishu_theme::TRAFFIC_LIGHTS_W }))
            .pr(px(crate::ui::header_right_inset()))
            .border_b_1().border_color(Theme::line_border())
            .child(
                div().id("dt-back").flex_none().size(px(28.)).ml(px(4.)).mr(px(8.))
                    .flex().items_center().justify_center().rounded(px(8.)).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover_strong()))
                    .child(icon("arrow-left", 17., Theme::icon()))
                    .on_click(cx.listener(|w, _, _, cx| w.close_datatable(cx))),
            )
            .child(icon("storage", 18., Theme::primary()))
            .child(div().ml(px(8.)).text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD)
                .text_color(Theme::text()).child(ui_text(title.to_string())))
            .child(div().flex_1())
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

        // header row
        let mut header = div().flex_none().flex().h(px(HEADER_H)).bg(Theme::table_header())
            .border_b_1().border_color(Theme::table_line())
            .child(
                div().flex_none().w(px(GUTTER_W)).h_full().flex().items_center().justify_center()
                    .border_r_1().border_color(Theme::table_line())
                    .child(checkbox_box(false)),
            );
        for f in fields {
            header = header.child(
                div().flex_none().w(px(f.width)).h_full().flex().items_center().gap(px(6.)).px(px(CELL_PAD_X))
                    .border_r_1().border_color(Theme::table_line())
                    .child(icon(field_glyph(f.ty), 14., Theme::text_caption()))
                    .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis()
                        .text_size(px(13.)).text_color(Theme::text()).child(ui_text(f.name.clone()))),
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

        // body rows
        let mut body = div().id("dt-body").flex_1().min_h_0().overflow_scroll().flex().flex_col();
        for (ri, rec) in records.iter().enumerate() {
            let mut row = div().flex_none().flex().h(px(row_h)).hover(|s| s.bg(Theme::block_bg_area()))
                .border_b_1().border_color(Theme::table_line())
                .child(
                    div().flex_none().w(px(GUTTER_W)).h_full().flex().items_center().justify_center()
                        .border_r_1().border_color(Theme::table_line())
                        .text_size(px(12.)).text_color(Theme::text_placeholder())
                        .child(format!("{}", ri + 1)),
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

        div().flex_1().min_h_0().flex().flex_col().child(header).child(body).into_any_element()
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
                let fid = field.id.clone();
                let mut row = div().id(("dt-sel", ri)).flex().items_center().gap(px(4.)).w_full().cursor_pointer();
                for id in ids {
                    if let Some(opt) = field.option(id) {
                        row = row.child(select_pill(opt));
                    }
                }
                row.on_click(cx.listener(move |w, ev: &ClickEvent, _, cx| {
                    let p = ev.position();
                    if let Some(dt) = w.datatable.as_mut() { dt.select_menu = Some((ri, 0, point(p.x, p.y + px(6.)))); let _ = &fid; }
                    cx.notify();
                })).into_any_element()
            }
            (FieldType::SingleSelect | FieldType::MultiSelect, _) => {
                // empty select: clicking opens the option menu
                let fidx = self.datatable.as_ref().and_then(|dt| dt.table.fields.iter().position(|f| f.id == field.id)).unwrap_or(0);
                div().id(("dt-sel-empty", ri)).w_full().h_full().cursor_pointer()
                    .on_click(cx.listener(move |w, ev: &ClickEvent, _, cx| {
                        let p = ev.position();
                        if let Some(dt) = w.datatable.as_mut() { dt.select_menu = Some((ri, fidx, point(p.x, p.y + px(6.)))); }
                        cx.notify();
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
            (FieldType::Date, CellValue::Date(ms)) => {
                div().text_size(px(13.)).text_color(Theme::text()).child(ui_text(fmt_date(*ms))).into_any_element()
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
                popover().id("dt-field-types").occlude().w(px(220.)).max_h(px(420.)).overflow_hidden().flex().flex_col()
                    .on_mouse_down_out(cx.listener(|w, _, _, cx| { if let Some(dt) = w.datatable.as_mut() { dt.add_field = None; } cx.notify(); }))
                    .child(div().id("dt-type-scroll").flex_1().min_h_0().overflow_y_scroll().py(px(4.))
                        .children(self.dt_type_list(cx))),
            )).with_priority(5).into_any_element());
        }

        if let Some((rec, fidx, pos)) = dt.select_menu {
            let field = dt.table.fields.get(fidx).cloned();
            if let Some(field) = field {
                let fid = field.id.clone();
                out.push(deferred(anchored().anchor(Corner::TopLeft).position(pos).snap_to_window_with_margin(px(8.)).child(
                    popover().id("dt-select-menu").occlude().w(px(200.)).py(px(4.))
                        .on_mouse_down_out(cx.listener(|w, _, _, cx| { if let Some(dt) = w.datatable.as_mut() { dt.select_menu = None; } cx.notify(); }))
                        .children(field.config.options.iter().map(|o| {
                            let (fid, oid) = (fid.clone(), o.id.clone());
                            div().id(SharedString::from(format!("dt-opt-{}", o.id))).flex().items_center().h(px(32.)).px(px(10.)).cursor_pointer()
                                .hover(|s| s.bg(Theme::fill_hover()))
                                .child(select_pill(o))
                                .on_click(cx.listener(move |w, _, _, cx| w.pick_select(rec, fid.clone(), oid.clone(), cx)))
                                .into_any_element()
                        })),
                )).with_priority(5).into_any_element());
            }
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
        out
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
    let done = t.add_field("已验收", FieldType::Checkbox);
    t.field_mut(&done).unwrap().width = 90.;
    let prio = t.add_field("优先级", FieldType::Rating);
    t.field_mut(&prio).unwrap().width = 120.;

    let rows = [
        ("接入版本历史后端", &s_done, "舒斯达", 1_762_900_000_000i64, true, 3u8),
        ("还原飞书多维表格 UI", &s_doing, "舒斯达", 1_763_200_000_000, false, 5),
        ("数据表字段类型体系", &s_done, "林道琰", 1_762_700_000_000, true, 4),
        ("内联 diff 高亮 (stage 2)", &s_todo, "刘心怡", 1_763_500_000_000, false, 2),
        ("store 持久化接入", &s_todo, "舒斯达", 1_763_700_000_000, false, 4),
    ];
    for (i, (name, st, who, date, ok, stars)) in rows.into_iter().enumerate() {
        let rid = t.records[i].id.clone();
        t.set_cell(&rid, &pid, CellValue::Text(name.into()));
        t.set_cell(&rid, &status, CellValue::Select(vec![st.clone()]));
        t.set_cell(&rid, &owner, CellValue::Person(vec![who.into()]));
        t.set_cell(&rid, &due, CellValue::Date(date));
        t.set_cell(&rid, &done, CellValue::Checkbox(ok));
        t.set_cell(&rid, &prio, CellValue::Rating(stars));
    }
    t
}
