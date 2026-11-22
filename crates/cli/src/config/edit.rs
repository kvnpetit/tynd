use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde_json::json;

use super::load::load_typed;
use crate::source_edit::{
    code_shape, is_identifier_byte, is_line_break, is_top_level_position, last_code_byte,
    matching_delimiter,
};

#[derive(Clone, Debug)]
pub(crate) struct EditableConfig {
    pub(crate) path: PathBuf,
    pub(crate) root: PathBuf,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ConfigMutation<'a> {
    AddApi(&'a str),
    RemoveApi(&'a str),
    AddPlugin {
        package: &'a str,
        permissions: &'a [String],
    },
    RemovePlugin(&'a str),
    /// Fills `updater.publicKeys` when it is absent or empty.
    SetUpdaterPublicKey(&'a str),
    /// Appends a key to `updater.publicKeys` (key rotation), keeping the others.
    AppendUpdaterPublicKey(&'a str),
}

pub(crate) fn resolve_editable_config(cwd: &Path) -> Result<EditableConfig> {
    let loaded = load_typed(cwd, None)?;
    let path = loaded.path.context("no supported tynd.config file found")?;
    Ok(EditableConfig {
        path,
        root: loaded.root,
    })
}

pub(crate) fn mutate_config_source(
    path: &Path,
    source: &str,
    mutation: ConfigMutation<'_>,
) -> Result<String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "ts" | "mts" | "cts" | "js" | "mjs" | "cjs" => {
            mutate_script_config(source, mutation, false)
        },
        "json" => mutate_json_config(source, mutation),
        "jsonc" => mutate_script_config(source, mutation, true),
        "toml" => mutate_toml_config(source, mutation),
        "yaml" | "yml" => mutate_yaml_config(source, mutation),
        _ => bail!("unsupported config format: {}", path.display()),
    }
    .with_context(|| format!("edit {}", path.display()))
}

/// The in-memory model of a data config. `serde_yaml`'s mapping keeps key
/// order (`serde_json`'s map is sorted without `preserve_order`, a feature
/// that would reorder the output of every crate in the workspace), so an edit
/// leaves every key where the user put it.
type Doc = serde_yaml::Value;

/// `"\r\n"` when `source` uses CRLF line endings, `"\n"` otherwise.
fn line_ending(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Rewrites the LF line endings of serializer output to `eol`.
fn with_line_ending(text: &str, eol: &str) -> String {
    if eol == "\n" {
        text.to_owned()
    } else {
        text.replace('\n', eol)
    }
}

/// The indentation of the first indented line of `source`, two spaces when
/// no line is indented.
fn json_indent(source: &str) -> &str {
    source
        .split('\n')
        .skip(1)
        .map(|line| {
            let end = line
                .find(|ch: char| ch != ' ' && ch != '\t')
                .unwrap_or(line.len());
            &line[..end]
        })
        .find(|indent| !indent.is_empty())
        .unwrap_or("  ")
}

fn mutate_json_config(source: &str, mutation: ConfigMutation<'_>) -> Result<String> {
    let mut value: Doc = serde_json::from_str(source).context("parse JSON config")?;
    if !mutate_value(&mut value, mutation)? {
        return Ok(source.to_owned());
    }
    let formatter = serde_json::ser::PrettyFormatter::with_indent(json_indent(source).as_bytes());
    let mut bytes = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, formatter);
    serde::Serialize::serialize(&value, &mut serializer).context("serialize JSON config")?;
    let eol = line_ending(source);
    // JSON strings escape their line breaks, so every raw `\n` is layout.
    let mut output = with_line_ending(&String::from_utf8(bytes)?, eol);
    if source.ends_with('\n') || source.trim().is_empty() {
        output.push_str(eol);
    }
    Ok(output)
}

fn mutate_yaml_config(source: &str, mutation: ConfigMutation<'_>) -> Result<String> {
    let mut value: Doc = serde_yaml::from_str(source).context("parse YAML config")?;
    if !mutate_value(&mut value, mutation)? {
        return Ok(source.to_owned());
    }
    // Key order and line endings survive; comments do not: `serde_yaml` has
    // no format-preserving editor.
    let mut output = serde_yaml::to_string(&value).context("serialize YAML config")?;
    // The serializer never writes a document marker and always ends with a
    // line break: mirror the source on both.
    if has_yaml_document_marker(source) {
        output.insert_str(0, "---\n");
    }
    if !source.ends_with('\n') && !source.trim().is_empty() {
        output.truncate(output.trim_end_matches('\n').len());
    }
    Ok(with_line_ending(&output, line_ending(source)))
}

/// Whether the first content line of `source` (after blank and comment
/// lines) is a `---` document start marker.
fn has_yaml_document_marker(source: &str) -> bool {
    source
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim_end)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .is_some_and(|line| {
            line.strip_prefix("---")
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
        })
}

/// The top-level keys a mutation can touch. Only these are edited in a TOML
/// document, so everything else (comments and layout included) stays as is.
const MUTATED_KEYS: [&str; 3] = ["apis", "plugins", "updater"];

fn mutate_toml_config(source: &str, mutation: ConfigMutation<'_>) -> Result<String> {
    let mut document: toml_edit::DocumentMut = source.parse().context("parse TOML config")?;
    let mut root = serde_yaml::Mapping::new();
    for key in MUTATED_KEYS {
        if let Some(item) = document.get(key) {
            root.insert(Doc::from(key), toml_item_to_doc(item)?);
        }
    }
    let mut value = Doc::Mapping(root);
    if !mutate_value(&mut value, mutation)? {
        return Ok(source.to_owned());
    }
    let root = value
        .as_mapping()
        .context("config root must be an object")?;
    for key in ["apis", "plugins"] {
        match root.get(key) {
            None => {
                document.remove(key);
            },
            Some(next) => sync_toml_entry(document.as_table_mut(), key, doc_to_toml(next)?),
        }
    }
    if let Some(keys) = root
        .get("updater")
        .and_then(|updater| updater.get("publicKeys"))
    {
        let keys = doc_to_toml(keys)?;
        if let Some(table) = document
            .get_mut("updater")
            .and_then(toml_edit::Item::as_table_like_mut)
        {
            sync_toml_entry(table, "publicKeys", keys);
        } else {
            let mut table = toml_edit::Table::new();
            table.insert("publicKeys", toml_edit::value(keys));
            document.insert("updater", toml_edit::Item::Table(table));
        }
    }
    Ok(document.to_string())
}

/// Sets `key` of `table` to `next`. Mutations only append or remove array
/// entries, so an existing array is edited in place: the entries that survive
/// keep their formatting and comments.
fn sync_toml_entry(table: &mut dyn toml_edit::TableLike, key: &str, next: toml_edit::Value) {
    let Some(item) = table.get_mut(key) else {
        table.insert(key, toml_edit::value(next));
        return;
    };
    if let (toml_edit::Item::ArrayOfTables(tables), toml_edit::Value::Array(entries)) =
        (&mut *item, &next)
    {
        if sync_toml_tables(tables, entries) {
            return;
        }
    }
    let (Some(array), toml_edit::Value::Array(next)) = (item.as_array_mut(), &next) else {
        *item = toml_edit::value(next);
        return;
    };
    let wanted: Vec<Option<Doc>> = next
        .iter()
        .map(|entry| toml_value_to_doc(entry).ok())
        .collect();
    let mut kept = 0;
    array.retain(|entry| {
        let keep = wanted
            .get(kept)
            .is_some_and(|want| want.is_some() && *want == toml_value_to_doc(entry).ok());
        if keep {
            kept += 1;
        }
        keep
    });
    for entry in next.iter().skip(kept) {
        array.push(entry.clone());
    }
}

/// Edits a `[[key]]` array of tables in place, like `sync_toml_entry` does
/// for an inline array, so the surviving tables keep their comments. A bare
/// package string becomes its `{ pkg = "..." }` table form, which the plugin
/// entry model accepts. Returns `false`, leaving `tables` untouched, when an
/// entry has no table form.
fn sync_toml_tables(tables: &mut toml_edit::ArrayOfTables, next: &toml_edit::Array) -> bool {
    let Some(entries) = next
        .iter()
        .map(|entry| match entry {
            toml_edit::Value::InlineTable(table) => Some(table.clone().into_table()),
            toml_edit::Value::String(package) => {
                let mut table = toml_edit::Table::new();
                table.insert("pkg", toml_edit::value(package.value().as_str()));
                Some(table)
            },
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let wanted: Vec<Option<Doc>> = entries
        .iter()
        .map(|table| toml_table_to_doc(table.iter()).ok())
        .collect();
    let mut kept = 0;
    tables.retain(|table| {
        let keep = wanted
            .get(kept)
            .is_some_and(|want| want.is_some() && *want == toml_table_to_doc(table.iter()).ok());
        if keep {
            kept += 1;
        }
        keep
    });
    for table in entries.into_iter().skip(kept) {
        tables.push(table);
    }
    true
}

fn toml_item_to_doc(item: &toml_edit::Item) -> Result<Doc> {
    match item {
        toml_edit::Item::None => Ok(Doc::Null),
        toml_edit::Item::Value(value) => toml_value_to_doc(value),
        toml_edit::Item::Table(table) => toml_table_to_doc(table.iter()),
        toml_edit::Item::ArrayOfTables(tables) => Ok(Doc::Sequence(
            tables
                .iter()
                .map(|table| toml_table_to_doc(table.iter()))
                .collect::<Result<_>>()?,
        )),
    }
}

fn toml_table_to_doc<'a>(
    entries: impl Iterator<Item = (&'a str, &'a toml_edit::Item)>,
) -> Result<Doc> {
    let mut mapping = serde_yaml::Mapping::new();
    for (key, item) in entries {
        mapping.insert(Doc::from(key), toml_item_to_doc(item)?);
    }
    Ok(Doc::Mapping(mapping))
}

fn toml_value_to_doc(value: &toml_edit::Value) -> Result<Doc> {
    Ok(match value {
        toml_edit::Value::String(text) => Doc::from(text.value().as_str()),
        toml_edit::Value::Integer(number) => Doc::from(*number.value()),
        toml_edit::Value::Float(number) => Doc::from(*number.value()),
        toml_edit::Value::Boolean(flag) => Doc::from(*flag.value()),
        toml_edit::Value::Datetime(datetime) => Doc::from(datetime.value().to_string()),
        toml_edit::Value::Array(array) => {
            Doc::Sequence(array.iter().map(toml_value_to_doc).collect::<Result<_>>()?)
        },
        toml_edit::Value::InlineTable(table) => {
            let mut mapping = serde_yaml::Mapping::new();
            for (key, value) in table {
                mapping.insert(Doc::from(key), toml_value_to_doc(value)?);
            }
            Doc::Mapping(mapping)
        },
    })
}

fn doc_to_toml(value: &Doc) -> Result<toml_edit::Value> {
    Ok(match value {
        Doc::String(text) => text.as_str().into(),
        Doc::Bool(flag) => (*flag).into(),
        Doc::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(integer), _) => integer.into(),
            (None, Some(float)) => float.into(),
            (None, None) => bail!("config number {number} does not fit TOML"),
        },
        Doc::Sequence(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(doc_to_toml(item)?);
            }
            toml_edit::Value::Array(array)
        },
        Doc::Mapping(mapping) => {
            let mut table = toml_edit::InlineTable::new();
            for (key, item) in mapping {
                let key = key.as_str().context("config keys must be strings")?;
                table.insert(key, doc_to_toml(item)?);
            }
            toml_edit::Value::InlineTable(table)
        },
        Doc::Null | Doc::Tagged(_) => bail!("config value has no TOML equivalent"),
    })
}

