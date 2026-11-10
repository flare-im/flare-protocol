//! Display blocks of a UI document (v2) and their resolution.
//!
//! Every path is a JSON pointer in the `valid_path` style (`/a/b/0`, segments
//! `[A-Za-z0-9_]`, at most 128 bytes). Top-level paths read the data the
//! document is resolved against: the merged card data (payload ← shared state
//! ← local state) for message-content, the source result for a page and an
//! admin page (`admin.pages`), the detail result for a detail document. Paths
//! named `*Path` inside an option, entry, row or column read that element
//! instead. A block whose data is
//! missing is not shown: a card says what it knows.
//!
//! Every block may also carry `hiddenWhen` (`0..=4` [`UiCondition`]s, like an
//! action's): the block is not shown when any of them matches the data the
//! document is resolved against. On a card that is the merged data, so a
//! condition can read the payload, the shared state or the viewer's local
//! state; a rows block's conditions read the page (the source result), never
//! a row. Hiding a block also hides the action it owns. Like every display
//! condition this is presentation only; plugins still authorize each call.
use super::{
    MAX_UI_INPUTS, ResolvedUiFormat, UiAction, UiActionTarget, UiCondition, UiDetail, UiField,
    UiFieldFormat, UiInput, UiTranslation, bind_inputs, resolve_action, resolve_fields,
    scalar_text, translate, valid_field, valid_long_text, valid_name, valid_path, valid_text,
};
use crate::{ProtocolError, Result, tagged::kind_tagged};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Options of a choices block.
pub const MAX_UI_CHOICE_OPTIONS: usize = 50;
/// Entries shown by an entries block (`limit` default and cap).
pub const DEFAULT_UI_ENTRIES_LIMIT: u8 = 10;
pub const MAX_UI_ENTRIES_LIMIT: u8 = 50;
/// Groups of a people block, and avatars per group (`max` default and cap).
pub const MAX_UI_PEOPLE_GROUPS: usize = 4;
pub const DEFAULT_UI_PEOPLE_AVATARS: u8 = 5;
pub const MAX_UI_PEOPLE_AVATARS: u8 = 10;
/// Mapped values of a status block or a column.
pub const MAX_UI_VALUE_LABELS: usize = 32;
/// Fields of a fields block.
pub const MAX_UI_BLOCK_FIELDS: usize = 16;
/// Rows resolved per page, columns and row actions of a rows block.
pub const MAX_UI_ROWS: usize = 100;
pub const MAX_UI_COLUMNS: usize = 12;
pub const MAX_UI_ROW_ACTIONS: usize = 4;
/// Characters of a text block value read from data (newlines kept).
pub const MAX_UI_TEXT_CHARS: usize = 4000;
/// Collapsed lines of a text block.
pub const MAX_UI_TEXT_LINES: u8 = 20;
/// Conditions of a choices block's `closedWhen`, of the `hiddenWhen` of a
/// block, an action or a detail, and of one `allOf`.
pub const MAX_UI_CONDITIONS: usize = 4;
/// Default action input name of a choices selection.
pub const DEFAULT_CHOICE_INPUT: &str = "selection";
/// Input name of the rows cursor sent to the rows source operation.
pub const ROWS_CURSOR_INPUT: &str = "cursor";
/// Bytes of a rows cursor.
pub const MAX_ROWS_CURSOR_BYTES: usize = 256;
/// Items resolved by a protected media block.
pub const MAX_UI_PROTECTED_MEDIA: usize = 16;

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}
fn yes() -> bool {
    true
}
fn is_true(value: &bool) -> bool {
    *value
}

/// A display block, tagged by `kind`. Every kind may carry `hiddenWhen`
/// conditions that leave it out of the resolved view.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UiBlock {
    Text(UiTextBlock),
    Fields(UiFieldsBlock),
    Choices(UiChoicesBlock),
    Entries(UiEntriesBlock),
    People(UiPeopleBlock),
    Status(UiStatusBlock),
    Progress(UiProgressBlock),
    Time(UiTimeBlock),
    Divider(UiDividerBlock),
    Note(UiNoteBlock),
    /// Page and admin page (`admin.pages`) documents only.
    Rows(UiRowsBlock),
    /// Admin page (`admin.pages`) documents only.
    #[serde(rename = "protected-media")]
    ProtectedMedia(UiProtectedMediaBlock),
}
kind_tagged! {
    UiBlock = "UiBlock" {
        "text" => Text(UiTextBlock),
        "fields" => Fields(UiFieldsBlock),
        "choices" => Choices(UiChoicesBlock),
        "entries" => Entries(UiEntriesBlock),
        "people" => People(UiPeopleBlock),
        "status" => Status(UiStatusBlock),
        "progress" => Progress(UiProgressBlock),
        "time" => Time(UiTimeBlock),
        "divider" => Divider(UiDividerBlock),
        "note" => Note(UiNoteBlock),
        "rows" => Rows(UiRowsBlock),
        "protected-media" => ProtectedMedia(UiProtectedMediaBlock),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiTextStyle {
    #[default]
    Body,
    Title,
    Secondary,
}

/// Semantic color of a badge, note or people group; kits map it to tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiTone {
    #[default]
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

/// Text, multi-line and collapsible. Exactly one of `text` (literal, at most
/// 2000 bytes, translated through `texts`) and `path` (a scalar in the data,
/// shown up to 4000 characters).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiTextBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Lines shown collapsed (`1..=20`); the host offers "expand" beyond them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<u8>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub style: UiTextStyle,
    /// Not shown when any condition (`0..=4`) matches the resolved data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}

/// Label/value rows, with the same rules as the document's `fields`
/// (`1..=16`); value labels are translated through `texts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiFieldsBlock {
    pub fields: Vec<UiField>,
    /// Not shown when any condition (`0..=4`) matches the resolved data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}

/// When a choices block shows counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiShowResults {
    /// Always (a public poll).
    Always,
    /// Once the viewer has chosen, or once closed.
    #[default]
    Voted,
    /// Only once closed.
    Closed,
}

/// A list of options, positional: option `i` has count `counts[i]`, and the
/// selection is a list of indices. Choosing binds `action`, whose input gets
/// `{<inputName>: [indices]}` in addition to its declared input. The options
/// come from the data (`optionsPath`) or are declared (`options`), exactly
/// one of the two.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiChoicesBlock {
    /// Array of option labels (scalars), or of objects read with `labelPath`.
    /// `1..=50` options, each with a non-blank label, or the block is not shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options_path: Option<String>,
    /// Declared options (`1..=50`), such as accept / maybe / decline; labels
    /// are translated through `texts`. Counts and the selection are still read
    /// from their paths, by position.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<UiChoiceOption>,
    /// Inside an option object: its label (`optionsPath` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_path: Option<String>,
    /// Array of non-negative integers aligned with the options; missing items
    /// count 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts_path: Option<String>,
    /// The viewer's choice: an array of indices, or one index (usually local
    /// state, such as `/mySelection`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_path: Option<String>,
    /// Denominator of the percentages (such as voter count); the sum of counts
    /// when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_path: Option<String>,
    /// Truthy (true, non-zero, "true") for a multiple choice; single otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiple_path: Option<String>,
    /// A multiple choice's limit: the most options the viewer may choose, a
    /// positive integer (capped at the option count). Absent, zero or not a
    /// number: no limit beyond the options. Ignored on a single choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_selections_path: Option<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub show_results: UiShowResults,
    /// Closed when any condition matches (`0..=4`): results shown per
    /// `showResults`, no choosing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub closed_when: Vec<UiCondition>,
    /// Id of a document action with an operation target. The action is not
    /// listed with the card's buttons; the block owns it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Action input name of the selection (default `selection`); must not
    /// collide with the action's declared input or form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_name: Option<String>,
    /// Not shown when any condition (`0..=4`) matches the resolved data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}
/// A declared option of a choices block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiChoiceOption {
    pub label: String,
}

