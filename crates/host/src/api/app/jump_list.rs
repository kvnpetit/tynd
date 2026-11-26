//! Windows jump list (`app.setJumpList` / `app.clearJumpList`): user tasks
//! and custom categories of links that relaunch the app executable with
//! arguments. With the single-instance API those arguments reach the running
//! instance through `singleInstance.onSecondLaunch`; without it a new
//! process starts with them. macOS (use `app.dock.setMenu`) and Linux have
//! no jump list.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

/// Most user tasks accepted.
const MAX_TASKS: usize = 32;
/// Most custom categories accepted.
const MAX_CATEGORIES: usize = 16;
/// Most items accepted in one category.
const MAX_CATEGORY_ITEMS: usize = 32;
/// Longest title or category name, in UTF-16 units (`MAX_PATH`).
const MAX_TITLE: usize = 260;
/// Longest description / argument string, in UTF-16 units (`INFOTIPSIZE`).
const MAX_TEXT: usize = 1024;
/// Most arguments of one item.
const MAX_ARGS: usize = 32;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawItem {
    title: String,
    #[serde(default)]
    args: Vec<String>,
    description: Option<String>,
    icon: Option<String>,
    icon_index: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCategory {
    name: String,
    items: Vec<RawItem>,
}

/// One jump list link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Item {
    pub(super) title: String,
    /// Command line passed to the executable (already quoted).
    pub(super) arguments: String,
    pub(super) description: Option<String>,
    /// Icon file (`.ico`, `.exe`, `.dll`) and index; `None` = app icon.
    pub(super) icon: Option<(PathBuf, i32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Category {
    pub(super) name: String,
    pub(super) items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct JumpList {
    pub(super) tasks: Vec<Item>,
    pub(super) categories: Vec<Category>,
    /// Keep the OS "Recent" category (`app.addRecentDocument`).
    pub(super) show_recent: bool,
}

fn err(message: impl std::fmt::Display) -> String {
    message.to_string()
}

fn check_text(field: &str, text: &str, max: usize, required: bool) -> Result<(), String> {
    if required && text.trim().is_empty() {
        return Err(err(format!("{field} must not be empty")));
    }
    if text.contains('\0') {
        return Err(err(format!("{field} must not contain NUL characters")));
    }
    if text.encode_utf16().count() > max {
        return Err(err(format!("{field} is longer than {max} characters")));
    }
    Ok(())
}

/// Parse and validate the renderer's jump list. `authorize_icon` resolves an
/// icon path through the FS scope.
pub(super) fn parse(
    tasks: Option<Value>,
    categories: Option<Value>,
    show_recent: Option<bool>,
    authorize_icon: &dyn Fn(&str) -> Result<PathBuf, String>,
) -> Result<JumpList, String> {
    let tasks: Vec<RawItem> = match tasks {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => serde_json::from_value(value).map_err(|e| err(format!("tasks: {e}")))?,
    };
    let categories: Vec<RawCategory> = match categories {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => {
            serde_json::from_value(value).map_err(|e| err(format!("categories: {e}")))?
        },
    };
    if tasks.len() > MAX_TASKS {
        return Err(err(format!("at most {MAX_TASKS} tasks are allowed")));
    }
    if categories.len() > MAX_CATEGORIES {
        return Err(err(format!(
            "at most {MAX_CATEGORIES} categories are allowed"
        )));
    }
    let tasks = tasks
        .into_iter()
        .enumerate()
        .map(|(index, raw)| item(&format!("tasks[{index}]"), raw, authorize_icon))
        .collect::<Result<Vec<_>, _>>()?;
    let mut parsed = Vec::with_capacity(categories.len());
    for (index, raw) in categories.into_iter().enumerate() {
        let field = format!("categories[{index}]");
        check_text(&format!("{field}.name"), &raw.name, MAX_TITLE, true)?;
        if raw.items.is_empty() {
            return Err(err(format!("{field}.items must not be empty")));
        }
        if raw.items.len() > MAX_CATEGORY_ITEMS {
            return Err(err(format!(
                "{field}.items holds at most {MAX_CATEGORY_ITEMS} items"
            )));
        }
        if parsed.iter().any(|c: &Category| c.name == raw.name) {
            return Err(err(format!("duplicate category name '{}'", raw.name)));
        }
        let items = raw
            .items
            .into_iter()
            .enumerate()
            .map(|(i, item_raw)| item(&format!("{field}.items[{i}]"), item_raw, authorize_icon))
            .collect::<Result<Vec<_>, _>>()?;
        parsed.push(Category {
            name: raw.name,
            items,
        });
    }
    Ok(JumpList {
        tasks,
        categories: parsed,
        show_recent: show_recent.unwrap_or(true),
    })
}

fn item(
    field: &str,
    raw: RawItem,
    authorize_icon: &dyn Fn(&str) -> Result<PathBuf, String>,
) -> Result<Item, String> {
    check_text(&format!("{field}.title"), &raw.title, MAX_TITLE, true)?;
    if let Some(description) = &raw.description {
        check_text(
            &format!("{field}.description"),
            description,
            MAX_TEXT,
            false,
        )?;
    }
    if raw.args.len() > MAX_ARGS {
        return Err(err(format!(
            "{field}.args holds at most {MAX_ARGS} arguments"
        )));
    }
    for arg in &raw.args {
        check_text(&format!("{field}.args"), arg, MAX_TEXT, false)?;
    }
    let arguments = command_line(&raw.args);
    check_text(&format!("{field}.args"), &arguments, MAX_TEXT, false)?;
    let index = raw.icon_index.unwrap_or(0);
    if index < 0 {
        return Err(err(format!("{field}.iconIndex must be 0 or more")));
    }
    let icon = match raw.icon {
        Some(path) => Some((
            authorize_icon(&path).map_err(|e| err(format!("{field}.icon: {e}")))?,
            index,
        )),
        None => None,
    };
    Ok(Item {
        title: raw.title,
        arguments,
        description: raw.description.filter(|d| !d.is_empty()),
        icon,
    })
}

/// Join `args` into one Windows command line, quoted so `CommandLineToArgvW`
/// (and the Rust / C runtimes) split it back into the same arguments.
pub(super) fn command_line(args: &[String]) -> String {
    let mut out = String::new();
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        quote_arg(arg, &mut out);
    }
    out
}

fn quote_arg(arg: &str, out: &mut String) {
    let needs_quotes = arg.is_empty() || arg.contains([' ', '\t', '\n', '\x0b', '"']);
    if !needs_quotes {
        out.push_str(arg);
        return;
    }
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // Backslashes before a quote are doubled, then the quote is
                // escaped.
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            },
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            },
        }
    }
    // Trailing backslashes precede the closing quote: double them.
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
}