fn mutate_value(value: &mut Doc, mutation: ConfigMutation<'_>) -> Result<bool> {
    let object = value
        .as_mapping_mut()
        .context("config root must be an object")?;
    match mutation {
        ConfigMutation::AddApi(api) => {
            let apis = object
                .entry(Doc::from("apis"))
                .or_insert_with(|| Doc::Sequence(Vec::new()))
                .as_sequence_mut()
                .context("config apis must be an array")?;
            ensure_string_array("apis", apis)?;
            if apis.iter().any(|value| value.as_str() == Some(api)) {
                return Ok(false);
            }
            apis.push(Doc::from(api));
            Ok(true)
        },
        ConfigMutation::RemoveApi(api) => remove_array_entries(object, "apis", |values| {
            ensure_string_array("apis", values)?;
            Ok(|value: &Doc| value.as_str() == Some(api))
        }),
        ConfigMutation::AddPlugin {
            package,
            permissions,
        } => {
            let plugins = object
                .entry(Doc::from("plugins"))
                .or_insert_with(|| Doc::Sequence(Vec::new()))
                .as_sequence_mut()
                .context("config plugins must be an array")?;
            ensure_plugin_array(plugins)?;
            if plugins
                .iter()
                .any(|entry| plugin_package(entry) == Some(package))
            {
                return Ok(false);
            }
            plugins.push(if permissions.is_empty() {
                Doc::from(package)
            } else {
                let mut entry = serde_yaml::Mapping::new();
                entry.insert(Doc::from("pkg"), Doc::from(package));
                entry.insert(
                    Doc::from("permissions"),
                    Doc::Sequence(
                        permissions
                            .iter()
                            .map(|name| Doc::from(name.as_str()))
                            .collect(),
                    ),
                );
                Doc::Mapping(entry)
            });
            Ok(true)
        },
        ConfigMutation::RemovePlugin(package) => {
            remove_array_entries(object, "plugins", |values| {
                ensure_plugin_array(values)?;
                Ok(|entry: &Doc| plugin_package(entry) == Some(package))
            })
        },
        ConfigMutation::SetUpdaterPublicKey(public_key)
        | ConfigMutation::AppendUpdaterPublicKey(public_key) => {
            let append = matches!(mutation, ConfigMutation::AppendUpdaterPublicKey(_));
            // Only the keys: the endpoint is the user's to choose, and a
            // placeholder would ship a broken updater.
            let updater = object
                .entry(Doc::from("updater"))
                .or_insert_with(|| Doc::Mapping(serde_yaml::Mapping::new()));
            let updater = updater
                .as_mapping_mut()
                .context("config updater must be an object")?;
            if append {
                let keys = updater
                    .entry(Doc::from("publicKeys"))
                    .or_insert_with(|| Doc::Sequence(Vec::new()))
                    .as_sequence_mut()
                    .context("config updater.publicKeys must be an array")?;
                ensure_string_array("updater.publicKeys", keys)?;
                if keys.iter().any(|key| key.as_str() == Some(public_key)) {
                    return Ok(false);
                }
                keys.push(Doc::from(public_key));
                return Ok(true);
            }
            // An empty list fails validation, so fill it; any other value is
            // the user's and stays untouched.
            let has_keys = updater
                .get("publicKeys")
                .is_some_and(|keys| keys.as_sequence().is_none_or(|list| !list.is_empty()));
            if has_keys {
                return Ok(false);
            }
            updater.insert(
                Doc::from("publicKeys"),
                Doc::Sequence(vec![Doc::from(public_key)]),
            );
            Ok(true)
        },
    }
}