/// Entries (solitaire lines, comments, voters). The first `limit` are shown;
/// when the total is larger and the contribution offers its detail, the host
/// offers "view all", which opens that detail (`more`). Without a detail, with
/// the detail hidden by its `hiddenWhen`, and inside the detail document
/// itself, `more` is false.
///
/// Numbering: with `numbered` (the default) kits show each entry's `index`;
/// with `numbered: false` they show no position numbers at all.
///
/// Grouping (`groupPath`): entries are grouped by the scalar at `groupPath`
/// inside each entry, in data order: a new group starts wherever that key
/// differs from the previous entry's (so the plugin sends entries already
/// ordered by group, such as voters by option). The first entry of each group
/// carries the group header ([`ResolvedUiEntry::group`]): its key, its label
/// (`groupLabelPath`, else the key), and when given its count
/// (`groupCountPath`, such as the option's votes) and total
/// (`groupTotalPath`, such as all votes) with the percentage. In a grouped
/// block `index` restarts at 1 in each group, and an entry without a key at
/// `groupPath` is not shown. Kits render one flat list and put a header row
/// before every entry that has `group`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiEntriesBlock {
    /// Array of entries: objects read with the paths below, or strings. A
    /// string (or another scalar) is the entry text when `textPath` is
    /// absent, so a string array such as the latest lines kept in shared
    /// state renders as is.
    pub items_path: String,
    /// Inside an entry object: its text. When set, an entry that is not an
    /// object has nothing to show and is skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_path: Option<String>,
    /// Display name inside an entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_path: Option<String>,
    /// User id inside an entry; the host resolves avatar (and name, when
    /// `namePath` is absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_path: Option<String>,
    /// Array of user ids aligned with the entries by position (a top-level
    /// path, such as `/lineUserIds` beside `/lines`). An entry without a user
    /// of its own (a string entry, or no value at `userPath`) takes the id at
    /// its position, when that is a valid user id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_ids_path: Option<String>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub numbered: bool,
    /// `1..=50`, default 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u8>,
    /// Total number of entries when the data holds only the latest ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_path: Option<String>,
    /// Shown when the list is present but empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_text: Option<String>,
    /// Inside an entry: its group key (a scalar). Turns grouping on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_path: Option<String>,
    /// Inside an entry: its group's label (the key when absent or blank).
    /// Read from the first entry of each group; requires `groupPath`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_label_path: Option<String>,
    /// Inside an entry: its group's count, a non-negative integer. Requires
    /// `groupPath`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_count_path: Option<String>,
    /// Inside an entry: the denominator of the group percentages, a
    /// non-negative integer. Requires `groupPath`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_total_path: Option<String>,
    /// A badge for each entry: an enumerated value inside the entry mapped to
    /// a label and a tone (a ledger line's kind).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_labels: Option<UiValueLabels>,
    /// Not shown when any condition (`0..=4`) matches the resolved data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}

/// Labels for an enumerated value inside each entry or row, shown as that
/// item's badge: the scalar at `path` (inside the item) mapped to a label,
/// translated through `texts`, and a tone. An unmapped value is shown as
/// sent, neutral, as in a status block; an item without a value has no
/// badge. `1..=32` values, keys `1..=64` bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiValueLabels {
    pub path: String,
    pub values: BTreeMap<String, UiStatusValue>,
}

/// Avatar stacks with counts, optionally grouped by status (confirmed and not
/// yet confirmed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPeopleBlock {
    /// `1..=4` groups; a group whose paths are both missing is not shown.
    pub groups: Vec<UiPeopleGroup>,
    /// Avatars per group (`1..=10`, default 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u8>,
    /// Not shown when any condition (`0..=4`) matches the resolved data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPeopleGroup {
    pub label: String,
    /// Array of user ids; the host resolves avatars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub users_path: Option<String>,
    /// The group size when the data lists only some of its users; the length
    /// of the users array otherwise. At least one of the two paths is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<UiTone>,
}

/// A badge: the scalar at `path` mapped to a label and a tone; an unmapped
/// value is shown as sent, neutral.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiStatusBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub path: String,
    /// `0..=32` entries; keys `1..=64` bytes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, UiStatusValue>,
    /// Not shown when any condition (`0..=4`) matches the resolved data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiStatusValue {
    pub label: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub tone: UiTone,
}

/// Done out of total (non-negative integers), with the percentage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiProgressBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub value_path: String,
    pub total_path: String,
    /// Not shown when any condition (`0..=4`) matches the resolved data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}

/// An instant or a range in epoch milliseconds (integer or digit string). The
/// host formats it in the viewer's locale and time zone; with `countdown` it
/// counts down to the end (or to the instant) while that is in the future.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiTimeBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_path: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub countdown: bool,
    /// Not shown when any condition (`0..=4`) matches the resolved data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiDividerBlock {
    /// Not shown when any condition (`0..=4`) matches the resolved data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}

