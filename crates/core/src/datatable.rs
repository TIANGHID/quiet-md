//! 数据表 (Data table): a Feishu-Bitable-style structured table — a set of
//! typed fields (columns), records (rows), and views over them. This is the
//! engine half (no GPUI): the model, its field taxonomy and cell values, the
//! mutations the UI drives, and JSON persistence. The grid UI lives in the
//! root crate (`datatable_view`).
//!
//! The field taxonomy mirrors Feishu's three groups (常规 / 业务 / 高级). Not
//! every type stores data the same way: text-like types keep a string, selects
//! keep option ids, computed types (公式, 查找引用, 创建时间, …) hold no stored
//! value and are derived. [`CellValue`] covers the stored shapes; derived types
//! render from record/field metadata.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A stable identifier (uuid v4) for a field, record, view or select option.
pub type Id = String;

fn new_id() -> Id {
    uuid::Uuid::new_v4().to_string()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The field (column) type. Grouped as Feishu groups them in the type picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    // 常规 (general)
    Text,
    SingleSelect,
    MultiSelect,
    Person,
    Group,
    Date,
    Attachment,
    Number,
    Checkbox,
    Url,
    Formula,
    Lookup,
    // 业务 (business)
    Flow,
    Button,
    AutoNumber,
    Phone,
    Email,
    Location,
    Barcode,
    Signature,
    Progress,
    Currency,
    Rating,
    // 高级 (advanced)
    TwoWayLink,
    OneWayLink,
    CreatedBy,
    ModifiedBy,
    CreatedTime,
    ModifiedTime,
}

impl FieldType {
    /// The Chinese label shown in the type picker, matching Feishu.
    pub fn label(self) -> &'static str {
        use FieldType::*;
        match self {
            Text => "文本",
            SingleSelect => "单选",
            MultiSelect => "多选",
            Person => "人员",
            Group => "群组",
            Date => "日期",
            Attachment => "附件",
            Number => "数字",
            Checkbox => "复选框",
            Url => "超链接",
            Formula => "公式",
            Lookup => "查找引用",
            Flow => "流程",
            Button => "按钮",
            AutoNumber => "自动编号",
            Phone => "电话号码",
            Email => "Email",
            Location => "地理位置",
            Barcode => "条码",
            Signature => "签字",
            Progress => "进度",
            Currency => "货币",
            Rating => "评分",
            TwoWayLink => "双向关联",
            OneWayLink => "单向关联",
            CreatedBy => "创建人",
            ModifiedBy => "修改人",
            CreatedTime => "创建时间",
            ModifiedTime => "最后更新时间",
        }
    }

    /// The group heading this type sits under in the picker.
    pub fn group(self) -> FieldGroup {
        use FieldType::*;
        match self {
            Text | SingleSelect | MultiSelect | Person | Group | Date | Attachment | Number
            | Checkbox | Url | Formula | Lookup => FieldGroup::General,
            Flow | Button | AutoNumber | Phone | Email | Location | Barcode | Signature
            | Progress | Currency | Rating => FieldGroup::Business,
            TwoWayLink | OneWayLink | CreatedBy | ModifiedBy | CreatedTime | ModifiedTime => {
                FieldGroup::Advanced
            }
        }
    }

    /// Whether the type stores an editable value (vs. being derived/computed).
    pub fn is_editable(self) -> bool {
        use FieldType::*;
        !matches!(
            self,
            Formula | Lookup | AutoNumber | CreatedBy | ModifiedBy | CreatedTime | ModifiedTime
        )
    }

    /// The type picker's order (常规 → 业务 → 高级, as Feishu lists them).
    pub fn all() -> &'static [FieldType] {
        use FieldType::*;
        &[
            Text, SingleSelect, MultiSelect, Person, Group, Date, Attachment, Number, Checkbox,
            Url, Formula, Lookup, Flow, Button, AutoNumber, Phone, Email, Location, Barcode,
            Signature, Progress, Currency, Rating, TwoWayLink, OneWayLink, CreatedBy, ModifiedBy,
            CreatedTime, ModifiedTime,
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldGroup {
    General,
    Business,
    Advanced,
}

impl FieldGroup {
    pub fn label(self) -> &'static str {
        match self {
            FieldGroup::General => "常规",
            FieldGroup::Business => "业务",
            FieldGroup::Advanced => "高级",
        }
    }
}