/// Removes the entries of the `key` array that the matcher built by `check`
/// (which validates the array first) selects. An empty list means the same as
/// no list, so an emptied one is dropped: add then remove restores the file.
fn remove_array_entries<F: Fn(&Doc) -> bool>(
    object: &mut serde_yaml::Mapping,
    key: &str,
    check: impl FnOnce(&[Doc]) -> Result<F>,
) -> Result<bool> {
    let Some(values) = object.get_mut(key) else {
        return Ok(false);
    };
    let values = values
        .as_sequence_mut()
        .with_context(|| format!("config {key} must be an array"))?;
    let selected = check(values)?;
    let before = values.len();
    values.retain(|value| !selected(value));
    let removed = values.len() != before;
    if removed && values.is_empty() {
        object.shift_remove(key);
    }
    Ok(removed)
}

fn ensure_string_array(label: &str, values: &[Doc]) -> Result<()> {
    ensure!(
        values.iter().all(Doc::is_string),
        "config {label} must contain only strings"
    );
    Ok(())
}

fn ensure_plugin_array(values: &[Doc]) -> Result<()> {
    ensure!(
        values.iter().all(|entry| plugin_package(entry).is_some()),
        "config plugins must contain package strings or objects with a string pkg"
    );
    Ok(())
}

fn plugin_package(value: &Doc) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get("pkg").and_then(Doc::as_str))
}

fn mutate_script_config(
    source: &str,
    mutation: ConfigMutation<'_>,
    json_keys: bool,
) -> Result<String> {
    let (object_open, object_close) = if json_keys {
        find_json_root_object(source)?
    } else {
        find_script_config_object(source)?
    };
    match mutation {
        ConfigMutation::AddApi(api) => add_script_array_entry(
            source,
            object_open,
            object_close,
            "apis",
            &serde_json::to_string(api)?,
            Some(api),
            json_keys,
        ),
        ConfigMutation::RemoveApi(api) => remove_script_array_entry(
            source,
            object_open,
            object_close,
            "apis",
            api,
            static_string_expression,
        ),
        ConfigMutation::AddPlugin {
            package,
            permissions,
        } => {
            if let Some((open, close)) =
                find_top_level_array_property(source, object_open, object_close, "plugins")?
            {
                let entries = script_plugin_entries(source, open, close)?;
                ensure!(
                    entries.iter().all(Option::is_some),
                    "config plugins contains dynamic entries; edit it manually"
                );
                if entries
                    .iter()
                    .any(|entry| entry.as_deref() == Some(package))
                {
                    return Ok(source.to_owned());
                }
            }
            let entry = if permissions.is_empty() {
                serde_json::to_string(package)?
            } else {
                serde_json::to_string(&json!({
                    "pkg": package,
                    "permissions": permissions,
                }))?
            };
            add_script_array_entry(
                source,
                object_open,
                object_close,
                "plugins",
                &entry,
                None,
                json_keys,
            )
        },
        ConfigMutation::RemovePlugin(package) => remove_script_array_entry(
            source,
            object_open,
            object_close,
            "plugins",
            package,
            script_plugin_package,
        ),
        ConfigMutation::SetUpdaterPublicKey(public_key) => set_script_updater_public_key(
            source,
            object_open,
            object_close,
            public_key,
            json_keys,
            false,
        ),
        ConfigMutation::AppendUpdaterPublicKey(public_key) => set_script_updater_public_key(
            source,
            object_open,
            object_close,
            public_key,
            json_keys,
            true,
        ),
    }
}

fn set_script_updater_public_key(
    source: &str,
    object_open: usize,
    object_close: usize,
    public_key: &str,
    json_keys: bool,
    append: bool,
) -> Result<String> {
    let rendered = serde_json::to_string(public_key)?;
    let eol = line_ending(source);
    let property_key = if json_keys {
        "\"publicKeys\""
    } else {
        "publicKeys"
    };
    if let Some(updater_open) =
        find_top_level_property_value(source, object_open, object_close, "updater")?
    {
        ensure!(
            source.as_bytes().get(updater_open) == Some(&b'{'),
            "config updater must be a static object; edit it manually"
        );
        let shape = code_shape(source, false);
        let updater_close = matching_delimiter(&shape, updater_open, b'{', b'}')?;
        if let Some(keys_open) =
            find_top_level_property_value(source, updater_open, updater_close, "publicKeys")?
        {
            // An empty list fails validation, so fill it; any other value is
            // the user's and stays untouched (unless rotating, which appends).
            if shape.as_bytes().get(keys_open) != Some(&b'[') {
                ensure!(
                    !append,
                    "config updater.publicKeys must be a static array; edit it manually"
                );
                return Ok(source.to_owned());
            }
            let keys_close = matching_delimiter(&shape, keys_open, b'[', b']')?;
            if append && last_code_byte(source, keys_open, keys_close).is_some() {
                let entries = script_string_entries(source, keys_open, keys_close)?;
                ensure!(
                    entries.iter().all(Option::is_some),
                    "config updater.publicKeys contains dynamic entries; edit it manually"
                );
                if entries
                    .iter()
                    .any(|entry| entry.as_deref() == Some(public_key))
                {
                    return Ok(source.to_owned());
                }
                return Ok(insert_array_entry(source, keys_open, keys_close, &rendered));
            }
            if last_code_byte(source, keys_open, keys_close).is_some() {
                return Ok(source.to_owned());
            }
            let mut next = source.to_owned();
            next.replace_range(keys_open..=keys_close, &format!("[{rendered}]"));
            return Ok(next);
        }
        let separator = comma_unless_blank(&source[updater_open + 1..updater_close]);
        let mut next = source.to_owned();
        next.insert_str(
            updater_open + 1,
            &format!("{eol}    {property_key}: [{rendered}]{separator}"),
        );
        return Ok(next);
    }

    let updater_key = if json_keys { "\"updater\"" } else { "updater" };
    let separator = comma_unless_blank(&source[object_open + 1..object_close]);
    let mut next = source.to_owned();
    next.insert_str(
        object_open + 1,
        &format!("{eol}  {updater_key}: {{ {property_key}: [{rendered}] }}{separator}"),
    );
    Ok(next)
}

/// `","` when `body` holds code (comments and blanks aside), so a property
/// inserted at its start must be followed by a separator; `""` otherwise.
fn comma_unless_blank(body: &str) -> &'static str {
    if code_shape(body, false).trim().is_empty() {
        ""
    } else {
        ","
    }
}