/// Secondary text (an anonymity notice). Exactly one of `text` and `path`, as
/// for a text block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiNoteBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<UiTone>,
    /// Not shown when any condition (`0..=4`) matches the resolved data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiColumnFormat {
    #[default]
    Text,
    /// Epoch milliseconds; the host formats it.
    Time,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiColumn {
    pub label: String,
    /// Inside a row.
    pub path: String,
    /// Labels for enumerated cell values (`0..=32`), translated through `texts`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub format: UiColumnFormat,
    /// How the value reads, as for a field; its paths read the row. Takes the
    /// place of `format` and `values`, which stay at their defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_format: Option<UiFieldFormat>,
}

/// How a host draws a rows block. A host that cannot draw cards draws the table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiRowsPresentation {
    #[default]
    Table,
    /// One card per row: the first column is the title, the second the
    /// description, the others its details, the row actions its control.
    Cards,
}

/// A list of rows. On an admin page the host calls `source` with the filter
/// values plus `cursor` (empty for the first page), resolves the document
/// against the result, and pages with the cursor at `cursorPath` until it is
/// empty or absent. On a page `source` is the page's source operation and the
/// rows page with the page ([`super::UiPageSource`]; no `cursorPath` of their
/// own); filter values join the source input. The filters are the inputs
/// listed in `filters`, or every document input when `filters` is absent; the
/// other inputs belong to action forms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiRowsBlock {
    /// Wire operation declared by the plugin.
    pub source: String,
    /// Array of row objects; at most 100 are resolved.
    pub path: String,
    /// Inside a row: its stable key (scalar); rows without one are skipped.
    pub key_path: String,
    /// `1..=12`.
    pub columns: Vec<UiColumn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_path: Option<String>,
    /// `0..=4` ids of document actions with operation or page targets (an
    /// admin page's: operation targets only). Their `hiddenWhen` and declared
    /// input read the row; the block owns them. A page target opens that page
    /// with the row's bound input as its arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_text: Option<String>,
    /// Ids of the document inputs that are filters (`0..=16`, unique). Absent:
    /// every input is a filter; `[]`: none is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<Vec<String>>,
    /// A badge for each row, as for entries ([`UiValueLabels`], read inside
    /// the row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_labels: Option<UiValueLabels>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub presentation: UiRowsPresentation,
    /// Not shown when any condition matches (`0..=4`). Unlike the row
    /// actions' conditions these read the page (the source result), not a row.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}

/// Private material a staff member reads one item at a time. Nothing about the
/// content is in the document: the list names the items, and reading one runs
/// `action`, an admin operation that requires a stated reason and names its
/// audit target, so each read is authorized, reasoned and audited like any
/// other admin action. Admin pages only.
///
/// The host asks for the reason, then runs the action with the item id in the
/// form input `inputName` and the reason in `reasonInput`, when the operation
/// wants it as input too. `urlPath` reads the action's result: the address the
/// host fetches for the viewer. A viewer who may not run the action does not
/// get the block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiProtectedMediaBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Array of item objects; at most 16 are resolved.
    pub items_path: String,
    /// Inside an item: its stable id (scalar); items without one are skipped.
    pub id_path: String,
    /// Inside an item: what the viewer reads it as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_path: Option<String>,
    /// Inside an item: its media type (`image/jpeg`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type_path: Option<String>,
    /// Id of a document action with an operation target; the block owns it.
    pub action: String,
    /// One of the action's `form` inputs.
    pub input_name: String,
    /// One of the action's `form` inputs. Together with `inputName` they are
    /// all of its form: the host collects nothing else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_input: Option<String>,
    pub url_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_text: Option<String>,
    /// Not shown when any condition (`0..=4`) matches the resolved data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
}

// ---- resolved blocks ----

