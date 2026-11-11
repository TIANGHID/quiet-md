//! 版本历史: a full-screen page that previews a past version of the open
//! document in the centre (read-only) with the version list in a right-hand
//! drawer. Versions come from the library's local snapshot store
//! (`fdl_core::history`); 恢复此版本 writes the selected one back as a new save.
//! Inline diff highlighting is a later pass; this is the frame and the roll-back.

use crate::app::{HistRow, HistoryView, View, Workspace};
use crate::editor::view::Editor;
use crate::model::Document;
use crate::store::PageWidth;
use crate::theme::{self, Theme};
use crate::ui::{ToastKind, TipExt, TitleBarExt, icon};
use gpui::{prelude::FluentBuilder, *};

const DRAWER_W: f32 = 300.;

/// A version's timestamp as a short local label: 今天 / 昨天 / 日期 + 时分.
fn version_label(at_ms: u64) -> String {
    let secs = at_ms / 1000;
    let (y, m, d, h, mi) = crate::time::civil(secs);
    let (ty, tm, td, _, _) = crate::time::civil(crate::time::now());
    let clock = format!("{h:02}:{mi:02}");
    let today = crate::time::days_from_civil(y, m, d);
    let now_day = crate::time::days_from_civil(ty, tm, td);
    if today == now_day {
        crate::localized_format!("今天 {}", "Today {}", clock)
    } else if today == now_day - 1 {
        crate::localized_format!("昨天 {}", "Yesterday {}", clock)
    } else if y == ty {
        format!("{m:02}/{d:02} {clock}")
    } else {
        format!("{y}/{m:02}/{d:02} {clock}")
    }
}

impl Workspace {
    /// Open 版本历史 for `id`: flush pending edits (so 当前版本 and the latest
    /// snapshot are current), build the version list, and preview the top one.
    pub(crate) fn open_history(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.flush(cx);
        let id = id.to_string();
        let meta = self.entries.iter().find(|e| e.id == id).map(|e| e.meta.clone());
        let width = meta.as_ref().map(|m| m.width).unwrap_or_default();
        let markdown = meta.as_ref().is_some_and(|m| m.markdown);
        let modified = meta.as_ref().map_or_else(crate::time::now, |m| m.modified);
        let cur = self.store.load(&id).unwrap_or_default();
        let title = cur.title.text.clone();
        // Current version first, then the snapshots (already newest-first).
        let mut rows = vec![HistRow { at_ms: modified.saturating_mul(1000), path: None }];
        rows.extend(self.store.snapshots(&id).into_iter().map(|s| HistRow { at_ms: s.at_ms, path: Some(s.path) }));
        let preview = self.preview_editor(cur, modified, width, markdown, window, cx);
        self.history = Some(HistoryView { id, title, rows, selected: 0, preview, width, markdown });
        self.view = View::History;
        cx.notify();
    }

    /// A read-only editor rendering `doc` as a past version looks.
    fn preview_editor(&self, doc: Document, modified: u64, width: PageWidth, markdown: bool, window: &mut Window, cx: &mut Context<Self>) -> Entity<Editor> {
        cx.new(|cx| {
            let mut e = Editor::new(doc, modified, width, window, cx);
            e.markdown = markdown;
            e.set_read_only(true, cx);
            e
        })
    }

    /// Preview row `i` in the centre.
    pub(crate) fn history_select(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(h) = self.history.as_ref() else { return };
        if i == h.selected || i >= h.rows.len() { return; }
        let (id, width, markdown) = (h.id.clone(), h.width, h.markdown);
        let secs = h.rows[i].at_ms / 1000;
        let doc = match &h.rows[i].path {
            None => self.store.load(&id).unwrap_or_default(),
            Some(p) => self.store.snapshot_doc(&id, p).unwrap_or_default(),
        };
        let preview = self.preview_editor(doc, secs, width, markdown, window, cx);
        let h = self.history.as_mut().unwrap();
        h.selected = i;
        h.preview = preview;
        cx.notify();
    }

    /// 恢复此版本: write the selected snapshot back as the current version (a
    /// normal save, so the pre-restore state is itself kept in history), push
    /// it into the open editor, and return to the document.
    pub(crate) fn restore_history(&mut self, cx: &mut Context<Self>) {
        let Some(h) = self.history.as_ref() else { return };
        let id = h.id.clone();
        let Some(path) = h.rows.get(h.selected).and_then(|r| r.path.clone()) else { return };
        match self.store.restore_snapshot(&id, &path) {
            Ok(doc) => {
                if self.current.as_deref() == Some(&id)
                    && let Some(ed) = self.editor.clone() {
                    ed.update(cx, |e, cx| e.receive_document(&crate::editor::core::Core::new(doc), cx));
                }
                self.history = None;
                self.view = View::Doc;
                self.entries = self.store.list();
                self.sort();
                self.show_toast_kind(ToastKind::Success, crate::settings::text("已恢复到所选版本"), cx);
                cx.notify();
            }
            Err(e) => self.show_toast_kind(ToastKind::Error, &crate::localized_format!("恢复失败：{}", "Could not restore: {}", crate::settings::diagnostic(&e.to_string())), cx),
        }
    }