fn add_script_array_entry(
    source: &str,
    object_open: usize,
    object_close: usize,
    property: &str,
    rendered: &str,
    string_identity: Option<&str>,
    json_keys: bool,
) -> Result<String> {
    if let Some((open, close)) =
        find_top_level_array_property(source, object_open, object_close, property)?
    {
        if let Some(identity) = string_identity {
            let entries = script_string_entries(source, open, close)?;
            ensure!(
                entries.iter().all(Option::is_some),
                "config {property} contains dynamic entries; edit it manually"
            );
            if entries
                .iter()
                .any(|entry| entry.as_deref() == Some(identity))
            {
                return Ok(source.to_owned());
            }
        }
        return Ok(insert_array_entry(source, open, close, rendered));
    }

    let key = if json_keys {
        serde_json::to_string(property)?
    } else {
        property.to_owned()
    };
    let mut next = source.to_owned();
    next.insert_str(
        object_open + 1,
        &format!("{}  {key}: [{rendered}],", line_ending(source)),
    );
    Ok(next)
}

/// Appends `rendered` to the array spanning `open..=close` (bracket indices).
/// A one-line array stays on one line; a multi-line array gets the entry on its
/// own line before the closing bracket. The separator goes right after the
/// last code byte, so string entries and trailing comments are respected.
fn insert_array_entry(source: &str, open: usize, close: usize, rendered: &str) -> String {
    let last = last_code_byte(source, open, close);
    let needs_separator = last.is_some_and(|index| source.as_bytes()[index] != b',');
    let mut next = source.to_owned();
    if !source.as_bytes()[open..close]
        .iter()
        .copied()
        .any(is_line_break)
    {
        let insert_at = last.map_or(open + 1, |index| index + 1);
        let prefix = match (last.is_some(), needs_separator) {
            (false, _) => "",
            (true, true) => ", ",
            (true, false) => " ",
        };
        next.insert_str(insert_at, &format!("{prefix}{rendered}"));
        return next;
    }
    let line_start = source[..close]
        .rfind(['\n', '\r'])
        .map_or(0, |index| index + 1);
    let eol = line_ending(source);
    if source[line_start..close].trim().is_empty() {
        let indent = &source[line_start..close];
        next.insert_str(line_start, &format!("{indent}  {rendered},{eol}"));
    } else {
        next.insert_str(close, &format!("{eol}    {rendered},{eol}  "));
    }
    if needs_separator {
        if let Some(index) = last {
            next.insert(index + 1, ',');
        }
    }
    next
}

/// Removes the entries of the top-level `property` array whose identity (read
/// by `identity`, `None` for a dynamic entry) is `target`. A one-line array
/// stays on one line.
fn remove_script_array_entry(
    source: &str,
    object_open: usize,
    object_close: usize,
    property: &str,
    target: &str,
    identity: fn(&str) -> Result<Option<String>>,
) -> Result<String> {
    let Some((open, close)) =
        find_top_level_array_property(source, object_open, object_close, property)?
    else {
        return Ok(source.to_owned());
    };
    let (entries, tail) = array_entries_with_comments(source, open, close)?;
    let mut retained = Vec::new();
    let mut removed = false;
    for entry in entries {
        let parsed = identity(entry.value)?;
        ensure!(
            parsed.is_some(),
            "config {property} contains dynamic entries; edit it manually"
        );
        if parsed.as_deref() == Some(target) {
            removed = true;
        } else {
            retained.push(entry);
        }
    }
    if !removed {
        return Ok(source.to_owned());
    }
    if retained.is_empty() && tail.is_empty() {
        // An empty list means the same as no list: drop the property `tynd add`
        // created so add then remove restores the original file.
        let (key_start, _) = find_top_level_property(source, object_open, object_close, property)?
            .context("removed array property disappeared")?;
        return Ok(remove_script_property(source, key_start, close));
    }
    let one_line = !source.as_bytes()[open..close]
        .iter()
        .copied()
        .any(is_line_break);
    let replacement = if one_line {
        let mut line = retained
            .iter()
            .map(|entry| {
                let mut text = entry.value.to_owned();
                for comment in &entry.trailing {
                    text.push(' ');
                    text.push_str(comment);
                }
                text
            })
            .collect::<Vec<_>>()
            .join(", ");
        for comment in &tail {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(comment);
        }
        line
    } else {
        let eol = line_ending(source);
        let mut block = String::new();
        for entry in &retained {
            block.push_str(eol);
            block.push_str("    ");
            block.push_str(entry.value);
            block.push(',');
            for comment in &entry.trailing {
                block.push(' ');
                block.push_str(comment);
            }
        }
        for comment in &tail {
            block.push_str(eol);
            block.push_str("    ");
            block.push_str(comment);
        }
        block.push_str(eol);
        block.push_str("  ");
        block
    };
    let mut next = source.to_owned();
    next.replace_range(open + 1..close, &replacement);
    Ok(next)
}

/// Removes the property whose key starts at `key_start` and whose value ends
/// at `value_close`, with its separator. A property alone on its line (as
/// `tynd add` writes it) goes with that line.
fn remove_script_property(source: &str, key_start: usize, value_close: usize) -> String {
    let bytes = source.as_bytes();
    let is_blank = |byte: u8| matches!(byte, b' ' | b'\t');
    let mut start = key_start;
    let mut end = value_close + 1;
    let mut after = end;
    while bytes.get(after).copied().is_some_and(is_blank) {
        after += 1;
    }
    if bytes.get(after) == Some(&b',') {
        end = after + 1;
        while start > 0 && is_blank(bytes[start - 1]) {
            start -= 1;
        }
        if start > 0 && bytes[start - 1] == b'\n' {
            start -= 1;
            if start > 0 && bytes[start - 1] == b'\r' {
                start -= 1;
            }
        } else {
            start = key_start;
            while bytes.get(end).copied().is_some_and(is_blank) {
                end += 1;
            }
        }
    } else {
        // Last property: take the separator before it instead.
        let mut before = key_start;
        while before > 0 && bytes[before - 1].is_ascii_whitespace() {
            before -= 1;
        }
        if before > 0 && bytes[before - 1] == b',' {
            start = before - 1;
        } else {
            end = after;
        }
    }
    let mut next = source.to_owned();
    next.replace_range(start..end, "");
    next
}

/// One entry of an array literal: its code (with any leading comments) and
/// the comments that follow it on the same line, before or after its comma.
struct ArrayEntry<'a> {
    value: &'a str,
    trailing: Vec<&'a str>,
}

/// Splits the array spanning `open..=close` into entries with their comments
/// attached, plus the comment-only `tail` before the closing bracket. A
/// comment after an entry's comma on the same line belongs to that entry.
fn array_entries_with_comments(
    source: &str,
    open: usize,
    close: usize,
) -> Result<(Vec<ArrayEntry<'_>>, Vec<&str>)> {
    let mut entries: Vec<ArrayEntry<'_>> = Vec::new();
    let mut tail = Vec::new();
    for (start, end) in top_level_entry_ranges(source, open + 1, close)? {
        let mut body_start = start;
        if let Some(previous) = entries.last_mut() {
            if let Some(head_end) = same_line_trivia_end(source, start, end)? {
                let comment = source[start..head_end].trim();
                if !comment.is_empty() {
                    previous.trailing.push(comment);
                }
                body_start = head_end;
            }
        }
        // `body_start - 1` is the separator, the opening bracket or a line
        // break ending trivia: a code position for `last_code_byte`.
        match last_code_byte(source, body_start - 1, end) {
            None => {
                let comment = source[body_start..end].trim();
                if !comment.is_empty() {
                    tail.push(comment);
                }
            },
            Some(last) => {
                let after = source[last + 1..end].trim();
                entries.push(ArrayEntry {
                    value: source[body_start..=last].trim(),
                    trailing: if after.is_empty() {
                        Vec::new()
                    } else {
                        vec![after]
                    },
                });
            },
        }
    }
    Ok((entries, tail))
}