#[cfg(target_os = "windows")]
pub(super) use win::{apply, clear};

#[cfg(not(target_os = "windows"))]
pub(super) fn apply(_list: &JumpList) -> Result<(), String> {
    Err(unsupported())
}

#[cfg(not(target_os = "windows"))]
pub(super) fn clear() -> Result<(), String> {
    Err(unsupported())
}

#[cfg(not(target_os = "windows"))]
fn unsupported() -> String {
    if cfg!(target_os = "macos") {
        "the jump list is Windows only; not supported on macos (use app.dock.setMenu)".to_string()
    } else {
        format!(
            "the jump list is Windows only; not supported on {}",
            std::env::consts::OS
        )
    }
}

#[cfg(target_os = "windows")]
mod win {
    use std::{collections::HashSet, path::Path};

    use windows::{
        core::{Interface, HSTRING, PCWSTR},
        Win32::{
            Foundation::RPC_E_CHANGED_MODE,
            Storage::EnhancedStorage::PKEY_Title,
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoUninitialize,
                    StructuredStorage::PROPVARIANT, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
                },
                Variant::VT_LPWSTR,
            },
            UI::Shell::{
                Common::{IObjectArray, IObjectCollection},
                DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW,
                PropertiesSystem::IPropertyStore,
                SHStrDupW, ShellLink, KDC_RECENT,
            },
        },
    };

    use super::{Item, JumpList};

    /// COM apartment for this call; uninitialized on drop when this call
    /// initialized it.
    struct Apartment(bool);

    impl Apartment {
        fn enter() -> Self {
            // SAFETY: no reserved pointer; balanced by `Drop`.
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            // S_OK / S_FALSE need a matching uninit; RPC_E_CHANGED_MODE means
            // the thread already has a (multi-threaded) apartment.
            Self(hr.is_ok() && hr != RPC_E_CHANGED_MODE)
        }
    }

    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: matches the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
        }
    }

    fn com_err(step: &str, error: &windows::core::Error) -> String {
        format!("{step}: {error}")
    }

    pub(in crate::api::app) fn apply(list: &JumpList) -> Result<(), String> {
        let _apartment = Apartment::enter();
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        // SAFETY: documented in-proc shell class; COM is initialised above.
        let destinations: ICustomDestinationList =
            unsafe { CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| com_err("create the destination list", &e))?;
        let mut slots = 0u32;
        // SAFETY: `slots` is writable; the list is aborted on any error below.
        let removed: IObjectArray = unsafe { destinations.BeginList(&raw mut slots) }
            .map_err(|e| com_err("BeginList", &e))?;
        let result = build(&destinations, list, &exe, &removed_arguments(&removed));
        // SAFETY: a list began above is committed or aborted exactly once.
        unsafe {
            match result {
                Ok(()) => destinations
                    .CommitList()
                    .map_err(|e| com_err("CommitList", &e)),
                Err(error) => {
                    let _ = destinations.AbortList();
                    Err(error)
                },
            }
        }
    }

    pub(in crate::api::app) fn clear() -> Result<(), String> {
        let _apartment = Apartment::enter();
        // SAFETY: documented in-proc shell class; a null app id targets this
        // process's AppUserModelID.
        unsafe {
            let destinations: ICustomDestinationList =
                CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| com_err("create the destination list", &e))?;
            destinations
                .DeleteList(PCWSTR::null())
                .map_err(|e| com_err("DeleteList", &e))
        }
    }

    fn build(
        destinations: &ICustomDestinationList,
        list: &JumpList,
        exe: &Path,
        removed: &HashSet<String>,
    ) -> Result<(), String> {
        for category in &list.categories {
            // Windows refuses a category holding a link the user removed
            // from the jump list; such items are skipped.
            let items: Vec<&Item> = category
                .items
                .iter()
                .filter(|item| !removed.contains(&item.arguments))
                .collect();
            if items.is_empty() {
                continue;
            }
            let collection = links(&items, exe)?;
            // SAFETY: valid collection of IShellLinkW built above.
            unsafe {
                destinations
                    .AppendCategory(&HSTRING::from(category.name.as_str()), &collection)
                    .map_err(|e| com_err(&format!("category '{}'", category.name), &e))?;
            }
        }
        if list.show_recent {
            // SAFETY: known category constant. Fails when the app has no
            // registered file type; the list is still valid without it.
            let _ = unsafe { destinations.AppendKnownCategory(KDC_RECENT) };
        }
        if !list.tasks.is_empty() {
            let tasks: Vec<&Item> = list.tasks.iter().collect();
            let collection = links(&tasks, exe)?;
            // SAFETY: valid collection of IShellLinkW built above.
            unsafe { destinations.AddUserTasks(&collection) }
                .map_err(|e| com_err("AddUserTasks", &e))?;
        }
        Ok(())
    }

    /// Arguments of the links the user removed from the jump list.
    fn removed_arguments(removed: &IObjectArray) -> HashSet<String> {
        let mut out = HashSet::new();
        // SAFETY: indices stay below GetCount; GetArguments writes at most
        // the buffer length and NUL-terminates.
        unsafe {
            let count = removed.GetCount().unwrap_or(0);
            for index in 0..count {
                let Ok(link) = removed.GetAt::<IShellLinkW>(index) else {
                    continue;
                };
                let mut buffer = [0u16; super::MAX_TEXT + 1];
                if link.GetArguments(&mut buffer).is_ok() {
                    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
                    out.insert(String::from_utf16_lossy(&buffer[..len]));
                }
            }
        }
        out
    }

    fn links(items: &[&Item], exe: &Path) -> Result<IObjectArray, String> {
        // SAFETY: documented in-proc shell class.
        let collection: IObjectCollection =
            unsafe { CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| com_err("create an object collection", &e))?;
        for item in items {
            let link = link(item, exe)?;
            // SAFETY: `link` is a valid COM object, AddRef'd by the collection.
            unsafe { collection.AddObject(&link) }.map_err(|e| com_err("AddObject", &e))?;
        }
        collection
            .cast::<IObjectArray>()
            .map_err(|e| com_err("IObjectArray", &e))
    }

    fn link(item: &Item, exe: &Path) -> Result<IShellLinkW, String> {
        let step = |what: &str| format!("link '{}': {what}", item.title);
        // SAFETY: documented in-proc shell class; every string argument is
        // an HSTRING that outlives its call.
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| com_err(&step("create"), &e))?;
            link.SetPath(&HSTRING::from(exe))
                .map_err(|e| com_err(&step("SetPath"), &e))?;
            link.SetArguments(&HSTRING::from(item.arguments.as_str()))
                .map_err(|e| com_err(&step("SetArguments"), &e))?;
            if let Some(description) = &item.description {
                link.SetDescription(&HSTRING::from(description.as_str()))
                    .map_err(|e| com_err(&step("SetDescription"), &e))?;
            }
            let (icon, index) = item
                .icon
                .as_ref()
                .map_or((exe, 0), |(path, index)| (path.as_path(), *index));
            link.SetIconLocation(&HSTRING::from(icon), index)
                .map_err(|e| com_err(&step("SetIconLocation"), &e))?;
            let store: IPropertyStore = link
                .cast()
                .map_err(|e| com_err(&step("IPropertyStore"), &e))?;
            let title = string_variant(&item.title).map_err(|e| com_err(&step("title"), &e))?;
            let key = PKEY_Title;
            store
                .SetValue(&raw const key, &raw const title)
                .map_err(|e| com_err(&step("SetValue(Title)"), &e))?;
            store.Commit().map_err(|e| com_err(&step("Commit"), &e))?;
            Ok(link)
        }
    }

    /// `VT_LPWSTR` PROPVARIANT (what `InitPropVariantFromString` builds):
    /// the CoTaskMem string is freed by `PROPVARIANT`'s `Drop`
    /// (`PropVariantClear`).
    fn string_variant(text: &str) -> windows::core::Result<PROPVARIANT> {
        // SAFETY: SHStrDupW copies the HSTRING into CoTaskMem memory, owned
        // by the variant from here on.
        unsafe {
            let copy = SHStrDupW(&HSTRING::from(text))?;
            let mut variant = PROPVARIANT::default();
            let inner = &mut *variant.Anonymous.Anonymous;
            inner.vt = VT_LPWSTR;
            inner.Anonymous.pwszVal = copy;
            Ok(variant)
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn allow_icon(path: &str) -> Result<PathBuf, String> {
        Ok(PathBuf::from(path))
    }

    fn deny_icon(_path: &str) -> Result<PathBuf, String> {
        Err("outside the FS scope".into())
    }

    #[test]
    fn command_line_quoting_round_trips_windows_rules() {
        let args = |list: &[&str]| list.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        assert_eq!(command_line(&args(&["--new-window"])), "--new-window");
        assert_eq!(command_line(&args(&["a b", "c"])), r#""a b" c"#);
        assert_eq!(command_line(&args(&[""])), r#""""#);
        assert_eq!(command_line(&args(&[r#"x"y"#])), r#""x\"y""#);
        assert_eq!(command_line(&args(&[r"C:\my dir\"])), r#""C:\my dir\\""#);
        assert_eq!(command_line(&args(&[r#"a\"b c"#])), r#""a\\\"b c""#);
        assert_eq!(command_line(&args(&[r"C:\plain\path"])), r"C:\plain\path");
    }

    #[test]
    fn parses_tasks_and_categories() {
        let list = parse(
            Some(json!([{ "title": "New window", "args": ["--new-window"], "description": "Open" }])),
            Some(json!([{ "name": "Projects", "items": [{ "title": "Acme", "args": ["--open", "C:\\work\\acme app"], "icon": "C:\\icons\\a.ico", "iconIndex": 2 }] }])),
            None,
            &allow_icon,
        )
        .unwrap();
        assert!(list.show_recent);
        assert_eq!(list.tasks[0].arguments, "--new-window");
        assert_eq!(list.tasks[0].description.as_deref(), Some("Open"));
        assert_eq!(list.tasks[0].icon, None);
        let item = &list.categories[0].items[0];
        assert_eq!(item.arguments, r#"--open "C:\work\acme app""#);
        assert_eq!(item.icon, Some((PathBuf::from("C:\\icons\\a.ico"), 2)));

        let empty = parse(None, None, Some(false), &allow_icon).unwrap();
        assert!(empty.tasks.is_empty() && empty.categories.is_empty() && !empty.show_recent);
    }

    #[test]
    fn rejects_invalid_lists() {
        let long = "x".repeat(MAX_TITLE + 1);
        let too_many: Vec<Value> = (0..=MAX_TASKS)
            .map(|i| json!({ "title": format!("t{i}") }))
            .collect();
        let cases = [
            (json!([{ "title": "" }]), Value::Null, "must not be empty"),
            (json!([{ "title": long }]), Value::Null, "longer than"),
            (json!([{ "title": "a\u{0}b" }]), Value::Null, "NUL"),
            (
                json!([{ "title": "a", "extra": 1 }]),
                Value::Null,
                "unknown field",
            ),
            (
                json!([{ "title": "a", "iconIndex": -1 }]),
                Value::Null,
                "iconIndex",
            ),
            (Value::Array(too_many), Value::Null, "at most"),
            (
                Value::Null,
                json!([{ "name": "Empty", "items": [] }]),
                "must not be empty",
            ),
            (
                Value::Null,
                json!([
                    { "name": "A", "items": [{ "title": "x" }] },
                    { "name": "A", "items": [{ "title": "y" }] }
                ]),
                "duplicate",
            ),
            (json!({ "title": "not an array" }), Value::Null, "tasks"),
        ];
        for (tasks, categories, expected) in cases {
            let error = parse(Some(tasks), Some(categories), None, &allow_icon).unwrap_err();
            assert!(
                error.contains(expected),
                "{error} should mention {expected}"
            );
        }
    }

    #[test]
    fn icons_go_through_the_fs_scope() {
        let error = parse(
            Some(json!([{ "title": "a", "icon": "C:\\secret.ico" }])),
            None,
            None,
            &deny_icon,
        )
        .unwrap_err();
        assert!(
            error.contains("tasks[0].icon: outside the FS scope"),
            "{error}"
        );
    }

    #[test]
    fn overlong_command_lines_are_rejected() {
        let args: Vec<String> = (0..MAX_ARGS).map(|_| "y".repeat(40)).collect();
        let error = parse(
            Some(json!([{ "title": "a", "args": args }])),
            None,
            None,
            &allow_icon,
        )
        .unwrap_err();
        assert!(error.contains("longer than"), "{error}");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn other_platforms_reject() {
        assert!(clear().unwrap_err().contains("not supported on"));
    }
}
