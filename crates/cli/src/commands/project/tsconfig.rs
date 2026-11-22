//! The tsconfig patcher of `tynd init`: adds the Tynd ambient types and the
//! backend to an existing TypeScript setup, keeping its formatting and
//! comments.

use std::{
    ffi::OsStr,
    fs,
    path::{Component, Path},
};

use anyhow::{ensure, Context, Result};
use serde_json::json;

use crate::{
    fsx::{atomic_write, write_json_file},
    source_edit::{
        code_shape, is_identifier_byte, is_top_level_position, last_code_byte, matching_delimiter,
    },
};

use super::shared::ensure_line_block;

pub(super) fn ensure_typescript_dx(root: &Path) -> Result<()> {
    let tsconfig = root.join("tsconfig.json");
    if !tsconfig.exists() {
        return write_json_file(
            &tsconfig,
            &json!({
                "compilerOptions": {
                    "target": "ES2022",
                    "module": "ESNext",
                    "moduleResolution": "bundler",
                    "strict": true,
                    "skipLibCheck": true,
                    "types": [BUN_TYPES, TYND_ENV_TYPES]
                },
                "include": ["frontend", "backend", "tynd.config.ts"]
            }),
        );
    }
    let source =
        fs::read_to_string(&tsconfig).with_context(|| format!("read {}", tsconfig.display()))?;
    let references = tsconfig_references(&source)?;
    // Solution-style configs (`files: []` + `references`, as in the vite and
    // Angular starters) do not forward their options to referenced projects,
    // so the Tynd frontend options go into the referenced app config.
    let frontend_config = references
        .as_deref()
        .and_then(frontend_reference)
        .map(|reference| root.join(reference))
        .filter(|path| path.is_file() && resolves_inside(root, path));
    if let Some(config) = &frontend_config {
        patch_file(config, patch_tsconfig_dx)?;
    } else {
        patch_file(&tsconfig, patch_tsconfig_dx)?;
        ensure_line_block(
            &root.join("global.d.ts"),
            &[&format!("/// <reference types=\"{TYND_ENV_TYPES}\" />")],
        )?;
    }
    if references.is_none() || !is_solution_root(&source)? {
        // A root with inputs of its own type-checks them: `tsc --noEmit -p .`
        // does not check referenced projects, and a root with inputs may only
        // reference `composite` projects that emit (TS6306, TS6310), so the
        // backend joins the root `include` instead, even next to `references`.
        return patch_file(&tsconfig, patch_flat_tsconfig_backend);
    }
    // Solution-style root (`files: []`, no `include`, `references`): the
    // backend becomes another referenced project, type-checked by `tsc -b`.
    let backend_config = root.join(BACKEND_TSCONFIG);
    if !backend_config.exists() {
        write_json_file(
            &backend_config,
            &json!({
                "compilerOptions": {
                    "tsBuildInfoFile": "./node_modules/.tmp/tsconfig.backend.tsbuildinfo",
                    "target": "ES2022",
                    "module": "ESNext",
                    "moduleResolution": "bundler",
                    "strict": true,
                    "skipLibCheck": true,
                    "noEmit": true,
                    "types": [BUN_TYPES]
                },
                "include": ["backend", "tynd.config.ts"]
            }),
        )?;
    }
    patch_file(&tsconfig, ensure_backend_reference)
}

fn patch_file(path: &Path, patch: fn(&str) -> Result<String>) -> Result<()> {
    let source = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let patched = patch(&source).with_context(|| format!("patch {}", path.display()))?;
    if patched != source {
        atomic_write(path, patched)?;
    }
    Ok(())
}

const TYND_ENV_TYPES: &str = "@tynd/core/env";

const BUN_TYPES: &str = "bun";

const BACKEND_TSCONFIG: &str = "tsconfig.backend.json";