/// A single/multi-select option: a label plus a palette colour index. The index
/// names a slot in the select colour palette (see `datatable_view`), so themes
/// can resolve it without the model depending on GPUI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectOption {
    pub id: Id,
    pub name: String,
    /// Palette slot (0..N). Wraps if a UI palette is shorter.
    #[serde(default)]
    pub color: u8,
}

impl SelectOption {
    pub fn new(name: impl Into<String>, color: u8) -> Self {
        SelectOption { id: new_id(), name: name.into(), color }
    }
}

/// Per-field configuration that only some types use. Kept flat (and all
/// `#[serde(default)]`) so adding a type never breaks older saved tables.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldConfig {
    /// 单选 / 多选 options, in display order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<SelectOption>,
    /// 数字 / 货币 decimal places.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
    /// 货币 symbol, e.g. "¥".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// 评分 maximum (stars), default 5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_max: Option<u8>,
    /// 日期 format token, e.g. "YYYY/MM/DD".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_format: Option<String>,
}

/// A column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub id: Id,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: FieldType,
    #[serde(default)]
    pub config: FieldConfig,
    /// Column width in px as shown in the default grid view.
    #[serde(default = "default_width")]
    pub width: f32,
}

fn default_width() -> f32 {
    FIELD_DEFAULT_WIDTH
}

/// Feishu's default column width (the first/primary column is wider).
pub const FIELD_DEFAULT_WIDTH: f32 = 140.0;
pub const FIELD_PRIMARY_WIDTH: f32 = 220.0;
pub const FIELD_MIN_WIDTH: f32 = 80.0;

impl Field {
    pub fn new(name: impl Into<String>, ty: FieldType) -> Self {
        Field { id: new_id(), name: name.into(), ty, config: FieldConfig::default(), width: FIELD_DEFAULT_WIDTH }
    }

    pub fn option(&self, id: &str) -> Option<&SelectOption> {
        self.config.options.iter().find(|o| o.id == id)
    }
}

/// A stored cell value. Derived types store [`CellValue::Empty`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", content = "v")]
pub enum CellValue {
    #[default]
    Empty,
    /// 文本 / 电话号码 / Email / 条码 / 地理位置 / 签字 (raw string).
    Text(String),
    /// 数字 / 货币 / 进度.
    Number(f64),
    /// 复选框.
    Checkbox(bool),
    /// 单选 / 多选: option ids, in display order (single keeps at most one).
    Select(Vec<Id>),
    /// 日期: unix milliseconds.
    Date(i64),
    /// 超链接.
    Url { text: String, href: String },
    /// 人员 / 群组: member ids or names.
    Person(Vec<String>),
    /// 评分: number of filled stars.
    Rating(u8),
    /// 关联: ids of linked records (in another table; here just ids).
    Link(Vec<Id>),
}

impl CellValue {
    pub fn is_empty(&self) -> bool {
        match self {
            CellValue::Empty => true,
            CellValue::Text(s) => s.is_empty(),
            CellValue::Select(v) | CellValue::Link(v) => v.is_empty(),
            CellValue::Person(v) => v.is_empty(),
            _ => false,
        }
    }