/// End (past the line break) of the whitespace and comments that open
/// `start..end` and finish on the same line; `None` when code comes first or
/// the span ends without a line break.
fn same_line_trivia_end(source: &str, start: usize, end: usize) -> Result<Option<usize>> {
    let bytes = source.as_bytes();
    let mut index = start;
    while index < end {
        match (bytes[index], bytes.get(index + 1).copied()) {
            (b'\r', Some(b'\n')) => return Ok(Some(index + 2)),
            (b'\n' | b'\r', _) => return Ok(Some(index + 1)),
            (b' ' | b'\t', _) => index += 1,
            (b'/', Some(b'/')) => {
                while index < end && !is_line_break(bytes[index]) {
                    index += 1;
                }
            },
            (b'/', Some(b'*')) => {
                let length = source[index + 2..]
                    .find("*/")
                    .context("unterminated block comment")?;
                index += 2 + length + 2;
            },
            _ => return Ok(None),
        }
    }
    Ok(None)
}

/// True when the array entry `start..end` holds only whitespace and comments,
/// such as the span after a trailing comma. `start - 1` is the separator or
/// the opening bracket.
fn is_trivia_entry(source: &str, start: usize, end: usize) -> bool {
    last_code_byte(source, start - 1, end).is_none()
}

fn script_string_entries(source: &str, open: usize, close: usize) -> Result<Vec<Option<String>>> {
    top_level_entry_ranges(source, open + 1, close)?
        .into_iter()
        .filter_map(|(start, end)| {
            let entry = &source[start..end];
            (!is_trivia_entry(source, start, end)).then_some(static_string_expression(entry))
        })
        .collect()
}

fn script_plugin_entries(source: &str, open: usize, close: usize) -> Result<Vec<Option<String>>> {
    top_level_entry_ranges(source, open + 1, close)?
        .into_iter()
        .filter_map(|(start, end)| {
            let entry = &source[start..end];
            (!is_trivia_entry(source, start, end)).then_some(script_plugin_package(entry))
        })
        .collect()
}

fn script_plugin_package(entry: &str) -> Result<Option<String>> {
    if let Some(value) = static_string_expression(entry)? {
        return Ok(Some(value));
    }
    let start = skip_trivia(entry, 0)?;
    if entry.as_bytes().get(start) != Some(&b'{') {
        return Ok(None);
    }
    let shape = code_shape(entry, false);
    let close = matching_delimiter(&shape, start, b'{', b'}')?;
    let Some(value) = find_top_level_property_value(entry, start, close, "pkg")? else {
        return Ok(None);
    };
    static_string_at(entry, value)
}

fn static_string_expression(source: &str) -> Result<Option<String>> {
    let start = skip_trivia(source, 0)?;
    static_string_at(source, start)
}

fn static_string_at(source: &str, start: usize) -> Result<Option<String>> {
    let Some(quote @ (b'\'' | b'"')) = source.as_bytes().get(start).copied() else {
        return Ok(None);
    };
    let mut escaped = false;
    let mut index = start + 1;
    while let Some(byte) = source.as_bytes().get(index).copied() {
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == quote {
            let raw = &source[start + 1..index];
            if raw.contains('\\') {
                return Ok(None);
            }
            return Ok(Some(raw.to_owned()));
        }
        index += 1;
    }
    bail!("unterminated string in config array")
}

fn find_json_root_object(source: &str) -> Result<(usize, usize)> {
    let open = skip_trivia(source, 0)?;
    ensure!(
        source.as_bytes().get(open) == Some(&b'{'),
        "JSONC config root must be an object"
    );
    let shape = code_shape(source, false);
    let close = matching_delimiter(&shape, open, b'{', b'}')?;
    ensure!(
        skip_trivia(source, close + 1)? == source.len(),
        "JSONC config contains data after its root object"
    );
    Ok((open, close))
}

fn find_script_config_object(source: &str) -> Result<(usize, usize)> {
    let shape = code_shape(source, false);
    let markers = [
        ("export default", false),
        ("module.exports", true),
        ("exports.default", true),
        ("export =", false),
    ];
    let (marker, needs_assignment, start) = markers
        .iter()
        .filter_map(|(marker, assignment)| {
            shape
                .find(marker)
                .map(|start| (*marker, *assignment, start))
        })
        .min_by_key(|(_, _, start)| *start)
        .context("config mutation requires a static default export")?;
    let mut cursor = skip_trivia(source, start + marker.len())?;
    if needs_assignment {
        ensure!(
            source.as_bytes().get(cursor) == Some(&b'='),
            "CommonJS config export is missing '='"
        );
        cursor = skip_trivia(source, cursor + 1)?;
    }
    for wrapper in ["defineConfig", "defineTyndConfig"] {
        if source[cursor..].starts_with(wrapper)
            && !source
                .as_bytes()
                .get(cursor + wrapper.len())
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
        {
            cursor = skip_trivia(source, cursor + wrapper.len())?;
            ensure!(
                source.as_bytes().get(cursor) == Some(&b'('),
                "{wrapper} config wrapper is missing '('"
            );
            cursor = skip_trivia(source, cursor + 1)?;
            break;
        }
    }
    ensure!(
        source.as_bytes().get(cursor) == Some(&b'{'),
        "config mutation requires a static object literal; edit dynamic configs manually"
    );
    let close = matching_delimiter(&shape, cursor, b'{', b'}')?;
    Ok((cursor, close))
}

fn find_top_level_array_property(
    source: &str,
    object_open: usize,
    object_close: usize,
    property: &str,
) -> Result<Option<(usize, usize)>> {
    let Some(open) = find_top_level_property_value(source, object_open, object_close, property)?
    else {
        return Ok(None);
    };
    ensure!(
        source.as_bytes().get(open) == Some(&b'['),
        "{property} must be an array literal"
    );
    let shape = code_shape(source, false);
    let close = matching_delimiter(&shape, open, b'[', b']')?;
    Ok(Some((open, close)))
}

fn find_top_level_property_value(
    source: &str,
    object_open: usize,
    object_close: usize,
    property: &str,
) -> Result<Option<usize>> {
    Ok(
        find_top_level_property(source, object_open, object_close, property)?
            .map(|(_, value)| value),
    )
}

/// Start of the key (its opening quote when quoted) and of the value of the
/// top-level `property` of the object spanning `object_open..=object_close`.
fn find_top_level_property(
    source: &str,
    object_open: usize,
    object_close: usize,
    property: &str,
) -> Result<Option<(usize, usize)>> {
    let shape = code_shape(source, true);
    ensure!(
        object_open < object_close && object_close <= shape.len(),
        "invalid config object bounds"
    );
    let pattern = regex::Regex::new(&format!(r"{}\s*:", regex::escape(property)))?;
    let scope = &shape[object_open + 1..object_close];
    for found in pattern.find_iter(scope) {
        let absolute = object_open + 1 + found.start();
        if !is_top_level_position(&shape, object_open + 1, absolute)
            || absolute.checked_sub(1).is_some_and(|before| {
                shape
                    .as_bytes()
                    .get(before)
                    .is_some_and(|byte| is_identifier_byte(*byte))
            })
        {
            continue;
        }
        let value = skip_trivia(source, object_open + 1 + found.end())?;
        ensure!(value < object_close, "{property} has no value");
        let quoted = absolute
            .checked_sub(1)
            .is_some_and(|before| matches!(source.as_bytes()[before], b'"' | b'\''));
        let key_start = if quoted { absolute - 1 } else { absolute };
        return Ok(Some((key_start, value)));
    }
    Ok(None)
}