/// Adds the Tynd ambient types to an existing tsconfig source, preserving its
/// formatting and comments.
fn patch_tsconfig_dx(source: &str) -> Result<String> {
    let mut next = source.to_owned();
    let (root_open, root_close) = find_root_object(&next)?;
    if find_top_level_property_value(&next, root_open, root_close, "compilerOptions")?.is_none() {
        next.insert_str(
            root_open + 1,
            "
  \"compilerOptions\": {
    \"types\": [\"@tynd/core/env\"]
  },",
        );
        return Ok(next);
    }
    ensure_tsconfig_types(&mut next)?;
    Ok(next)
}

/// Bounds `(open, close)` of the root `references` array, if any.
fn references_bounds(source: &str) -> Result<Option<(usize, usize)>> {
    let (root_open, root_close) = find_root_object(source)?;
    let Some(start) = find_top_level_property_value(source, root_open, root_close, "references")?
    else {
        return Ok(None);
    };
    let shape = code_shape(source, true);
    ensure!(
        shape.as_bytes().get(start) == Some(&b'['),
        "tsconfig references must be an array"
    );
    let close = matching_delimiter(&shape, start, b'[', b']')?;
    Ok(Some((start, close)))
}

/// `path` values of the root `references` array; `None` without references.
fn tsconfig_references(source: &str) -> Result<Option<Vec<String>>> {
    let Some((open, close)) = references_bounds(source)? else {
        return Ok(None);
    };
    let shape = code_shape(source, true);
    let pattern = regex::Regex::new(r#""path"\s*:\s*"([^"]*)""#)?;
    let paths = pattern
        .captures_iter(&source[open..close])
        .filter(|found| {
            // Skip matches inside comments, which the shape blanks out.
            found
                .get(0)
                .is_some_and(|whole| shape.as_bytes().get(open + whole.start() + 1) == Some(&b'p'))
        })
        .filter_map(|found| found.get(1).map(|value| value.as_str().to_owned()))
        .collect();
    Ok(Some(paths))
}

/// The referenced project that type-checks the frontend: `tsconfig.app.json`
/// when present, else the first reference that is not a tooling, test or
/// backend project.
fn frontend_reference(references: &[String]) -> Option<&str> {
    let file_name = |reference: &str| {
        Path::new(reference)
            .file_name()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase)
            .unwrap_or_default()
    };
    let mut references = references
        .iter()
        .filter(|reference| is_project_relative(reference));
    references
        .clone()
        .find(|reference| file_name(reference) == "tsconfig.app.json")
        .or_else(|| {
            references.find(|reference| {
                let name = file_name(reference);
                Path::new(reference.as_str())
                    .extension()
                    .is_some_and(|extension| extension == "json")
                    && !["node", "spec", "test", "backend"]
                        .iter()
                        .any(|excluded| name.contains(excluded))
            })
        })
        .map(String::as_str)
}