    /// A plain-text rendering used for search, copy and markdown export.
    pub fn as_plain(&self, field: &Field) -> String {
        match self {
            CellValue::Empty => String::new(),
            CellValue::Text(s) => s.clone(),
            CellValue::Number(n) => format_number(*n, field),
            CellValue::Checkbox(b) => if *b { "✓".into() } else { String::new() },
            CellValue::Select(ids) => ids
                .iter()
                .filter_map(|id| field.option(id).map(|o| o.name.clone()))
                .collect::<Vec<_>>()
                .join(", "),
            CellValue::Date(ms) => ms.to_string(),
            CellValue::Url { text, href } => if text.is_empty() { href.clone() } else { text.clone() },
            CellValue::Person(v) => v.join(", "),
            CellValue::Rating(n) => "★".repeat(*n as usize),
            CellValue::Link(v) => v.join(", "),
        }
    }
}

fn format_number(n: f64, field: &Field) -> String {
    let prec = field.config.precision.unwrap_or(0) as usize;
    let body = format!("{n:.prec$}");
    match &field.config.currency {
        Some(sym) if field.ty == FieldType::Currency => format!("{sym}{body}"),
        _ if field.ty == FieldType::Progress => format!("{body}%"),
        _ => body,
    }
}

/// A row: its cells keyed by field id, plus audit metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: Id,
    #[serde(default)]
    pub cells: HashMap<Id, CellValue>,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub modified: u64,
}

impl Record {
    pub fn new() -> Self {
        let t = now_ms();
        Record { id: new_id(), cells: HashMap::new(), created: t, modified: t }
    }

    pub fn get(&self, field: &str) -> &CellValue {
        self.cells.get(field).unwrap_or(&CellValue::Empty)
    }
}

impl Default for Record {
    fn default() -> Self {
        Self::new()
    }
}

/// Row height presets, as Feishu's 行高 menu offers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RowHeight {
    #[default]
    Short,
    Medium,
    Tall,
    ExtraTall,
}

impl RowHeight {
    pub fn px(self) -> f32 {
        match self {
            RowHeight::Short => 32.0,
            RowHeight::Medium => 56.0,
            RowHeight::Tall => 88.0,
            RowHeight::ExtraTall => 128.0,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            RowHeight::Short => "低",
            RowHeight::Medium => "中",
            RowHeight::Tall => "高",
            RowHeight::ExtraTall => "超高",
        }
    }
}

/// A view over the table. Stage 1 models the 表格 (grid) view; other view kinds
/// (看板 / 甘特 / 画册 / 表单) are future work but named here so the tab bar can
/// list them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub id: Id,
    pub name: String,
    #[serde(default)]
    pub kind: ViewKind,
    /// Field display order; ids not present fall back to table order, appended.
    #[serde(default)]
    pub field_order: Vec<Id>,
    /// Hidden field ids.
    #[serde(default)]
    pub hidden: Vec<Id>,
    #[serde(default)]
    pub row_height: RowHeight,
    /// Number of leading columns kept frozen (the primary column, by default).
    #[serde(default = "one")]
    pub frozen: usize,
}

fn one() -> usize {
    1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ViewKind {
    #[default]
    Grid,
    Kanban,
    Gallery,
    Gantt,
    Form,
}

impl ViewKind {
    pub fn label(self) -> &'static str {
        match self {
            ViewKind::Grid => "表格",
            ViewKind::Kanban => "看板",
            ViewKind::Gallery => "画册",
            ViewKind::Gantt => "甘特图",
            ViewKind::Form => "表单",
        }
    }
}

impl View {
    pub fn grid(name: impl Into<String>) -> Self {
        View {
            id: new_id(),
            name: name.into(),
            kind: ViewKind::Grid,
            field_order: Vec::new(),
            hidden: Vec::new(),
            row_height: RowHeight::Short,
            frozen: 1,
        }
    }
}

/// The whole data table: fields, records and views.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Table {
    #[serde(default = "new_id")]
    pub id: Id,
    pub name: String,
    pub fields: Vec<Field>,
    pub records: Vec<Record>,
    pub views: Vec<View>,
}

impl Default for Table {
    fn default() -> Self {
        Self::blank("未命名多维表格")
    }
}