fn top_level_entry_ranges(source: &str, start: usize, end: usize) -> Result<Vec<(usize, usize)>> {
    ensure!(
        start <= end && end <= source.len(),
        "invalid config array bounds"
    );
    let shape = code_shape(source, false);
    let mut ranges = Vec::new();
    let mut entry_start = start;
    let mut depth = 0_i32;
    for (offset, byte) in shape.as_bytes()[start..end].iter().copied().enumerate() {
        match byte {
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => depth -= 1,
            b',' if depth == 0 => {
                let separator = start + offset;
                ranges.push((entry_start, separator));
                entry_start = separator + 1;
            },
            _ => {},
        }
        ensure!(depth >= 0, "unbalanced delimiter in config array");
    }
    ensure!(depth == 0, "unbalanced delimiter in config array");
    ranges.push((entry_start, end));
    Ok(ranges)
}

fn skip_trivia(source: &str, mut offset: usize) -> Result<usize> {
    loop {
        while source
            .as_bytes()
            .get(offset)
            .is_some_and(u8::is_ascii_whitespace)
        {
            offset += 1;
        }
        let rest = &source[offset..];
        if let Some(comment) = rest.strip_prefix("//") {
            offset += 2 + comment.find('\n').map_or(comment.len(), |index| index + 1);
        } else if let Some(comment) = rest.strip_prefix("/*") {
            let end = comment.find("*/").context("unterminated block comment")?;
            offset += 2 + end + 2;
        } else {
            return Ok(offset);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{mutate_config_source, ConfigMutation};

    fn set_key(file: &str, source: &str) -> String {
        mutate_config_source(
            Path::new(file),
            source,
            ConfigMutation::SetUpdaterPublicKey("KEY"),
        )
        .expect("edit succeeds")
    }

    #[test]
    fn updater_public_key_is_written_as_lists() {
        let json: serde_json::Value =
            serde_json::from_str(&set_key("tynd.config.json", "{}")).expect("valid JSON");
        assert_eq!(
            json["updater"],
            serde_json::json!({ "publicKeys": ["KEY"] })
        );

        let script = set_key("tynd.config.ts", "export default {};\n");
        assert!(
            script.contains("updater: { publicKeys: [\"KEY\"] }"),
            "{script}"
        );

        let existing = set_key(
            "tynd.config.ts",
            "export default { updater: { endpoints: [\"https://u.example.com\"] } };\n",
        );
        assert!(existing.contains("publicKeys: [\"KEY\"]"), "{existing}");
    }

    #[test]
    fn existing_public_keys_are_left_unchanged() {
        let source = "export default { updater: { publicKeys: [\"OLD\"] } };\n";
        assert_eq!(set_key("tynd.config.ts", source), source);
        let source = "{ \"updater\": { \"publicKeys\": [\"OLD\"] } }";
        let json: serde_json::Value =
            serde_json::from_str(&set_key("tynd.config.json", source)).expect("valid JSON");
        assert_eq!(json["updater"]["publicKeys"], serde_json::json!(["OLD"]));
    }

    #[test]
    fn empty_public_keys_are_filled() {
        let script = set_key(
            "tynd.config.ts",
            "export default { updater: { endpoints: [\"https://u.example.com\"], publicKeys: [ /* none */ ] } };\n",
        );
        assert!(script.contains("publicKeys: [\"KEY\"] }"), "{script}");
        let json: serde_json::Value = serde_json::from_str(&set_key(
            "tynd.config.json",
            "{ \"updater\": { \"endpoints\": [\"https://u.example.com\"], \"publicKeys\": [] } }",
        ))
        .expect("valid JSON");
        assert_eq!(json["updater"]["publicKeys"], serde_json::json!(["KEY"]));
    }

    fn append_key(file: &str, source: &str) -> String {
        mutate_config_source(
            Path::new(file),
            source,
            ConfigMutation::AppendUpdaterPublicKey("NEW"),
        )
        .expect("edit succeeds")
    }

    #[test]
    fn rotated_public_key_is_appended_after_the_old_one() {
        let script = append_key(
            "tynd.config.ts",
            "export default { updater: { publicKeys: [\"OLD\"] } };\n",
        );
        assert!(
            script.contains("publicKeys: [\"OLD\", \"NEW\"]"),
            "{script}"
        );
        assert_eq!(append_key("tynd.config.ts", &script), script);
        let created = append_key("tynd.config.ts", "export default {};\n");
        assert!(created.contains("publicKeys: [\"NEW\"]"), "{created}");
        let json: serde_json::Value = serde_json::from_str(&append_key(
            "tynd.config.json",
            "{ \"updater\": { \"publicKeys\": [\"OLD\"] } }",
        ))
        .expect("valid JSON");
        assert_eq!(
            json["updater"]["publicKeys"],
            serde_json::json!(["OLD", "NEW"])
        );
    }

    fn add_api(file: &str, source: &str, api: &str) -> String {
        mutate_config_source(Path::new(file), source, ConfigMutation::AddApi(api))
            .expect("edit succeeds")
    }

    #[test]
    fn second_api_extends_a_one_line_array() {
        let source = "export default {\n  apis: [\"dialog\"],\n};\n";
        assert_eq!(
            add_api("tynd.config.ts", source, "tray"),
            "export default {\n  apis: [\"dialog\", \"tray\"],\n};\n"
        );
        assert_eq!(
            add_api(
                "tynd.config.ts",
                "export default { apis: [\"dialog\",] };\n",
                "tray"
            ),
            "export default { apis: [\"dialog\", \"tray\"] };\n"
        );
        assert_eq!(
            add_api("tynd.config.ts", "export default { apis: [] };\n", "tray"),
            "export default { apis: [\"tray\"] };\n"
        );
    }

    #[test]
    fn api_is_appended_to_a_multi_line_array() {
        let source = "export default {\n  apis: [\n    \"dialog\" // first\n  ],\n};\n";
        assert_eq!(
            add_api("tynd.config.ts", source, "tray"),
            "export default {\n  apis: [\n    \"dialog\", // first\n    \"tray\",\n  ],\n};\n"
        );
        let source = "export default {\n  apis: [\n    \"dialog\",\n  ],\n};\n";
        assert_eq!(
            add_api("tynd.config.ts", source, "tray"),
            "export default {\n  apis: [\n    \"dialog\",\n    \"tray\",\n  ],\n};\n"
        );
        let source = "export default {\n  apis: [\n    \"dialog\"],\n};\n";
        assert_eq!(
            add_api("tynd.config.ts", source, "tray"),
            "export default {\n  apis: [\n    \"dialog\",\n    \"tray\",\n  ],\n};\n"
        );
    }

    #[test]
    fn repeated_api_additions_stay_valid() {
        let ts = add_api("tynd.config.ts", "export default {};\n", "dialog");
        let ts = add_api("tynd.config.ts", &ts, "tray");
        let ts = add_api("tynd.config.ts", &ts, "tray");
        assert!(ts.contains("apis: [\"dialog\", \"tray\"],"), "{ts}");

        let jsonc = add_api("tynd.config.jsonc", "{}", "dialog");
        let jsonc = add_api("tynd.config.jsonc", &jsonc, "tray");
        assert!(
            jsonc.contains("\"apis\": [\"dialog\", \"tray\"],"),
            "{jsonc}"
        );

        let json = add_api("tynd.config.json", "{}", "dialog");
        let json = add_api("tynd.config.json", &json, "tray");
        let json = add_api("tynd.config.json", &json, "tray");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(value["apis"], serde_json::json!(["dialog", "tray"]));
    }

    fn remove_api(file: &str, source: &str, api: &str) -> String {
        mutate_config_source(Path::new(file), source, ConfigMutation::RemoveApi(api))
            .expect("edit succeeds")
    }

    #[test]
    fn api_is_removed_from_script_configs() {
        assert_eq!(
            remove_api(
                "tynd.config.ts",
                "export default {\n  apis: [\"dialog\", \"tray\"],\n};\n",
                "tray"
            ),
            "export default {\n  apis: [\"dialog\"],\n};\n"
        );
        assert_eq!(
            remove_api(
                "tynd.config.ts",
                "export default { apis: [\"tray\"] };\n",
                "tray"
            ),
            "export default { };\n"
        );
        assert_eq!(
            remove_api(
                "tynd.config.ts",
                "export default {\n  apis: [\n    \"dialog\",\n    \"tray\",\n  ],\n};\n",
                "dialog"
            ),
            "export default {\n  apis: [\n    \"tray\",\n  ],\n};\n"
        );
        assert_eq!(
            remove_api(
                "tynd.config.jsonc",
                "{\n  // apis\n  \"apis\": [\"dialog\", \"tray\"]\n}\n",
                "dialog"
            ),
            "{\n  // apis\n  \"apis\": [\"tray\"]\n}\n"
        );
    }

    #[test]
    fn removing_an_absent_api_leaves_the_source_unchanged() {
        for (file, source) in [
            ("tynd.config.ts", "export default { apis: [\"dialog\"] };\n"),
            ("tynd.config.ts", "export default {};\n"),
            ("tynd.config.jsonc", "{ \"apis\": [\"dialog\"] }\n"),
            ("tynd.config.json", "{ \"apis\": [\"dialog\"] }\n"),
            ("tynd.config.toml", "apis = [\"dialog\"]\n"),
            ("tynd.config.yaml", "apis:\n  - dialog\n"),
        ] {
            assert_eq!(remove_api(file, source, "tray"), source, "{file}");
        }
    }

    #[test]
    fn dynamic_api_entries_are_not_removed_blindly() {
        let error = mutate_config_source(
            Path::new("tynd.config.ts"),
            "export default { apis: [\"tray\", ...extra] };\n",
            ConfigMutation::RemoveApi("tray"),
        )
        .expect_err("dynamic entries are refused");
        assert!(
            format!("{error:#}").contains("dynamic entries"),
            "{error:#}"
        );
    }

    #[test]
    fn api_is_removed_from_data_configs() {
        let json = remove_api(
            "tynd.config.json",
            "{ \"apis\": [\"dialog\", \"tray\"] }\n",
            "tray",
        );
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(value["apis"], serde_json::json!(["dialog"]));

        let toml = remove_api(
            "tynd.config.toml",
            "name = \"app\"\napis = [\"dialog\", \"tray\"]\n",
            "dialog",
        );
        let value: toml::Value = toml::from_str(&toml).expect("valid TOML");
        assert_eq!(value["apis"], toml::Value::Array(vec!["tray".into()]));
        assert_eq!(value["name"].as_str(), Some("app"));

        let yaml = remove_api(
            "tynd.config.yml",
            "name: app\napis:\n  - dialog\n  - tray\n",
            "tray",
        );
        let value: serde_json::Value = serde_yaml::from_str(&yaml).expect("valid YAML");
        assert_eq!(value["apis"], serde_json::json!(["dialog"]));
        assert_eq!(value["name"], "app");
    }

    #[test]
    fn comment_after_the_last_trailing_comma_is_not_an_entry() {
        for (file, open, key, close) in [
            ("tynd.config.ts", "export default {\n  ", "apis", "\n};\n"),
            ("tynd.config.jsonc", "{\n  ", "\"apis\"", "\n}\n"),
        ] {
            let source =
                format!("{open}{key}: [\n    \"fs\",\n    \"tray\", // system tray\n  ],{close}");
            assert_eq!(
                remove_api(file, &source, "fs"),
                format!("{open}{key}: [\n    \"tray\", // system tray\n  ],{close}"),
                "{file}"
            );
            assert_eq!(
                add_api(file, &source, "dialog"),
                format!(
                    "{open}{key}: [\n    \"fs\",\n    \"tray\", // system tray\n    \"dialog\",\n  ],{close}"
                ),
                "{file}"
            );
        }
    }

    #[test]
    fn comments_stay_with_the_entries_that_survive_a_removal() {
        for (file, open, key, close) in [
            ("tynd.config.ts", "export default {\n  ", "apis", "\n};\n"),
            ("tynd.config.jsonc", "{\n  ", "\"apis\"", "\n}\n"),
        ] {
            let source = format!("{open}{key}: [\n    \"fs\", // files\n    \"tray\"\n  ],{close}");
            assert_eq!(
                remove_api(file, &source, "tray"),
                format!("{open}{key}: [\n    \"fs\", // files\n  ],{close}"),
                "{file}"
            );
            let source = format!(
                "{open}{key}: [\n    // core\n    \"fs\" // files\n    , \"tray\" /* t */\n  ],{close}"
            );
            assert_eq!(
                remove_api(file, &source, "tray"),
                format!("{open}{key}: [\n    // core\n    \"fs\", // files\n  ],{close}"),
                "{file}"
            );
        }
        assert_eq!(
            remove_api(
                "tynd.config.ts",
                "export default { apis: [\"fs\" /* a */, \"tray\", /* end */] };\n",
                "tray"
            ),
            "export default { apis: [\"fs\" /* a */ /* end */] };\n"
        );
    }

    #[test]
    fn plugin_removal_ignores_a_trailing_comment_entry() {
        let source = "export default {\n  plugins: [\n    \"@tynd/plugin-a\",\n    \"@tynd/plugin-b\", // b\n  ],\n};\n";
        assert_eq!(
            mutate_config_source(
                Path::new("tynd.config.ts"),
                source,
                ConfigMutation::RemovePlugin("@tynd/plugin-a"),
            )
            .expect("edit succeeds"),
            "export default {\n  plugins: [\n    \"@tynd/plugin-b\", // b\n  ],\n};\n"
        );
    }

    #[test]
    fn add_then_remove_api_round_trips() {
        for (file, source) in [
            ("tynd.config.ts", "export default { apis: [\"dialog\"] };\n"),
            ("tynd.config.jsonc", "{ \"apis\": [\"dialog\"] }\n"),
        ] {
            let added = add_api(file, source, "tray");
            assert_ne!(added, source, "{file}");
            assert_eq!(remove_api(file, &added, "tray"), source, "{file}");
        }
    }

    #[test]
    fn add_then_remove_plugin_restores_the_original_file() {
        for (file, source) in [
            (
                "tynd.config.ts",
                "export default defineConfig({\n  name: \"app\",\n});\n",
            ),
            ("tynd.config.ts", "export default { name: \"app\" };\n"),
            ("tynd.config.ts", "export default {};\n"),
            ("tynd.config.jsonc", "{\n  // app\n  \"name\": \"app\"\n}\n"),
            ("tynd.config.json", "{\n  \"name\": \"app\"\n}\n"),
            ("tynd.config.yaml", "name: app\n"),
            ("tynd.config.toml", "name = \"app\"\n"),
        ] {
            let added = mutate_config_source(
                Path::new(file),
                source,
                ConfigMutation::AddPlugin {
                    package: "@tynd/plugin-clipboard",
                    permissions: &[],
                },
            )
            .expect("add succeeds");
            assert!(added.contains("@tynd/plugin-clipboard"), "{file}: {added}");
            let removed = mutate_config_source(
                Path::new(file),
                &added,
                ConfigMutation::RemovePlugin("@tynd/plugin-clipboard"),
            )
            .expect("remove succeeds");
            assert_eq!(removed, source, "{file}");
        }
    }

    #[test]
    fn emptied_array_property_is_dropped_wherever_it_sits() {
        for (source, expected) in [
            (
                "export default { plugins: [\"p\"], name: \"app\" };\n",
                "export default { name: \"app\" };\n",
            ),
            (
                "export default { name: \"app\", plugins: [\"p\"] };\n",
                "export default { name: \"app\" };\n",
            ),
            (
                "export default {\n  name: \"app\",\n  plugins: [\n    \"p\",\n  ],\n};\n",
                "export default {\n  name: \"app\",\n};\n",
            ),
            (
                "export default {\r\n  plugins: [\"p\"],\r\n  name: \"app\",\r\n};\r\n",
                "export default {\r\n  name: \"app\",\r\n};\r\n",
            ),
        ] {
            let removed = mutate_config_source(
                Path::new("tynd.config.ts"),
                source,
                ConfigMutation::RemovePlugin("p"),
            )
            .expect("remove succeeds");
            assert_eq!(removed, expected);
        }
    }

    fn plugin_round_trip(file: &str, source: &str) -> String {
        let added = mutate_config_source(
            Path::new(file),
            source,
            ConfigMutation::AddPlugin {
                package: "@tynd/plugin-clipboard",
                permissions: &[],
            },
        )
        .expect("add succeeds");
        assert!(added.contains("@tynd/plugin-clipboard"), "{file}: {added}");
        let removed = mutate_config_source(
            Path::new(file),
            &added,
            ConfigMutation::RemovePlugin("@tynd/plugin-clipboard"),
        )
        .expect("remove succeeds");
        assert_eq!(removed, source, "{file}");
        added
    }

    #[test]
    fn data_config_edits_keep_key_order_indent_and_line_endings() {
        let json = "{\n    \"name\": \"app\",\n    \"version\": \"1.0.0\",\n    \"build\": {\n        \"out\": \"dist\",\n        \"entry\": \"main.ts\"\n    }\n}\n";
        let added = plugin_round_trip("tynd.config.json", json);
        assert!(
            added.starts_with("{\n    \"name\": \"app\",\n    \"version\": \"1.0.0\",\n    \"build\": {\n        \"out\": \"dist\",\n        \"entry\": \"main.ts\"\n    },\n    \"plugins\": ["),
            "{added}"
        );

        let crlf = json.replace('\n', "\r\n");
        let added = plugin_round_trip("tynd.config.json", &crlf);
        assert!(!added.replace("\r\n", "").contains('\n'), "{added:?}");

        let yaml = "name: app\nversion: 1.0.0\nbuild:\n  out: dist\n  entry: main.ts\n";
        let added = plugin_round_trip("tynd.config.yaml", yaml);
        assert!(
            added.starts_with("name: app\nversion: 1.0.0\nbuild:\n  out: dist\n  entry: main.ts\n"),
            "{added}"
        );
        plugin_round_trip("tynd.config.yaml", &yaml.replace('\n', "\r\n"));
    }

    #[test]
    fn toml_edits_keep_comments_and_layout() {
        let source = "# Tynd app\nname = \"app\"  # display name\nversion = \"1.0.0\"\napis = [\"fs\"] # used apis\n\n[build]\nout = \"dist\"\n";
        let added = plugin_round_trip("tynd.config.toml", source);
        assert!(
            added.starts_with("# Tynd app\nname = \"app\"  # display name\n"),
            "{added}"
        );
        let added = add_api("tynd.config.toml", source, "tray");
        assert!(
            added.contains("apis = [\"fs\", \"tray\"] # used apis\n"),
            "{added}"
        );
        assert_eq!(remove_api("tynd.config.toml", &added, "tray"), source);
        let keyed = set_key("tynd.config.toml", source);
        let value: toml::Value = toml::from_str(&keyed).expect("valid TOML");
        assert_eq!(value["updater"]["publicKeys"][0].as_str(), Some("KEY"));
        assert!(keyed.starts_with("# Tynd app\n"), "{keyed}");
    }

    #[test]
    fn toml_array_of_tables_round_trips_with_its_comments() {
        let source = "name = \"app\"\n\n# first plugin\n[[plugins]]\npkg = \"@tynd/plugin-a\" # a\nfeatures = [\"x\"]\n\n# second plugin\n[[plugins]]\npkg = \"@tynd/plugin-b\"\n";
        let added = plugin_round_trip("tynd.config.toml", source);
        assert!(added.starts_with(source), "{added}");
        assert!(
            added.contains("[[plugins]]\npkg = \"@tynd/plugin-clipboard\"\n"),
            "{added}"
        );
        let removed = mutate_config_source(
            Path::new("tynd.config.toml"),
            source,
            ConfigMutation::RemovePlugin("@tynd/plugin-a"),
        )
        .expect("remove succeeds");
        assert_eq!(
            removed,
            "name = \"app\"\n\n# second plugin\n[[plugins]]\npkg = \"@tynd/plugin-b\"\n"
        );
    }

    #[test]
    fn yaml_edits_keep_the_document_marker_and_trailing_newline() {
        let added = plugin_round_trip("tynd.config.yaml", "---\nname: app\n");
        assert!(added.starts_with("---\nname: app\n"), "{added}");
        let added = plugin_round_trip("tynd.config.yaml", "name: app");
        assert!(!added.ends_with('\n'), "{added:?}");
        let added = plugin_round_trip("tynd.config.yaml", "---\r\nname: app");
        assert!(added.starts_with("---\r\n"), "{added:?}");
        assert!(!added.ends_with('\n'), "{added:?}");
    }

    #[test]
    fn script_additions_use_the_line_endings_of_the_file() {
        for source in [
            "export default {\r\n  name: \"app\",\r\n};\r\n",
            "export default {\r\n  apis: [\r\n    \"fs\",\r\n  ],\r\n};\r\n",
            "export default {\r\n  apis: [\r\n    \"fs\"],\r\n};\r\n",
        ] {
            let added = add_api("tynd.config.ts", source, "tray");
            assert!(added.contains("tray"), "{added:?}");
            assert!(!added.replace("\r\n", "").contains('\n'), "{added:?}");
            let keyed = set_key("tynd.config.ts", source);
            assert!(!keyed.replace("\r\n", "").contains('\n'), "{keyed:?}");
        }
    }
}