/// Whether `reference` is a plain project-relative path: not absolute and
/// without `..`, so it cannot point at a config outside the project.
fn is_project_relative(reference: &str) -> bool {
    !reference.is_empty()
        && Path::new(reference)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

/// Whether `path` resolves inside `root` once links are followed.
fn resolves_inside(root: &Path, path: &Path) -> bool {
    match (root.canonicalize(), path.canonicalize()) {
        (Ok(root), Ok(path)) => path.starts_with(root),
        _ => false,
    }
}

/// Bounds `(open, close)` of the root `include` array, if it is one.
fn include_bounds(source: &str) -> Result<Option<(usize, usize)>> {
    let (root_open, root_close) = find_root_object(source)?;
    let Some(start) = find_top_level_property_value(source, root_open, root_close, "include")?
    else {
        return Ok(None);
    };
    let shape = code_shape(source, true);
    if shape.as_bytes().get(start) != Some(&b'[') {
        return Ok(None);
    }
    let close = matching_delimiter(&shape, start, b'[', b']')?;
    Ok(Some((start, close)))
}

/// Whether the root project type-checks `backend/`: true without `include`
/// and `files` (everything is included) or when `include` names it.
fn includes_backend(source: &str) -> Result<bool> {
    if let Some((open, close)) = include_bounds(source)? {
        return Ok(code_shape(source, true)[open..close].contains("backend"));
    }
    let (root_open, root_close) = find_root_object(source)?;
    let has = |property| find_top_level_property_value(source, root_open, root_close, property);
    // A non-array `include` is left to tsc to reject.
    Ok(has("include")?.is_some() || has("files")?.is_none())
}

/// Whether the root has no inputs of its own: `files` and `include` are each
/// absent or `[]`, at least one present, as in solution-style configs that
/// only aggregate `references`.
fn is_solution_root(source: &str) -> Result<bool> {
    let (root_open, root_close) = find_root_object(source)?;
    let shape = code_shape(source, true);
    // `Some(true)` only for a property whose value is an array with no entries.
    let empty_array = |property| -> Result<Option<bool>> {
        let Some(value) = find_top_level_property_value(source, root_open, root_close, property)?
        else {
            return Ok(None);
        };
        if shape.as_bytes().get(value) != Some(&b'[') {
            return Ok(Some(false));
        }
        let close = matching_delimiter(&shape, value, b'[', b']')?;
        Ok(Some(last_code_byte(source, value, close).is_none()))
    };
    Ok(matches!(
        (empty_array("include")?, empty_array("files")?),
        (Some(true), None | Some(true)) | (None, Some(true))
    ))
}

/// Makes a root tsconfig with inputs of its own type-check the backend: adds
/// `backend` and `tynd.config.ts` to its `include` and the Bun types the
/// backend needs to a restricted `types` list.
fn patch_flat_tsconfig_backend(source: &str) -> Result<String> {
    let mut next = source.to_owned();
    if !includes_backend(&next)? {
        if let Some((open, close)) = include_bounds(&next)? {
            let existing = &next[open + 1..close];
            let mut items = vec!["\"backend\""];
            if !array_contains_string(existing, "tynd.config.ts") {
                items.push("\"tynd.config.ts\"");
            }
            // Appended inline after the last entry, `["src", "backend", ...]`,
            // and before any trailing comment so the comment cannot swallow it.
            let (at, separator) = match last_code_byte(&next, open, close) {
                None => (open + 1, ""),
                Some(last) if next.as_bytes()[last] == b',' => (last + 1, " "),
                Some(last) => (last + 1, ", "),
            };
            next.insert_str(at, &format!("{separator}{}", items.join(", ")));
        } else {
            // `files` without `include`: only the listed files are checked.
            let (root_open, _) = find_root_object(&next)?;
            next.insert_str(
                root_open + 1,
                "\n  \"include\": [\"backend\", \"tynd.config.ts\"],",
            );
        }
    }
    let (root_open, root_close) = find_root_object(&next)?;
    if find_top_level_property_value(&next, root_open, root_close, "compilerOptions")?.is_none() {
        return Ok(next);
    }
    let (compiler_start, compiler_close) = compiler_options_bounds(&next)?;
    if let Some(types_start) =
        find_top_level_property_value(&next, compiler_start, compiler_close, "types")?
    {
        let shape = code_shape(&next, true);
        if shape.as_bytes().get(types_start) == Some(&b'[') {
            let types_close = matching_delimiter(&shape, types_start, b'[', b']')?;
            if !array_contains_string(&next[types_start + 1..types_close], BUN_TYPES) {
                insert_array_string(&mut next, types_start, types_close, BUN_TYPES);
            }
        }
    }
    Ok(next)
}

/// Adds the backend project to the root `references`, creating the array
/// when the root has none.
fn ensure_backend_reference(source: &str) -> Result<String> {
    let entry = format!("{{ \"path\": \"./{BACKEND_TSCONFIG}\" }}");
    let mut next = source.to_owned();
    if let Some((open, close)) = references_bounds(source)? {
        if source[open..close].contains(BACKEND_TSCONFIG) {
            return Ok(next);
        }
        // Inserted after the last entry, before any trailing comment.
        let (at, separator) = match last_code_byte(source, open, close) {
            None => (open + 1, ""),
            Some(last) if source.as_bytes()[last] == b',' => (last + 1, ""),
            Some(last) => (last + 1, ","),
        };
        next.insert_str(at, &format!("{separator}\n    {entry}"));
        return Ok(next);
    }
    let (root_open, _) = find_root_object(source)?;
    next.insert_str(root_open + 1, &format!("\n  \"references\": [{entry}],"));
    Ok(next)
}

/// Bounds `(open, close)` of the root `compilerOptions` object.
fn compiler_options_bounds(source: &str) -> Result<(usize, usize)> {
    let (root_open, root_close) = find_root_object(source)?;
    let compiler_start =
        find_top_level_property_value(source, root_open, root_close, "compilerOptions")?
            .context("compilerOptions property disappeared while patching tsconfig")?;
    let shape = code_shape(source, true);
    ensure!(
        shape.as_bytes().get(compiler_start) == Some(&b'{'),
        "tsconfig compilerOptions must be an object"
    );
    let compiler_close = matching_delimiter(&shape, compiler_start, b'{', b'}')?;
    Ok((compiler_start, compiler_close))
}

fn ensure_tsconfig_types(next: &mut String) -> Result<()> {
    let (compiler_start, compiler_close) = compiler_options_bounds(next)?;
    let Some(types_start) =
        find_top_level_property_value(next, compiler_start, compiler_close, "types")?
    else {
        next.insert_str(compiler_start + 1, "\n    \"types\": [\"@tynd/core/env\"],");
        return Ok(());
    };
    let shape = code_shape(next, true);
    ensure!(
        shape.as_bytes().get(types_start) == Some(&b'['),
        "tsconfig compilerOptions.types must be an array"
    );
    let types_close = matching_delimiter(&shape, types_start, b'[', b']')?;
    if !array_contains_string(&next[types_start + 1..types_close], TYND_ENV_TYPES) {
        insert_array_string(next, types_start, types_close, TYND_ENV_TYPES);
    }
    Ok(())
}

fn find_root_object(source: &str) -> Result<(usize, usize)> {
    let shape = code_shape(source, true);
    let open = shape
        .bytes()
        .position(|byte| byte == b'{')
        .context("tsconfig root must be an object")?;
    let close = matching_delimiter(&shape, open, b'{', b'}')?;
    Ok((open, close))
}

fn find_top_level_property_value(
    source: &str,
    object_open: usize,
    object_close: usize,
    property: &str,
) -> Result<Option<usize>> {
    let shape = code_shape(source, true);
    ensure!(
        object_open < object_close && object_close <= shape.len(),
        "invalid object bounds"
    );
    let pattern = regex::Regex::new(&format!(r"{}\s*:", regex::escape(property)))?;
    let scope = &shape[object_open + 1..object_close];
    for found in pattern.find_iter(scope) {
        let absolute = object_open + 1 + found.start();
        if !is_top_level_position(&shape, object_open + 1, absolute) {
            continue;
        }
        if property
            .as_bytes()
            .first()
            .is_some_and(|byte| is_identifier_byte(*byte))
            && absolute > 0
            && shape
                .as_bytes()
                .get(absolute - 1)
                .is_some_and(|byte| is_identifier_byte(*byte))
        {
            continue;
        }
        let mut value = object_open + 1 + found.end();
        while shape
            .as_bytes()
            .get(value)
            .is_some_and(u8::is_ascii_whitespace)
        {
            value += 1;
        }
        ensure!(value < object_close, "{property} has no value");
        return Ok(Some(value));
    }
    Ok(None)
}

fn array_contains_string(content: &str, value: &str) -> bool {
    content.contains(&serde_json::to_string(value).unwrap_or_default())
        || content.contains(&format!("'{value}'"))
}

fn insert_array_string(source: &mut String, open: usize, close: usize, value: &str) {
    source.insert_str(
        close,
        &format!(
            "\n      {},\n    ",
            serde_json::to_string(value).unwrap_or_default()
        ),
    );
    // The separator goes right after the last entry, before any trailing
    // comment, so it is never commented out.
    if let Some(last) = last_code_byte(source, open, close) {
        if source.as_bytes()[last] != b',' {
            source.insert(last + 1, ',');
        }
    }
}

#[cfg(test)]
mod tests;
