//! 历史版本: the versions a document has saved on disk, with a plain-text
//! preview and a restore action. Snapshots themselves live in `fdl_core`
//! (`store::list_versions` / `restore_version`).

use crate::app::Workspace;
use crate::store::VersionEntry;
use crate::theme::Theme;
use crate::ui::format_relative;
use gpui::{prelude::FluentBuilder, *};

#[derive(Clone)]
pub struct HistoryPanel {
    pub id: String,
    /// Newest first.
    pub versions: Vec<VersionEntry>,
    pub selected: Option<String>,
    pub preview: String,
}

/// A dialog button whose label comes from `tr` (the shared buttons run their
/// labels through the localize table, which has no entry for these yet).
fn action_button(id: &'static str, label: &'static str, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(30.))
        .px(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(9.))
        .cursor_pointer()
        .text_size(px(13.))
        .line_height(px(20.))
        .font_weight(FontWeight::SEMIBOLD)
        .when(primary, |d| d.bg(Theme::primary()).text_color(gpui::white()))
        .when(!primary, |d| d.border_1().border_color(Theme::button_border()).text_color(Theme::text()))
        .child(label)
}

impl Workspace {
    /// Open from the document's "…" menu: flush first, so the newest version
    /// is what is actually on disk.
    pub fn open_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.flush(cx);
        let Some(id) = self.current.clone() else { return };
        let versions = self.store.list_versions(&id);
        let selected = versions.first().map(|v| v.id.clone());
        let preview = selected.as_deref().map(|v| self.store.version_preview(&id, v)).unwrap_or_default();
        self.history = Some(HistoryPanel { id, versions, selected, preview });
        self.doc_menu = false;
        self.doc_sub = None;
        window.focus(&self.focus);
        cx.notify();
    }

    fn select_history(&mut self, version_id: String, cx: &mut Context<Self>) {
        let Some(panel) = self.history.as_ref() else { return };
        let preview = self.store.version_preview(&panel.id, &version_id);
        if let Some(panel) = self.history.as_mut() {
            panel.selected = Some(version_id);
            panel.preview = preview;
        }
        cx.notify();
    }

    /// Replace the open document with the selected version. The restore
    /// itself keeps the replaced state as one more version.
    pub fn restore_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.history.clone() else { return };
        let Some(version_id) = panel.selected.clone() else { return };
        if !self.flush(cx) {
            self.show_toast_kind(crate::ui::ToastKind::Error, crate::settings::tr("请先保存当前修改", "Save the current changes first"), cx);
            return;
        }
        if !self.document_peers(&panel.id, window, cx).is_empty() {
            self.show_toast_kind(crate::ui::ToastKind::Error, crate::settings::tr("请先关闭正在编辑该文档的其他窗口", "Close other windows editing this document first"), cx);
            return;
        }
        match self.store.restore_version(&panel.id, &version_id) {
            Ok(()) => {
                self.history = None;
                // Rebuild the editor from disk and reset the save state, the
                // same path the 保存失败 dialog uses when giving up changes.
                self.resolve_save_failed(false, window, cx);
                self.show_toast(crate::settings::tr("已恢复到所选版本", "Restored the selected version"), cx);
            }
            Err(e) => self.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx),
        }
        cx.notify();
    }

    pub(crate) fn render_history(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let panel = self.history.clone()?;
        let tr = crate::settings::tr;
        let empty = panel.versions.is_empty();
        let rows: Vec<Stateful<Div>> = panel.versions.iter().map(|v| {
            let version_id = v.id.clone();
            let selected = panel.selected.as_deref() == Some(version_id.as_str());
            let when = format!("{} · {}", format_relative(v.saved), crate::docinfo::format_size(v.size));
            let title = v.title.clone();
            div()
                .id(SharedString::from(format!("history-row-{version_id}")))
                .flex_none()
                .h(px(52.))
                .px(px(12.))
                .mb(px(4.))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(2.))
                .rounded(px(9.))
                .cursor_pointer()
                .when(selected, |d| d.bg(Theme::fill_hover_strong()))
                .child(div().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text()).child(SharedString::from(when)))
                .child(div().text_size(px(11.)).line_height(px(16.)).text_color(Theme::text_caption()).truncate().child(SharedString::from(title)))
                .on_click(cx.listener(move |w, _, _, cx| w.select_history(version_id.clone(), cx)))
        }).collect();
        let preview_lines: Vec<Div> = panel.preview.lines().take(40).map(|line| {
            div().text_size(px(13.)).line_height(px(22.)).text_color(Theme::text()).truncate().child(SharedString::from(line.to_string()))
        }).collect();
        let close = cx.listener(|w, _, _, cx| { w.history = None; cx.notify(); });
        let mask_close = cx.listener(|w, _, _, cx| { w.history = None; cx.notify(); });
        let cancel = action_button("history-cancel", tr("取消", "Cancel"), false)
            .on_click(cx.listener(|w, _, _, cx| { w.history = None; cx.notify(); }));
        let restore = action_button("history-restore", tr("恢复此版本", "Restore this version"), true)
            .on_click(cx.listener(|w, _, window, cx| w.restore_selected(window, cx)));
        let modal = crate::trash::dialog("history-dialog")
            .w(px(820.))
            .h(px(540.))
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div().flex_none().h(px(64.)).px(px(24.)).flex().items_center().justify_between()
                    .border_b_1().border_color(Theme::line_divider())
                    .child(div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).child(tr("历史版本", "Version history")))
                    .child(crate::trash::close_button("history-close").on_click(close)),
            )
            .child(
                div().flex_1().min_h_0().flex()
                    .child(
                        div().flex_none().w(px(260.)).h_full().p(px(12.)).border_r_1().border_color(Theme::line_divider()).flex().flex_col()
                            .child(div().px(px(4.)).pb(px(8.)).text_size(px(11.)).text_color(Theme::text_caption()).child(tr("已保存的版本（最新在前）", "Saved versions (newest first)")))
                            .child(
                                div().flex_1().min_h_0().overflow_y_scroll().flex().flex_col()
                                    .children(rows)
                                    .when(empty, |d| d.child(div().px(px(4.)).pt(px(12.)).text_size(px(12.)).text_color(Theme::text_placeholder()).child(tr("暂无历史版本，保存文档后会自动生成。", "No versions yet; saving the document creates one.")))),
                            ),
                    )
                    .child(
                        div().flex_1().min_w_0().p(px(20.)).overflow_y_scroll().flex().flex_col().gap(px(2.)).children(preview_lines),
                    ),
            )
            .child(
                div().flex_none().h(px(64.)).px(px(24.)).flex().items_center().justify_between().gap(px(16.))
                    .border_t_1().border_color(Theme::line_divider())
                    .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(Theme::text_caption()).truncate()
                        .child(tr("恢复会覆盖当前内容；覆盖前的状态会保留为一个新版本。", "Restoring replaces the current content; the replaced state is kept as a new version.")))
                    .child(div().flex_none().flex().items_center().gap(px(10.)).child(cancel)
                        .when(panel.selected.is_some(), |d| d.child(restore))),
            );
        Some(
            deferred(
                div().id("history-mask").absolute().inset_0().flex().items_center().justify_center()
                    .bg(crate::trash::scrim())
                    .on_mouse_down(MouseButton::Left, mask_close)
                    .child(modal),
            ).with_priority(3).into_any_element(),
        )
    }
}