// A resolved block is built per rendered page and serialized right away; boxing the choices
// variant (it carries its submit action) would only add an allocation.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResolvedUiBlock {
    Text(ResolvedUiText),
    Fields(ResolvedUiFields),
    Choices(ResolvedUiChoices),
    Entries(ResolvedUiEntries),
    People(ResolvedUiPeople),
    Status(ResolvedUiStatus),
    Progress(ResolvedUiProgress),
    Time(ResolvedUiTime),
    Divider(ResolvedUiDivider),
    Note(ResolvedUiNote),
    Rows(ResolvedUiRows),
    #[serde(rename = "protected-media")]
    ProtectedMedia(ResolvedUiProtectedMedia),
}
kind_tagged! {
    ResolvedUiBlock = "ResolvedUiBlock" {
        "text" => Text(ResolvedUiText),
        "fields" => Fields(ResolvedUiFields),
        "choices" => Choices(ResolvedUiChoices),
        "entries" => Entries(ResolvedUiEntries),
        "people" => People(ResolvedUiPeople),
        "status" => Status(ResolvedUiStatus),
        "progress" => Progress(ResolvedUiProgress),
        "time" => Time(ResolvedUiTime),
        "divider" => Divider(ResolvedUiDivider),
        "note" => Note(ResolvedUiNote),
        "rows" => Rows(ResolvedUiRows),
        "protected-media" => ProtectedMedia(ResolvedUiProtectedMedia),
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiText {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_lines: Option<u8>,
    pub style: UiTextStyle,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiFields {
    pub fields: Vec<super::ResolvedUiField>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiChoices {
    pub options: Vec<ResolvedUiChoice>,
    pub multiple: bool,
    /// The viewer has a selection.
    pub voted: bool,
    pub closed: bool,
    /// Counts and percentages are present.
    pub show_results: bool,
    /// The percentage denominator, when results are shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    pub input_name: String,
    /// A multiple choice's limit (`1..=` option count), when the block
    /// declares one: kits stop further ticks at it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_selections: Option<u32>,
    /// The submit action, present while the viewer can choose (not closed, not
    /// hidden).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<UiAction>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiChoice {
    pub index: u32,
    pub label: String,
    pub selected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    /// `0..=100`, rounded half up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiEntries {
    pub items: Vec<ResolvedUiEntry>,
    /// Show `index` before each entry; false: no numbers at all.
    pub numbered: bool,
    pub total: u64,
    /// More entries exist than shown and the contribution offers its detail
    /// (declared, not hidden by its `hiddenWhen`): offer "view all".
    pub more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_text: Option<String>,
    /// The block groups its entries: the first entry of each group carries
    /// [`ResolvedUiEntry::group`], and `index` counts within the group.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub grouped: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiEntry {
    /// 1-based position in the data; in a grouped block, within its group.
    pub index: u32,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Grouped block only: present on the first entry of each group, the
    /// header kits render before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<ResolvedUiEntryGroup>,
    /// The entry's badge, from the block's `valueLabels`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<ResolvedUiBadge>,
}
/// An item's enumerated value as words and a tone ([`UiValueLabels`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiBadge {
    pub text: String,
    pub tone: UiTone,
}
/// The header of a group of entries ("Option A · 5 votes · 42%").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiEntryGroup {
    /// The group key as text; equal keys are the same group across pages.
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// `0..=100`, rounded half up, when both `count` and `total` are known (0
    /// when `total` is 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
}
impl ResolvedUiEntries {
    /// Append a page's entries to a grouped block (see
    /// [`super::ResolvedPluginView::append_page`]).
    pub(crate) fn append_grouped(&mut self, page: Vec<ResolvedUiEntry>) {
        let last_header = self.items.iter().rposition(|item| item.group.is_some());
        let last_key =
            last_header.and_then(|i| self.items[i].group.as_ref().map(|g| g.key.clone()));
        let offset = self.items.last().map_or(0, |item| item.index);
        let mut continuing = true;
        for mut item in page {
            if let Some(group) = item.group.take() {
                if continuing && last_key.as_deref() == Some(group.key.as_str()) {
                    if let Some(header) = last_header.and_then(|i| self.items[i].group.as_mut()) {
                        header.count = group.count.or(header.count);
                        header.total = group.total.or(header.total);
                        header.percent = group.percent.or(header.percent);
                    }
                } else {
                    continuing = false;
                    item.group = Some(group);
                }
            }
            if continuing {
                item.index = item.index.saturating_add(offset);
            }
            self.items.push(item);
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiPeople {
    pub groups: Vec<ResolvedUiPeopleGroup>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiPeopleGroup {
    pub label: String,
    /// At most the block's `max` ids, in data order.
    pub user_ids: Vec<String>,
    pub count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<UiTone>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub text: String,
    pub tone: UiTone,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub value: u64,
    pub total: u64,
    /// `0..=100`, rounded half up; 0 when the total is 0.
    pub percent: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiTime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<i64>,
    pub countdown: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiDivider {}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiNote {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<UiTone>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiRows {
    /// The operation that returns pages (see [`UiRowsBlock::source_input`]).
    pub source: String,
    pub columns: Vec<ResolvedUiColumn>,
    pub rows: Vec<ResolvedUiRow>,
    /// Cursor of the next page; absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// The row actions, resolved once, with their targets (an operation to
    /// invoke or a page to open); each row lists the ones it offers.
    pub actions: Vec<UiAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_text: Option<String>,
    /// The ids of the view's `inputs` shown as filters, as declared; absent:
    /// every input is a filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub presentation: UiRowsPresentation,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiProtectedMedia {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub items: Vec<ResolvedUiProtectedItem>,
    /// The read, absent on a read-only copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<UiAction>,
    pub input_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_input: Option<String>,
    pub url_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty_text: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiProtectedItem {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// The action's declared input, bound from the resolved data.
    pub input: Value,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiColumn {
    pub label: String,
    pub format: UiColumnFormat,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiRow {
    pub key: String,
    /// One per column.
    pub cells: Vec<ResolvedUiCell>,
    pub actions: Vec<ResolvedUiRowAction>,
    /// Input of the contribution's detail operation for this row, when the
    /// contribution declares a detail and the row binds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_input: Option<Value>,
    /// The row's badge, from the block's `valueLabels`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<ResolvedUiBadge>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiCell {
    /// Empty when the row has no value.
    pub text: String,
    /// time columns: the instant to format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<i64>,
    /// What the column's `valueFormat` adds to `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ResolvedUiFormat>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiRowAction {
    pub id: String,
    /// The action's declared input bound from the row: the operation input,
    /// or for a page target the arguments of the page it opens.
    pub input: Value,
}

// ---- validation ----

fn optional_path(path: &Option<String>) -> bool {
    path.as_deref().is_none_or(valid_path)
}
fn optional_text(text: &Option<String>) -> bool {
    text.as_deref().is_none_or(valid_text)
}
fn valid_value_labels(values: &BTreeMap<String, String>) -> bool {
    values.len() <= MAX_UI_VALUE_LABELS
        && values
            .iter()
            .all(|(value, label)| !value.is_empty() && value.len() <= 64 && valid_text(label))
}
fn valid_item_labels(labels: &Option<UiValueLabels>) -> bool {
    labels.as_ref().is_none_or(|labels| {
        valid_path(&labels.path)
            && !labels.values.is_empty()
            && labels.values.len() <= MAX_UI_VALUE_LABELS
            && labels.values.iter().all(|(value, mapped)| {
                !value.is_empty() && value.len() <= 64 && valid_text(&mapped.label)
            })
    })
}
fn operation_action<'a>(actions: &'a [UiAction], id: &str) -> Option<&'a UiAction> {
    actions
        .iter()
        .find(|a| a.id == id && matches!(a.target, UiActionTarget::Operation { .. }))
}
/// An action a rows block may own: an operation, or a page the row opens with
/// the row's bound input as its arguments. Never a native capability. Admin
/// pages admit operation targets only (`admin.pages`).
fn row_action<'a>(actions: &'a [UiAction], id: &str) -> Option<&'a UiAction> {
    actions.iter().find(|a| {
        a.id == id
            && matches!(
                a.target,
                UiActionTarget::Operation { .. } | UiActionTarget::Page { .. }
            )
    })
}
fn text_or_path(text: &Option<String>, path: &Option<String>) -> bool {
    match (text, path) {
        (Some(text), None) => valid_long_text(text),
        (None, Some(path)) => valid_path(path),
        _ => false,
    }
}

/// A condition list of at most [`MAX_UI_CONDITIONS`] valid conditions.
pub(crate) fn valid_conditions(conditions: &[UiCondition]) -> bool {
    conditions.len() <= MAX_UI_CONDITIONS && conditions.iter().all(UiCondition::is_valid)
}

impl UiBlock {
    /// The block's display conditions: it is not shown when any of them
    /// matches the data the document is resolved against.
    pub fn hidden_when(&self) -> &[UiCondition] {
        match self {
            Self::Text(b) => &b.hidden_when,
            Self::Fields(b) => &b.hidden_when,
            Self::Choices(b) => &b.hidden_when,
            Self::Entries(b) => &b.hidden_when,
            Self::People(b) => &b.hidden_when,
            Self::Status(b) => &b.hidden_when,
            Self::Progress(b) => &b.hidden_when,
            Self::Time(b) => &b.hidden_when,
            Self::Divider(b) => &b.hidden_when,
            Self::Note(b) => &b.hidden_when,
            Self::Rows(b) => &b.hidden_when,
            Self::ProtectedMedia(b) => &b.hidden_when,
        }
    }

    /// Action ids this block owns (they are not listed as card buttons).
    pub(crate) fn owned_actions(&self) -> Vec<&str> {
        match self {
            Self::Choices(block) => block.action.as_deref().into_iter().collect(),
            Self::Rows(block) => block.actions.iter().map(String::as_str).collect(),
            Self::ProtectedMedia(block) => vec![block.action.as_str()],
            _ => Vec::new(),
        }
    }

    pub(crate) fn validate(&self, actions: &[UiAction], inputs: &[UiInput]) -> Result<()> {
        let ok = match self {
            Self::Text(b) => {
                text_or_path(&b.text, &b.path)
                    && b.max_lines
                        .is_none_or(|n| (1..=MAX_UI_TEXT_LINES).contains(&n))
            }
            Self::Fields(b) => {
                !b.fields.is_empty()
                    && b.fields.len() <= MAX_UI_BLOCK_FIELDS
                    && b.fields.iter().all(valid_field)
            }
            Self::Choices(b) => {
                let input_name = b.input_name.as_deref().unwrap_or(DEFAULT_CHOICE_INPUT);
                let options = match (&b.options_path, b.options.is_empty()) {
                    (Some(path), true) => valid_path(path),
                    (None, false) => {
                        b.options.len() <= MAX_UI_CHOICE_OPTIONS
                            && b.options.iter().all(|o| valid_text(&o.label))
                            && b.label_path.is_none()
                    }
                    _ => false,
                };
                options
                    && [
                        &b.label_path,
                        &b.counts_path,
                        &b.selected_path,
                        &b.total_path,
                        &b.multiple_path,
                        &b.max_selections_path,
                    ]
                    .into_iter()
                    .all(optional_path)
                    && b.closed_when.len() <= MAX_UI_CONDITIONS
                    && b.closed_when.iter().all(UiCondition::is_valid)
                    && valid_name(input_name)
                    && b.action.as_deref().is_none_or(|id| {
                        operation_action(actions, id).is_some_and(|action| {
                            !action.input.contains_key(input_name)
                                && !action.form.iter().any(|f| f == input_name)
                        })
                    })
            }
            Self::Entries(b) => {
                valid_path(&b.items_path)
                    && [
                        &b.text_path,
                        &b.name_path,
                        &b.user_path,
                        &b.user_ids_path,
                        &b.count_path,
                        &b.group_path,
                        &b.group_label_path,
                        &b.group_count_path,
                        &b.group_total_path,
                    ]
                    .into_iter()
                    .all(optional_path)
                    && (b.group_path.is_some()
                        || (b.group_label_path.is_none()
                            && b.group_count_path.is_none()
                            && b.group_total_path.is_none()))
                    && b.limit
                        .is_none_or(|n| (1..=MAX_UI_ENTRIES_LIMIT).contains(&n))
                    && optional_text(&b.empty_text)
                    && valid_item_labels(&b.value_labels)
            }
            Self::People(b) => {
                !b.groups.is_empty()
                    && b.groups.len() <= MAX_UI_PEOPLE_GROUPS
                    && b.groups.iter().all(|g| {
                        valid_text(&g.label)
                            && (g.users_path.is_some() || g.count_path.is_some())
                            && optional_path(&g.users_path)
                            && optional_path(&g.count_path)
                    })
                    && b.max
                        .is_none_or(|n| (1..=MAX_UI_PEOPLE_AVATARS).contains(&n))
            }
            Self::Status(b) => {
                optional_text(&b.label)
                    && valid_path(&b.path)
                    && b.values.len() <= MAX_UI_VALUE_LABELS
                    && b.values.iter().all(|(value, mapped)| {
                        !value.is_empty() && value.len() <= 64 && valid_text(&mapped.label)
                    })
            }
            Self::Progress(b) => {
                optional_text(&b.label) && valid_path(&b.value_path) && valid_path(&b.total_path)
            }
            Self::Time(b) => {
                optional_text(&b.label) && valid_path(&b.path) && optional_path(&b.end_path)
            }
            Self::Divider(_) => true,
            Self::Note(b) => text_or_path(&b.text, &b.path),
            Self::Rows(b) => {
                let mut ids = BTreeSet::new();
                !b.source.is_empty()
                    && b.source.len() <= 128
                    && valid_path(&b.path)
                    && valid_path(&b.key_path)
                    && !b.columns.is_empty()
                    && b.columns.len() <= MAX_UI_COLUMNS
                    && b.columns.iter().all(|c| {
                        valid_text(&c.label)
                            && valid_path(&c.path)
                            && valid_value_labels(&c.values)
                            && c.value_format.as_ref().is_none_or(|format| {
                                format.is_valid()
                                    && c.format == UiColumnFormat::Text
                                    && c.values.is_empty()
                            })
                    })
                    && optional_path(&b.cursor_path)
                    && b.actions.len() <= MAX_UI_ROW_ACTIONS
                    && b.actions
                        .iter()
                        .all(|id| ids.insert(id) && row_action(actions, id).is_some())
                    && optional_text(&b.empty_text)
                    && b.filters.as_ref().is_none_or(|filters| {
                        let mut seen = BTreeSet::new();
                        filters.len() <= MAX_UI_INPUTS
                            && filters.iter().all(|id| {
                                seen.insert(id) && inputs.iter().any(|input| &input.id == id)
                            })
                    })
                    && valid_item_labels(&b.value_labels)
            }
            Self::ProtectedMedia(b) => {
                optional_text(&b.label)
                    && valid_path(&b.items_path)
                    && valid_path(&b.id_path)
                    && optional_path(&b.label_path)
                    && optional_path(&b.media_type_path)
                    && valid_path(&b.url_path)
                    && optional_text(&b.empty_text)
                    && b.reason_input.as_ref() != Some(&b.input_name)
                    && operation_action(actions, &b.action).is_some_and(|action| {
                        action.form.contains(&b.input_name)
                            && action.form.iter().all(|id| {
                                id == &b.input_name || b.reason_input.as_ref() == Some(id)
                            })
                            && b.reason_input
                                .as_ref()
                                .is_none_or(|id| action.form.contains(id))
                    })
            }
        };
        if ok && valid_conditions(self.hidden_when()) {
            Ok(())
        } else {
            Err(ProtocolError::Denied("invalid UI block"))
        }
    }
}

// ---- resolution ----

pub(crate) struct Scope<'a> {
    pub data: &'a Value,
    pub viewer: &'a str,
    pub translation: Option<&'a UiTranslation>,
    pub actions: &'a [UiAction],
    /// The contribution's detail while it is offered: declared and not
    /// hidden by its `hiddenWhen`. None inside a detail document.
    pub detail: Option<&'a UiDetail>,
}

fn at<'v>(data: &'v Value, path: &Option<String>) -> Option<&'v Value> {
    data.pointer(path.as_deref()?).filter(|v| !v.is_null())
}
fn as_count(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) if !s.is_empty() && s.len() <= 20 => s.parse().ok(),
        _ => None,
    }
}
fn as_instant(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) if !s.is_empty() && s.len() <= 20 => s.parse().ok(),
        _ => None,
    }
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.0),
        Value::String(s) => s == "true" || s == "1",
        _ => false,
    }
}
fn percent(value: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    let (value, total) = (u128::from(value.min(total)), u128::from(total));
    ((value * 100 + total / 2) / total) as u8
}
fn long_text(value: &Value) -> Option<String> {
    let text: String = match value {
        Value::String(s) => s
            .chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(MAX_UI_TEXT_CHARS)
            .collect(),
        other => scalar_text(other)?,
    };
    (!text.trim().is_empty()).then_some(text)
}
fn non_blank(value: Option<&Value>) -> Option<String> {
    value
        .and_then(scalar_text)
        .filter(|text| !text.trim().is_empty())
}
fn user_id(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|id| crate::resource::valid_resource_id(id))
        .map(str::to_owned)
}
fn literal_or_path(
    scope: &Scope<'_>,
    text: &Option<String>,
    path: &Option<String>,
) -> Option<String> {
    match text {
        Some(text) => Some(translate(scope.translation, text)),
        None => long_text(at(scope.data, path)?),
    }
}
fn label(scope: &Scope<'_>, text: &Option<String>) -> Option<String> {
    text.as_deref().map(|t| translate(scope.translation, t))
}
/// The badge of an entry or row `item` ([`UiValueLabels`]).
fn badge(
    labels: &Option<UiValueLabels>,
    item: &Value,
    translation: Option<&UiTranslation>,
) -> Option<ResolvedUiBadge> {
    let labels = labels.as_ref()?;
    let raw = non_blank(item.pointer(&labels.path))?;
    Some(match labels.values.get(&raw) {
        Some(mapped) => ResolvedUiBadge {
            text: translate(translation, &mapped.label),
            tone: mapped.tone,
        },
        None => ResolvedUiBadge {
            text: raw,
            tone: UiTone::Neutral,
        },
    })
}
fn visible(action: &UiAction, data: &Value, viewer: &str) -> bool {
    !action.hidden_when.iter().any(|c| c.matches(data, viewer))
}