    pub(crate) fn close_history(&mut self, cx: &mut Context<Self>) {
        self.history = None;
        self.view = View::Doc;
        cx.notify();
    }

    pub(crate) fn render_history(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(h) = self.history.as_ref() else { return div().into_any_element() };
        let preview = h.preview.clone();
        let selected = h.selected;
        let can_restore = h.rows.get(selected).is_some_and(|r| r.path.is_some());
        let title = crate::settings::display_title(&h.title);

        // The version drawer rows (single level, newest first, 当前版本 on top).
        let rows: Vec<AnyElement> = h.rows.iter().enumerate().map(|(i, r)| {
            let on = i == selected;
            let current = r.path.is_none();
            div()
                .id(("hist-row", i))
                .px(px(12.)).py(px(9.))
                .flex().flex_col().gap(px(2.))
                .rounded(px(8.))
                .cursor_pointer()
                .when(on, |d| d.bg(Theme::sidebar_selected()))
                .when(!on, |d| d.hover(|s| s.bg(Theme::fill_hover())))
                .child(
                    div().flex().items_center().gap(px(6.))
                        .child(div().text_size(px(13.)).line_height(px(18.))
                            .font_weight(if on { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                            .text_color(Theme::text()).child(version_label(r.at_ms)))
                        .when(current, |d| d.child(
                            div().px(px(5.)).py(px(1.)).rounded(px(4.)).bg(Theme::primary_500().opacity(0.14))
                                .text_size(px(10.)).line_height(px(13.)).text_color(Theme::primary())
                                .child(crate::settings::text("当前版本")))),
                )
                .on_click(cx.listener(move |w, _, window, cx| w.history_select(i, window, cx)))
                .into_any_element()
        }).collect();

        let drawer = div()
            .flex_none().w(px(DRAWER_W)).h_full()
            .flex().flex_col()
            .bg(Theme::bg_body())
            .border_l_1().border_color(Theme::line_border())
            .child(
                div().flex_none().h(px(crate::app::HEADER_H)).px(px(16.)).flex().items_center()
                    .child(div().text_size(px(14.)).line_height(px(20.)).font_weight(FontWeight::SEMIBOLD)
                        .text_color(Theme::text()).child(crate::settings::text("版本历史"))),
            )
            .child(
                div().id("hist-list").flex_1().min_h_0().overflow_y_scroll().px(px(8.)).pb(px(8.))
                    .children(rows),
            )
            .child(
                div().flex_none().px(px(12.)).py(px(12.)).border_t_1().border_color(Theme::line_border())
                    .child(
                        div().id("hist-restore")
                            .h(px(34.)).flex().items_center().justify_center()
                            .rounded(px(9.))
                            .when(can_restore, |d| d.bg(Theme::primary()).cursor_pointer()
                                .hover(|s| s.opacity(0.88))
                                .text_color(white()))
                            .when(!can_restore, |d| d.bg(Theme::fill_hover()).text_color(Theme::text_disabled()))
                            .text_size(px(13.)).font_weight(FontWeight::MEDIUM)
                            .child(crate::settings::text("恢复此版本"))
                            .when(can_restore, |d| d.on_click(cx.listener(|w, _, _, cx| w.restore_history(cx)))),
                    ),
            );

        // Top bar spanning the whole page: 返回, then the document's title.
        let top = div()
            .flex_none().h(px(crate::app::HEADER_H)).flex().items_center()
            .pl(px(if self.sidebar_open { 16. } else { theme::TRAFFIC_LIGHTS_W }))
            .pr(px(crate::ui::header_right_inset()))
            .border_b_1().border_color(Theme::line_border())
            .title_bar()
            .child(
                div().id("hist-back").no_drag().flex_none().size(px(28.)).ml(px(4.)).mr(px(10.))
                    .flex().items_center().justify_center().rounded(px(8.)).cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover_strong()))
                    .tip(crate::settings::tr("返回", "Back"), None)
                    .child(icon("arrow-left", 17., Theme::icon()))
                    .on_click(cx.listener(|w, _, _, cx| w.close_history(cx))),
            )
            .child(div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD)
                .text_color(Theme::text()).child(title))
            .child(div().flex_1());

        div()
            .flex_1().min_w_0().h_full().flex().flex_col()
            .child(top)
            .child(
                div().flex_1().min_h_0().flex()
                    .child(div().relative().flex_1().min_w_0().h_full()
                        .bg(crate::editor_theme::Theme::bg_body())
                        .child(preview))
                    .child(drawer),
            )
            .into_any_element()
    }
}