impl Table {
    /// A blank table like Feishu's: one 文本 primary field, five empty rows and
    /// one grid view.
    pub fn blank(name: impl Into<String>) -> Self {
        let mut primary = Field::new("文本", FieldType::Text);
        primary.width = FIELD_PRIMARY_WIDTH;
        let records = (0..5).map(|_| Record::new()).collect();
        Table {
            id: new_id(),
            name: name.into(),
            fields: vec![primary],
            records,
            views: vec![View::grid("表格")],
        }
    }

    /// The primary (first) field — Feishu's 标题列, never deletable.
    pub fn primary(&self) -> Option<&Field> {
        self.fields.first()
    }

    pub fn field(&self, id: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == id)
    }

    pub fn field_mut(&mut self, id: &str) -> Option<&mut Field> {
        self.fields.iter_mut().find(|f| f.id == id)
    }

    /// Add a field at the end; returns its id.
    pub fn add_field(&mut self, name: impl Into<String>, ty: FieldType) -> Id {
        let f = Field::new(name, ty);
        let id = f.id.clone();
        self.fields.push(f);
        id
    }

    /// Remove a field (and its cells from every record). The primary field is
    /// never removed; returns whether a removal happened.
    pub fn remove_field(&mut self, id: &str) -> bool {
        if self.primary().is_some_and(|p| p.id == id) {
            return false;
        }
        let before = self.fields.len();
        self.fields.retain(|f| f.id != id);
        if self.fields.len() == before {
            return false;
        }
        for r in &mut self.records {
            r.cells.remove(id);
        }
        for v in &mut self.views {
            v.field_order.retain(|f| f != id);
            v.hidden.retain(|f| f != id);
        }
        true
    }

    /// Append an empty record; returns its id.
    pub fn add_record(&mut self) -> Id {
        let r = Record::new();
        let id = r.id.clone();
        self.records.push(r);
        id
    }

    pub fn remove_record(&mut self, id: &str) -> bool {
        let before = self.records.len();
        self.records.retain(|r| r.id != id);
        self.records.len() != before
    }

    pub fn record(&self, id: &str) -> Option<&Record> {
        self.records.iter().find(|r| r.id == id)
    }

    /// Set a cell value, stamping the record's modified time. A no-op (but still
    /// returns true) if the field is missing.
    pub fn set_cell(&mut self, record: &str, field: &str, value: CellValue) -> bool {
        let Some(r) = self.records.iter_mut().find(|r| r.id == record) else {
            return false;
        };
        if value.is_empty() {
            r.cells.remove(field);
        } else {
            r.cells.insert(field.to_string(), value);
        }
        r.modified = now_ms();
        true
    }

    pub fn cell(&self, record: &str, field: &str) -> &CellValue {
        self.record(record).map(|r| r.get(field)).unwrap_or(&CellValue::Empty)
    }

    /// Fields in a view's display order, skipping hidden ones.
    pub fn visible_fields<'a>(&'a self, view: &View) -> Vec<&'a Field> {
        let mut out: Vec<&Field> = Vec::new();
        for id in &view.field_order {
            if !view.hidden.contains(id)
                && let Some(f) = self.field(id)
            {
                out.push(f);
            }
        }
        for f in &self.fields {
            if !view.hidden.contains(&f.id) && !view.field_order.contains(&f.id) {
                out.push(f);
            }
        }
        out
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> anyhow::Result<Table> {
        Ok(serde_json::from_str(s)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_table_matches_feishu_defaults() {
        let t = Table::blank("未命名多维表格");
        assert_eq!(t.fields.len(), 1);
        assert_eq!(t.primary().unwrap().ty, FieldType::Text);
        assert_eq!(t.primary().unwrap().width, FIELD_PRIMARY_WIDTH);
        assert_eq!(t.records.len(), 5);
        assert_eq!(t.views.len(), 1);
        assert_eq!(t.views[0].kind, ViewKind::Grid);
    }

    #[test]
    fn full_field_taxonomy_is_present() {
        // 12 常规 + 11 业务 + 6 高级 = 29 types, matching the live picker.
        assert_eq!(FieldType::all().len(), 29);
        let general = FieldType::all().iter().filter(|t| t.group() == FieldGroup::General).count();
        let business = FieldType::all().iter().filter(|t| t.group() == FieldGroup::Business).count();
        let advanced = FieldType::all().iter().filter(|t| t.group() == FieldGroup::Advanced).count();
        assert_eq!((general, business, advanced), (12, 11, 6));
    }

    #[test]
    fn add_and_set_cells() {
        let mut t = Table::blank("t");
        let pid = t.primary().unwrap().id.clone();
        let rid = t.records[0].id.clone();
        assert!(t.set_cell(&rid, &pid, CellValue::Text("hello".into())));
        assert_eq!(t.cell(&rid, &pid), &CellValue::Text("hello".into()));
        // emptying a cell drops it
        assert!(t.set_cell(&rid, &pid, CellValue::Text(String::new())));
        assert!(t.cell(&rid, &pid).is_empty());
    }

    #[test]
    fn primary_field_is_protected() {
        let mut t = Table::blank("t");
        let pid = t.primary().unwrap().id.clone();
        assert!(!t.remove_field(&pid), "primary must not be removable");
        let nid = t.add_field("数量", FieldType::Number);
        assert!(t.remove_field(&nid));
        assert_eq!(t.fields.len(), 1);
    }

    #[test]
    fn select_cells_render_option_names() {
        let mut t = Table::blank("t");
        let fid = t.add_field("状态", FieldType::SingleSelect);
        let (oa, ob);
        {
            let f = t.field_mut(&fid).unwrap();
            let a = SelectOption::new("进行中", 0);
            let b = SelectOption::new("已完成", 3);
            oa = a.id.clone();
            ob = b.id.clone();
            f.config.options = vec![a, b];
        }
        let rid = t.records[0].id.clone();
        t.set_cell(&rid, &fid, CellValue::Select(vec![ob.clone()]));
        let f = t.field(&fid).unwrap();
        assert_eq!(t.cell(&rid, &fid).as_plain(f), "已完成");
        assert!(f.option(&oa).is_some());
    }

    #[test]
    fn removing_a_field_drops_its_cells() {
        let mut t = Table::blank("t");
        let fid = t.add_field("数量", FieldType::Number);
        let rid = t.records[0].id.clone();
        t.set_cell(&rid, &fid, CellValue::Number(42.0));
        assert!(t.remove_field(&fid));
        assert!(!t.record(&rid).unwrap().cells.contains_key(&fid));
    }

    #[test]
    fn json_round_trips() {
        let mut t = Table::blank("库存表");
        let num = t.add_field("数量", FieldType::Number);
        t.field_mut(&num).unwrap().config.precision = Some(2);
        let sel = t.add_field("状态", FieldType::SingleSelect);
        t.field_mut(&sel).unwrap().config.options = vec![SelectOption::new("在售", 2)];
        let rid = t.records[0].id.clone();
        t.set_cell(&rid, &num, CellValue::Number(3.5));
        let json = t.to_json();
        let back = Table::from_json(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn progress_and_currency_format() {
        let mut t = Table::blank("t");
        let cur = t.add_field("金额", FieldType::Currency);
        {
            let f = t.field_mut(&cur).unwrap();
            f.config.precision = Some(2);
            f.config.currency = Some("¥".into());
        }
        let prog = t.add_field("进度", FieldType::Progress);
        let rid = t.records[0].id.clone();
        t.set_cell(&rid, &cur, CellValue::Number(1200.0));
        t.set_cell(&rid, &prog, CellValue::Number(60.0));
        assert_eq!(t.cell(&rid, &cur).as_plain(t.field(&cur).unwrap()), "¥1200.00");
        assert_eq!(t.cell(&rid, &prog).as_plain(t.field(&prog).unwrap()), "60%");
    }
}