impl UiBlock {
    pub(crate) fn resolve(&self, scope: &Scope<'_>) -> Option<ResolvedUiBlock> {
        if self
            .hidden_when()
            .iter()
            .any(|c| c.matches(scope.data, scope.viewer))
        {
            return None;
        }
        Some(match self {
            Self::Text(b) => ResolvedUiBlock::Text(ResolvedUiText {
                text: literal_or_path(scope, &b.text, &b.path)?,
                max_lines: b.max_lines,
                style: b.style,
            }),
            Self::Fields(b) => {
                let fields = resolve_fields(
                    &b.fields,
                    scope.data,
                    scope.translation,
                    |field, _| translate(scope.translation, &field.label),
                    |field, raw| {
                        field
                            .values
                            .get(raw)
                            .map(|mapped| translate(scope.translation, mapped))
                    },
                );
                if fields.is_empty() {
                    return None;
                }
                ResolvedUiBlock::Fields(ResolvedUiFields { fields })
            }
            Self::Choices(b) => ResolvedUiBlock::Choices(b.resolve(scope)?),
            Self::Entries(b) => ResolvedUiBlock::Entries(b.resolve(scope)?),
            Self::People(b) => {
                let max = usize::from(b.max.unwrap_or(DEFAULT_UI_PEOPLE_AVATARS));
                let groups: Vec<_> = b
                    .groups
                    .iter()
                    .filter_map(|g| {
                        let users = at(scope.data, &g.users_path).and_then(Value::as_array);
                        let count = at(scope.data, &g.count_path).and_then(as_count);
                        if users.is_none() && count.is_none() {
                            return None;
                        }
                        let ids: Vec<String> = users
                            .into_iter()
                            .flatten()
                            .filter_map(|v| user_id(Some(v)))
                            .collect();
                        Some(ResolvedUiPeopleGroup {
                            label: translate(scope.translation, &g.label),
                            count: count.unwrap_or(ids.len() as u64),
                            user_ids: ids.into_iter().take(max).collect(),
                            tone: g.tone,
                        })
                    })
                    .collect();
                if groups.is_empty() {
                    return None;
                }
                ResolvedUiBlock::People(ResolvedUiPeople { groups })
            }
            Self::Status(b) => {
                let raw = non_blank(scope.data.pointer(&b.path))?;
                let (text, tone) = match b.values.get(&raw) {
                    Some(mapped) => (translate(scope.translation, &mapped.label), mapped.tone),
                    None => (raw, UiTone::Neutral),
                };
                ResolvedUiBlock::Status(ResolvedUiStatus {
                    label: label(scope, &b.label),
                    text,
                    tone,
                })
            }
            Self::Progress(b) => {
                let value = scope.data.pointer(&b.value_path).and_then(as_count)?;
                let total = scope.data.pointer(&b.total_path).and_then(as_count)?;
                ResolvedUiBlock::Progress(ResolvedUiProgress {
                    label: label(scope, &b.label),
                    value,
                    total,
                    percent: percent(value, total),
                })
            }
            Self::Time(b) => ResolvedUiBlock::Time(ResolvedUiTime {
                label: label(scope, &b.label),
                at_ms: scope.data.pointer(&b.path).and_then(as_instant)?,
                end_ms: at(scope.data, &b.end_path).and_then(as_instant),
                countdown: b.countdown,
            }),
            Self::Divider(_) => ResolvedUiBlock::Divider(ResolvedUiDivider {}),
            Self::Note(b) => ResolvedUiBlock::Note(ResolvedUiNote {
                text: literal_or_path(scope, &b.text, &b.path)?,
                tone: b.tone,
            }),
            Self::Rows(b) => ResolvedUiBlock::Rows(b.resolve(scope)?),
            Self::ProtectedMedia(b) => ResolvedUiBlock::ProtectedMedia(b.resolve(scope)?),
        })
    }
}

