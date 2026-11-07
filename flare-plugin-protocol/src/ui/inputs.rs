//! Form inputs of a UI document (v2). A composer-action or card action names
//! the inputs it collects in [`super::UiAction::form`]; the inputs of an admin
//! page (`admin.pages`) are the filters sent to its rows source. The host renders the form,
//! the core SDK validates it with [`validate_form_values`], and the plugin
//! re-validates the same rules on its side.
use super::{UiTranslation, translate, valid_name, valid_path, valid_text};
use crate::{ProtocolError, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use std::collections::BTreeSet;

/// `text`: default and largest `maxLength` (characters).
pub const TEXT_DEFAULT_MAX_LENGTH: u32 = 256;
pub const TEXT_MAX_LENGTH: u32 = 1024;
/// `textarea`: default and largest `maxLength` (characters).
pub const TEXTAREA_DEFAULT_MAX_LENGTH: u32 = 2000;
pub const TEXTAREA_MAX_LENGTH: u32 = 8000;
/// `options`: default and largest `maxLength` of one item (characters).
pub const OPTION_DEFAULT_MAX_LENGTH: u32 = 100;
pub const OPTION_MAX_LENGTH: u32 = 256;
/// `options`: most items (and the default `max`).
pub const OPTIONS_MAX_ITEMS: u32 = 50;
/// `members`: most user ids (and the default `max`).
pub const MEMBERS_MAX_ITEMS: u32 = 500;
/// `select`: most choices.
pub const SELECT_MAX_CHOICES: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiInputKind {
    /// One line of text; value is a string.
    Text,
    /// Secret text: no default, data binding or durable page command.
    Password,
    /// Multi-line text; value is a string (newlines allowed).
    Textarea,
    /// Value is a JSON number within `min..=max`.
    Number,
    /// Value is a boolean; never `required`.
    Toggle,
    /// Value is the `value` of one of `choices`.
    Select,
    /// Value is an instant in epoch milliseconds (integer); the host picks it
    /// in the viewer's time zone.
    Datetime,
    /// An editable list of option texts; value is an array of strings, item
    /// count within `min..=max`, each item non-empty and unique.
    Options,
    /// Conversation members picked from the list the host provides; value is
    /// an array of user ids, count within `min..=max`. `max: 1` is a single pick.
    Members,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiInputChoice {
    /// `1..=128` bytes, unique within the input.
    pub value: String,
    pub label: String,
}

/// A form input. Fields that do not apply to `kind` are rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiInput {
    /// The key of the value in the action input: `[A-Za-z0-9_]{1,64}`, unique
    /// in the document.
    pub id: String,
    pub kind: UiInputKind,
    pub label: String,
    /// A value is required: non-blank text, at least one item.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Characters: text (`1..=1024`, default 256), textarea (`1..=8000`,
    /// default 2000), options per item (`1..=256`, default 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// number: smallest value. options, members: fewest items (integer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Number>,
    /// number: largest value. options (`<= 50`), members (`<= 500`): most items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Number>,
    /// options and members only: the most items read from the data the
    /// document is resolved against instead (a composer form's
    /// `/config/<key>`). When it holds a positive integer the resolved input's
    /// `max` is that integer, capped at 50 or 500; it keeps the declared `max`
    /// when the value is absent, not such an integer, or would make the input
    /// invalid (below `min`, or below the item count of `default`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_path: Option<String>,
    /// Initial value of the input's type; options may hold empty placeholder
    /// items. Not allowed on members.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Initial value read from the card data instead (for editing an existing
    /// entry); falls back to `default` when absent or of the wrong type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_path: Option<String>,
    /// select only: `1..=1024` choices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<UiInputChoice>,
    /// Read bounded select choices from the authenticated source projection.
    /// Removed before rendering; every host consumes the same resolved list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices_path: Option<String>,
}

/// One problem with a submitted form value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiFormIssue {
    Required,
    /// Not an input of this form.
    Unknown,
    InvalidType,
    TooLong,
    OutOfRange,
    NotAChoice,
    TooFewItems,
    TooManyItems,
    /// An empty, too long or malformed item.
    InvalidItem,
    DuplicateItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiFormError {
    pub input: String,
    pub issue: UiFormIssue,
}

fn count(n: &Number) -> Option<u32> {
    n.as_u64().and_then(|v| u32::try_from(v).ok())
}

fn single_line(s: &str) -> bool {
    !s.chars().any(char::is_control)
}

fn multi_line(s: &str) -> bool {
    !s.chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

impl UiInput {
    pub fn effective_max_length(&self) -> u32 {
        self.max_length.unwrap_or(match self.kind {
            UiInputKind::Textarea => TEXTAREA_DEFAULT_MAX_LENGTH,
            UiInputKind::Options => OPTION_DEFAULT_MAX_LENGTH,
            _ => TEXT_DEFAULT_MAX_LENGTH,
        })
    }

    /// `(fewest, most)` items of options and members.
    fn item_bounds(&self) -> (u32, u32) {
        let cap = if self.kind == UiInputKind::Members {
            MEMBERS_MAX_ITEMS
        } else {
            OPTIONS_MAX_ITEMS
        };
        (
            self.min.as_ref().and_then(count).unwrap_or(0),
            self.max.as_ref().and_then(count).unwrap_or(cap),
        )
    }

    pub(crate) fn validate(&self) -> Result<()> {
        use UiInputKind::*;
        let invalid = Err(ProtocolError::Denied("invalid UI input"));
        let text_like = matches!(self.kind, Text | Password | Textarea | Options);
        let ranged = matches!(self.kind, Number | Options | Members);
        if !valid_name(&self.id)
            || !valid_text(&self.label)
            || self.placeholder.as_deref().is_some_and(|p| !valid_text(p))
            || self.default_path.as_deref().is_some_and(|p| !valid_path(p))
            || self
                .max_path
                .as_deref()
                .is_some_and(|p| !valid_path(p) || !matches!(self.kind, Options | Members))
            || (self.kind == Password && (self.default.is_some() || self.default_path.is_some()))
            || (self.required && self.kind == Toggle)
            || (self.max_length.is_some() && !text_like)
            || ((self.min.is_some() || self.max.is_some()) && !ranged)
            || (self.default.is_some() && self.kind == Members)
            || (self.choices.is_empty() == (self.kind == Select))
            || self
                .choices_path
                .as_deref()
                .is_some_and(|p| self.kind != Select || !valid_path(p))
        {
            return invalid;
        }
        if let Some(max_length) = self.max_length {
            let cap = match self.kind {
                Text | Password => TEXT_MAX_LENGTH,
                Textarea => TEXTAREA_MAX_LENGTH,
                _ => OPTION_MAX_LENGTH,
            };
            if !(1..=cap).contains(&max_length) {
                return invalid;
            }
        }
        match self.kind {
            Number => {
                let bound = |n: &Option<serde_json::Number>| {
                    n.as_ref().map(|n| n.as_f64().filter(|v| v.is_finite()))
                };
                let (min, max) = (bound(&self.min), bound(&self.max));
                if matches!(min, Some(None))
                    || matches!(max, Some(None))
                    || matches!((min.flatten(), max.flatten()), (Some(a), Some(b)) if a > b)
                {
                    return invalid;
                }
            }
            Options | Members => {
                let cap = if self.kind == Members {
                    MEMBERS_MAX_ITEMS
                } else {
                    OPTIONS_MAX_ITEMS
                };
                let (fewest, most) = self.item_bounds();
                if self.min.as_ref().is_some_and(|n| count(n).is_none())
                    || self.max.as_ref().is_some_and(|n| count(n).is_none())
                    || most == 0
                    || most > cap
                    || fewest > most
                {
                    return invalid;
                }
            }
            Select => {
                let mut values = BTreeSet::new();
                if self.choices.len() > SELECT_MAX_CHOICES
                    || self.choices.iter().any(|c| {
                        c.value.is_empty()
                            || c.value.len() > 128
                            || !valid_text(&c.label)
                            || !values.insert(c.value.as_str())
                    })
                {
                    return invalid;
                }
            }
            _ => {}
        }
        if self
            .default
            .as_ref()
            .is_some_and(|default| !self.default_fits(default))
        {
            return invalid;
        }
        Ok(())
    }

    /// Whether `value` is a usable initial value (looser than a submission:
    /// options may hold empty placeholder items, text may be blank).
    fn default_fits(&self, value: &Value) -> bool {
        use UiInputKind::*;
        let max = self.effective_max_length() as usize;
        match (self.kind, value) {
            (Text, Value::String(s)) => s.chars().count() <= max && single_line(s),
            (Textarea, Value::String(s)) => s.chars().count() <= max && multi_line(s),
            (Number, Value::Number(n)) => self.number_in_range(n),
            (Toggle, Value::Bool(_)) => true,
            (Select, Value::String(s)) => self.choices.iter().any(|c| &c.value == s),
            (Datetime, Value::Number(n)) => n.as_i64().is_some_and(|v| v >= 0),
            (Options, Value::Array(items)) => {
                items.len() <= self.item_bounds().1 as usize
                    && items.iter().all(|item| {
                        item.as_str()
                            .is_some_and(|s| s.chars().count() <= max && single_line(s))
                    })
            }
            _ => false,
        }
    }

    fn number_in_range(&self, n: &Number) -> bool {
        let Some(value) = n.as_f64().filter(|v| v.is_finite()) else {
            return false;
        };
        self.min
            .as_ref()
            .and_then(Number::as_f64)
            .is_none_or(|min| value >= min)
            && self
                .max
                .as_ref()
                .and_then(Number::as_f64)
                .is_none_or(|max| value <= max)
    }

    /// The input as the host renders it: labels in the view language, `max`
    /// read from `data` when `maxPath` holds a usable one, then the initial
    /// value read from `data` when `defaultPath` holds one that fits.
    pub(crate) fn resolve(&self, data: &Value, translation: Option<&UiTranslation>) -> UiInput {
        let mut input = self.clone();
        if let Some(choices) = self
            .choices_path
            .as_deref()
            .and_then(|p| data.pointer(p))
            .and_then(|v| serde_json::from_value::<Vec<UiInputChoice>>(v.clone()).ok())
        {
            let mut candidate = input.clone();
            candidate.choices = choices;
            candidate.choices_path = None;
            if candidate.validate().is_ok() {
                input.choices = candidate.choices;
            }
        }
        input.choices_path = None;
        input.max_path = None;
        if let Some(most) = self
            .max_path
            .as_deref()
            .and_then(|path| data.pointer(path))
            .and_then(Value::as_u64)
        {
            let cap = if self.kind == UiInputKind::Members {
                MEMBERS_MAX_ITEMS
            } else {
                OPTIONS_MAX_ITEMS
            };
            let mut candidate = input.clone();
            candidate.max = Some(Number::from(most.min(u64::from(cap))));
            if candidate.validate().is_ok() {
                input.max = candidate.max;
            }
        }
        input.label = translate(translation, &self.label);
        input.placeholder = self
            .placeholder
            .as_deref()
            .map(|p| translate(translation, p));
        for choice in &mut input.choices {
            choice.label = translate(translation, &choice.label);
        }
        if let Some(value) = self
            .default_path
            .as_deref()
            .and_then(|path| data.pointer(path))
            .filter(|value| {
                !value.is_null()
                    && (input.default_fits(value)
                        || (self.kind == UiInputKind::Members && input.check(value).is_ok()))
            })
        {
            input.default = Some(value.clone());
        }
        input.default_path = None;
        input
    }

    /// Check one submitted value; returns the value to forward.
    fn check(&self, value: &Value) -> std::result::Result<Value, UiFormIssue> {
        use UiFormIssue::*;
        use UiInputKind::*;
        let max = self.effective_max_length() as usize;
        match (self.kind, value) {
            (Text | Password | Textarea, Value::String(s)) => {
                let shape_ok = if matches!(self.kind, Text | Password) {
                    single_line(s)
                } else {
                    multi_line(s)
                };
                if !shape_ok {
                    Err(InvalidType)
                } else if s.chars().count() > max {
                    Err(TooLong)
                } else if self.required && s.trim().is_empty() {
                    Err(Required)
                } else {
                    Ok(value.clone())
                }
            }
            (Number, Value::Number(n)) => {
                if self.number_in_range(n) {
                    Ok(value.clone())
                } else {
                    Err(OutOfRange)
                }
            }
            (Toggle, Value::Bool(_)) => Ok(value.clone()),
            (Select, Value::String(s)) => {
                if self.choices.iter().any(|c| &c.value == s) {
                    Ok(value.clone())
                } else {
                    Err(NotAChoice)
                }
            }
            (Datetime, Value::Number(n)) => match n.as_i64() {
                Some(v) if v >= 0 => Ok(value.clone()),
                Some(_) => Err(OutOfRange),
                None => Err(InvalidType),
            },
            (Options | Members, Value::Array(items)) => {
                let (fewest, most) = self.item_bounds();
                let mut seen = BTreeSet::new();
                for item in items {
                    let Some(text) = item.as_str() else {
                        return Err(InvalidItem);
                    };
                    let key = if self.kind == Options {
                        let trimmed = text.trim();
                        if trimmed.is_empty() || text.chars().count() > max || !single_line(text) {
                            return Err(InvalidItem);
                        }
                        trimmed
                    } else {
                        if !crate::resource::valid_resource_id(text) {
                            return Err(InvalidItem);
                        }
                        text
                    };
                    if !seen.insert(key) {
                        return Err(DuplicateItem);
                    }
                }
                if self.required && items.is_empty() {
                    Err(Required)
                } else if items.len() < fewest as usize {
                    Err(TooFewItems)
                } else if items.len() > most as usize {
                    Err(TooManyItems)
                } else {
                    Ok(value.clone())
                }
            }
            _ => Err(InvalidType),
        }
    }
}

/// Validate the submitted `values` (an object keyed by input id) of `inputs`.
/// An absent or null value takes the input's `default`; the result holds every
/// input that has a value, ready to merge into the action input. All problems
/// are reported, one per input.
pub fn validate_form_values(
    inputs: &[&UiInput],
    values: &Value,
) -> std::result::Result<Map<String, Value>, Vec<UiFormError>> {
    let empty = Map::new();
    let Some(values) = values.as_object().or(values.is_null().then_some(&empty)) else {
        return Err(vec![UiFormError {
            input: String::new(),
            issue: UiFormIssue::InvalidType,
        }]);
    };
    let mut errors: Vec<UiFormError> = values
        .keys()
        .filter(|key| !inputs.iter().any(|input| &input.id == *key))
        .map(|key| UiFormError {
            input: key.clone(),
            issue: UiFormIssue::Unknown,
        })
        .collect();
    let mut output = Map::new();
    for input in inputs {
        let value = values
            .get(&input.id)
            .filter(|v| !v.is_null())
            .or(input.default.as_ref());
        let Some(value) = value else {
            if input.required {
                errors.push(UiFormError {
                    input: input.id.clone(),
                    issue: UiFormIssue::Required,
                });
            }
            continue;
        };
        match input.check(value) {
            Ok(value) => {
                output.insert(input.id.clone(), value);
            }
            Err(issue) => errors.push(UiFormError {
                input: input.id.clone(),
                issue,
            }),
        }
    }
    if errors.is_empty() {
        Ok(output)
    } else {
        Err(errors)
    }
}