impl UiChoicesBlock {
    pub fn input_name(&self) -> &str {
        self.input_name.as_deref().unwrap_or(DEFAULT_CHOICE_INPUT)
    }

    /// The option labels: the declared ones in the view language, or those
    /// read from `optionsPath`.
    fn labels(&self, data: &Value, translation: Option<&UiTranslation>) -> Option<Vec<String>> {
        if !self.options.is_empty() {
            return Some(
                self.options
                    .iter()
                    .map(|option| translate(translation, &option.label))
                    .collect(),
            );
        }
        let items = data.pointer(self.options_path.as_deref()?)?.as_array()?;
        if items.is_empty() || items.len() > MAX_UI_CHOICE_OPTIONS {
            return None;
        }
        items
            .iter()
            .map(|item| match &self.label_path {
                Some(path) => non_blank(item.pointer(path)),
                None => non_blank(Some(item)),
            })
            .collect()
    }

    fn multiple(&self, data: &Value) -> bool {
        at(data, &self.multiple_path).is_some_and(truthy)
    }

    /// The most options a multiple choice over `options` options takes, when
    /// the block declares a positive limit.
    fn max_selections(&self, data: &Value, options: usize) -> Option<u32> {
        if !self.multiple(data) {
            return None;
        }
        let max = at(data, &self.max_selections_path).and_then(as_count)?;
        (max > 0).then(|| max.min(options as u64) as u32)
    }

    fn resolve(&self, scope: &Scope<'_>) -> Option<ResolvedUiChoices> {
        let labels = self.labels(scope.data, scope.translation)?;
        let counts: Vec<u64> = at(scope.data, &self.counts_path)
            .and_then(Value::as_array)
            .map(|items| items.iter().map(|v| as_count(v).unwrap_or(0)).collect())
            .unwrap_or_default();
        let selected: BTreeSet<u64> = match at(scope.data, &self.selected_path) {
            Some(Value::Array(items)) => items.iter().filter_map(as_count).collect(),
            Some(single) => as_count(single).into_iter().collect(),
            None => BTreeSet::new(),
        };
        let count_of = |index: usize| counts.get(index).copied().unwrap_or(0);
        let voted = (0..labels.len()).any(|i| selected.contains(&(i as u64)));
        let closed = self
            .closed_when
            .iter()
            .any(|c| c.matches(scope.data, scope.viewer));
        let show_results = match self.show_results {
            UiShowResults::Always => true,
            UiShowResults::Voted => voted || closed,
            UiShowResults::Closed => closed,
        };
        let total = at(scope.data, &self.total_path)
            .and_then(as_count)
            .unwrap_or_else(|| (0..labels.len()).map(count_of).fold(0, u64::saturating_add));
        let max_selections = self.max_selections(scope.data, labels.len());
        let options = labels
            .into_iter()
            .enumerate()
            .map(|(index, label)| ResolvedUiChoice {
                index: index as u32,
                label,
                selected: selected.contains(&(index as u64)),
                count: show_results.then(|| count_of(index)),
                percent: show_results.then(|| percent(count_of(index), total)),
            })
            .collect();
        let action = self
            .action
            .as_deref()
            .filter(|_| !closed)
            .and_then(|id| operation_action(scope.actions, id))
            .filter(|action| visible(action, scope.data, scope.viewer))
            .map(|action| resolve_action(action, scope.translation));
        Some(ResolvedUiChoices {
            options,
            multiple: self.multiple(scope.data),
            voted,
            closed,
            show_results,
            total: show_results.then_some(total),
            input_name: self.input_name().to_owned(),
            max_selections,
            action,
        })
    }

    /// The action input entry for a selection on a card resolved from `data`:
    /// `(inputName, [indices])`, after checking it is non-empty, unique, in
    /// range, a single index unless the choice is multiple, and within a
    /// multiple choice's `maxSelectionsPath` limit.
    pub fn selection_input(&self, data: &Value, selection: &[u32]) -> Result<(String, Value)> {
        let options = self
            .labels(data, None)
            .ok_or(ProtocolError::Denied("UI choices unavailable"))?
            .len();
        let unique: BTreeSet<_> = selection.iter().collect();
        if selection.is_empty()
            || unique.len() != selection.len()
            || selection.iter().any(|i| *i as usize >= options)
            || (selection.len() > 1 && !self.multiple(data))
            || self
                .max_selections(data, options)
                .is_some_and(|max| selection.len() > max as usize)
        {
            return Err(ProtocolError::Denied("invalid UI choice selection"));
        }
        Ok((
            self.input_name().to_owned(),
            Value::Array(selection.iter().map(|i| Value::from(*i)).collect()),
        ))
    }
}

impl UiEntriesBlock {
    fn resolve(&self, scope: &Scope<'_>) -> Option<ResolvedUiEntries> {
        let items = scope.data.pointer(&self.items_path)?.as_array()?;
        let aligned_users = at(scope.data, &self.user_ids_path).and_then(Value::as_array);
        let limit = usize::from(self.limit.unwrap_or(DEFAULT_UI_ENTRIES_LIMIT));
        let mut current_group: Option<String> = None;
        let mut in_group = 0u32;
        let shown: Vec<ResolvedUiEntry> = items
            .iter()
            .take(limit)
            .enumerate()
            .filter_map(|(position, item)| {
                let text = match &self.text_path {
                    Some(path) => non_blank(item.pointer(path)),
                    // A string (or other scalar) entry is its own text.
                    None => non_blank(Some(item)),
                };
                let name = self
                    .name_path
                    .as_deref()
                    .and_then(|p| non_blank(item.pointer(p)));
                let user = self
                    .user_path
                    .as_deref()
                    .and_then(|p| user_id(item.pointer(p)))
                    .or_else(|| user_id(aligned_users.and_then(|ids| ids.get(position))));
                if text.is_none() && name.is_none() && user.is_none() {
                    return None;
                }
                let (index, group) = match &self.group_path {
                    None => (position as u32 + 1, None),
                    Some(path) => {
                        let key = non_blank(item.pointer(path))?;
                        let header = (current_group.as_deref() != Some(key.as_str()))
                            .then(|| self.group_header(item, &key));
                        if header.is_some() {
                            current_group = Some(key);
                            in_group = 0;
                        }
                        in_group += 1;
                        (in_group, header)
                    }
                };
                Some(ResolvedUiEntry {
                    index,
                    text: text.unwrap_or_default(),
                    name,
                    user_id: user,
                    group,
                    badge: badge(&self.value_labels, item, scope.translation),
                })
            })
            .collect();
        let total = at(scope.data, &self.count_path)
            .and_then(as_count)
            .unwrap_or(items.len() as u64);
        Some(ResolvedUiEntries {
            more: scope.detail.is_some() && total > shown.len() as u64,
            items: shown,
            numbered: self.numbered,
            total,
            empty_text: label(scope, &self.empty_text),
            grouped: self.group_path.is_some(),
        })
    }

    /// The header of the group `key` starts at `item`. The label is data (an
    /// option label), shown as sent like every other value read from data.
    fn group_header(&self, item: &Value, key: &str) -> ResolvedUiEntryGroup {
        let read = |path: &Option<String>| path.as_deref().and_then(|p| item.pointer(p));
        let count = read(&self.group_count_path).and_then(as_count);
        let total = read(&self.group_total_path).and_then(as_count);
        ResolvedUiEntryGroup {
            key: key.to_owned(),
            label: non_blank(read(&self.group_label_path)).unwrap_or_else(|| key.to_owned()),
            count,
            total,
            percent: count.zip(total).map(|(count, total)| percent(count, total)),
        }
    }
}

impl UiRowsBlock {
    /// The source operation input for a page: the validated filter values plus
    /// `cursor` (empty for the first page).
    pub fn source_input(&self, cursor: Option<&str>, filters: Map<String, Value>) -> Result<Value> {
        let cursor = cursor.unwrap_or_default();
        if cursor.len() > MAX_ROWS_CURSOR_BYTES
            || cursor.chars().any(char::is_control)
            || filters.contains_key(ROWS_CURSOR_INPUT)
        {
            return Err(ProtocolError::Denied("invalid UI rows request"));
        }
        let mut input = filters;
        input.insert(ROWS_CURSOR_INPUT.into(), Value::from(cursor));
        Ok(Value::Object(input))
    }

    fn resolve(&self, scope: &Scope<'_>) -> Option<ResolvedUiRows> {
        let raw_rows = scope.data.pointer(&self.path)?.as_array()?;
        let row_actions: Vec<&UiAction> = self
            .actions
            .iter()
            .filter_map(|id| row_action(scope.actions, id))
            .collect();
        let rows = raw_rows
            .iter()
            .take(MAX_UI_ROWS)
            .filter_map(|row| {
                let key = non_blank(row.pointer(&self.key_path))?;
                let cells = self
                    .columns
                    .iter()
                    .map(|column| {
                        let value = row.pointer(&column.path).filter(|v| !v.is_null());
                        let raw = value.and_then(scalar_text).unwrap_or_default();
                        if let Some(format) =
                            column.value_format.as_ref().filter(|_| !raw.is_empty())
                        {
                            let (text, format) = format.resolve(row, &raw, scope.translation);
                            return ResolvedUiCell {
                                text: text.unwrap_or(raw),
                                time_ms: None,
                                format,
                            };
                        }
                        match column.format {
                            UiColumnFormat::Time => ResolvedUiCell {
                                time_ms: value.and_then(as_instant),
                                text: raw,
                                format: None,
                            },
                            UiColumnFormat::Text => ResolvedUiCell {
                                text: column
                                    .values
                                    .get(&raw)
                                    .map_or(raw, |mapped| translate(scope.translation, mapped)),
                                time_ms: None,
                                format: None,
                            },
                        }
                    })
                    .collect();
                let actions = row_actions
                    .iter()
                    .filter(|action| visible(action, row, scope.viewer))
                    .filter_map(|action| {
                        Some(ResolvedUiRowAction {
                            id: action.id.clone(),
                            input: bind_inputs(&action.input, row).ok()?,
                        })
                    })
                    .collect();
                Some(ResolvedUiRow {
                    key,
                    cells,
                    actions,
                    detail_input: scope.detail.and_then(|d| d.bind_input(row).ok()),
                    badge: badge(&self.value_labels, row, scope.translation),
                })
            })
            .collect();
        Some(ResolvedUiRows {
            source: self.source.clone(),
            columns: self
                .columns
                .iter()
                .map(|c| ResolvedUiColumn {
                    label: translate(scope.translation, &c.label),
                    format: c.format,
                })
                .collect(),
            rows,
            next_cursor: non_blank(at(scope.data, &self.cursor_path)),
            actions: row_actions
                .into_iter()
                .map(|action| resolve_action(action, scope.translation))
                .collect(),
            empty_text: label(scope, &self.empty_text),
            filters: self.filters.clone(),
            presentation: self.presentation,
        })
    }
}

impl UiProtectedMediaBlock {
    fn resolve(&self, scope: &Scope<'_>) -> Option<ResolvedUiProtectedMedia> {
        let raw = scope.data.pointer(&self.items_path)?.as_array()?;
        let action = operation_action(scope.actions, &self.action)
            .filter(|action| visible(action, scope.data, scope.viewer))?;
        // The declared input reads the record, the same for every item.
        let input = bind_inputs(&action.input, scope.data).ok()?;
        let items = raw
            .iter()
            .take(MAX_UI_PROTECTED_MEDIA)
            .filter_map(|item| {
                let id = non_blank(item.pointer(&self.id_path))?;
                Some(ResolvedUiProtectedItem {
                    label: non_blank(at(item, &self.label_path)).unwrap_or_else(|| id.clone()),
                    media_type: non_blank(at(item, &self.media_type_path)),
                    input: input.clone(),
                    id,
                })
            })
            .collect();
        Some(ResolvedUiProtectedMedia {
            label: label(scope, &self.label),
            items,
            action: Some(resolve_action(action, scope.translation)),
            input_name: self.input_name.clone(),
            reason_input: self.reason_input.clone(),
            url_path: self.url_path.clone(),
            empty_text: label(scope, &self.empty_text),
        })
    }
}
