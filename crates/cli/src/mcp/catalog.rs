use anyhow::Result;
use serde_json::{json, Value};

/// The generated catalog (`packages/mcp/catalog/`), compiled into the binary
/// by `build.rs`: `FRAMEWORK_DOCS` and `PLUGIN_SCHEMAS`, `(file name,
/// contents)` sorted by file name.
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/mcp_catalog.rs"));
}

type Assets = &'static [(&'static str, &'static str)];

#[derive(Clone, Debug)]
pub(super) struct FrameworkDoc {
    pub(super) slug: String,
    pub(super) title: String,
    pub(super) body: String,
}

#[derive(Clone, Debug)]
pub(super) struct PluginSchema {
    pub(super) name: String,
    pub(super) summary: String,
    pub(super) source: String,
}

#[derive(Clone, Debug)]
pub(super) struct Catalog {
    docs: Assets,
    schemas: Assets,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct KnownError {
    pub(crate) code: &'static str,
    pub(crate) title: &'static str,
    pub(crate) symptom: &'static str,
    pub(crate) cause: &'static str,
    pub(crate) fix: &'static str,
    pub(crate) doc: Option<&'static str>,
}

impl Catalog {
    /// The catalog compiled into this binary.
    pub(super) fn embedded() -> Self {
        Self {
            docs: embedded::FRAMEWORK_DOCS,
            schemas: embedded::PLUGIN_SCHEMAS,
        }
    }

    pub(super) fn list_docs(&self) -> Result<Vec<FrameworkDoc>> {
        Ok(named_assets(self.docs, ".md", |name, body| {
            let slug = name.trim_end_matches(".md").to_owned();
            FrameworkDoc {
                title: extract_markdown_title(&body).unwrap_or_else(|| slug.clone()),
                slug,
                body,
            }
        }))
    }

    pub(super) fn find_doc(&self, slug: &str) -> Result<Option<FrameworkDoc>> {
        if !safe_catalog_name(slug) {
            return Ok(None);
        }
        Ok(self
            .list_docs()?
            .into_iter()
            .find(|document| document.slug == slug))
    }

    pub(super) fn search_docs(&self, query: &str, limit: usize) -> Result<Vec<FrameworkDoc>> {
        let query = query.to_lowercase();
        let mut documents = self
            .list_docs()?
            .into_iter()
            .filter_map(|document| {
                let title_hits = usize::from(document.title.to_lowercase().contains(&query)) * 5;
                let body_hits = if query.is_empty() {
                    0
                } else {
                    document.body.to_lowercase().matches(&query).count()
                };
                let score = title_hits + body_hits;
                (score > 0).then_some((score, document))
            })
            .collect::<Vec<_>>();
        documents.sort_by_key(|left| std::cmp::Reverse(left.0));
        Ok(documents
            .into_iter()
            .take(limit)
            .map(|(_, document)| document)
            .collect())
    }

    pub(super) fn list_plugin_schemas(&self) -> Result<Vec<PluginSchema>> {
        Ok(named_assets(self.schemas, ".gen.ts", |name, source| {
            let plugin_name = name.trim_end_matches(".gen.ts").to_owned();
            PluginSchema {
                summary: extract_typescript_summary(&source)
                    .unwrap_or_else(|| format!("{plugin_name} plugin schema")),
                name: plugin_name,
                source,
            }
        }))
    }

    pub(super) fn find_plugin_schema(&self, name: &str) -> Result<Option<PluginSchema>> {
        if !safe_catalog_name(name) {
            return Ok(None);
        }
        Ok(self
            .list_plugin_schemas()?
            .into_iter()
            .find(|schema| schema.name == name))
    }
}

fn named_assets<T>(assets: Assets, suffix: &str, map: impl Fn(&str, String) -> T) -> Vec<T> {
    assets
        .iter()
        .filter(|(name, _)| name.strip_suffix(suffix).is_some_and(safe_catalog_name))
        .map(|(name, body)| map(name, (*body).to_owned()))
        .collect()
}

fn safe_catalog_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && !value.contains("..")
}

fn extract_markdown_title(markdown: &str) -> Option<String> {
    markdown
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
}

fn extract_typescript_summary(source: &str) -> Option<String> {
    source
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !matches!(*line, "/**" | "/*" | "*/")
                && !line.starts_with('*')
                && !line.starts_with("//")
        })
        .map(|line| {
            line.trim_matches(|character| matches!(character, '/' | '*' | ' '))
                .to_owned()
        })
        .filter(|summary| !summary.is_empty())
}

pub(crate) fn known_errors() -> &'static [KnownError] {
    &KNOWN_ERRORS
}

pub(crate) fn find_known_error(code: &str) -> Option<&'static KnownError> {
    KNOWN_ERRORS.iter().find(|known| known.code == code)
}

pub(crate) fn known_error_json(error: &KnownError) -> Value {
    json!({
        "code": error.code,
        "title": error.title,
        "symptom": error.symptom,
        "cause": error.cause,
        "fix": error.fix,
        "doc": error.doc,
    })
}

static KNOWN_ERRORS: [KnownError; 16] = [
    KnownError {
        code: "PERMISSION_DENIED",
        title: "Capability denied",
        symptom: "TyndIpcError code=PERMISSION_DENIED on a backend RPC.",
        cause: "The capability for the called API is not granted in the active capability set.",
        fix: "Add the exact command to a scoped capability in app.start({ capabilities: [...] }), then restart dev.",
        doc: Some("tynd://docs/security"),
    },
    KnownError {
        code: "UNKNOWN_FN",
        title: "Backend function not registered",
        symptom: "TyndIpcError code=UNKNOWN_FN with the missing fn name.",
        cause: "The backend module / plugin did not register the called function.",
        fix: "Regenerate types, restart dev, and confirm the function is exported.",
        doc: Some("tynd://docs/backend-rpc"),
    },
    KnownError {
        code: "INVALID_ARGS",
        title: "IPC arg validation failed",
        symptom: "TyndIpcError code=INVALID_ARGS with validation path mismatch.",
        cause: "Frontend payload shape does not match the backend validator.",
        fix: "Inspect ipc.failed, then fix the call site or validator.",
        doc: Some("tynd://docs/ipc"),
    },
    KnownError {
        code: "HANDLER_THREW",
        title: "Backend handler crashed",
        symptom: "TyndIpcError code=HANDLER_THREW and host stack trace.",
        cause: "Unhandled exception in the backend handler.",
        fix: "Inspect app.logs.tail, handle the error, and rerun.",
        doc: Some("tynd://docs/observability"),
    },
    KnownError {
        code: "TIMEOUT",
        title: "RPC timed out",
        symptom: "TyndIpcError code=TIMEOUT.",
        cause: "Backend handler is too slow or stuck.",
        fix: "Trace or paginate work, or adjust timeout; inspect runtime.snapshot.",
        doc: Some("tynd://docs/observability"),
    },
    KnownError {
        code: "BACKEND_DEAD",
        title: "Host process died",
        symptom: "In-flight RPCs reject with BACKEND_DEAD.",
        cause: "Backend runtime or Rust host terminated.",
        fix: "Inspect logs, restart the app, and retain the trace.",
        doc: None,
    },
    KnownError {
        code: "BACKEND_ERROR",
        title: "Backend threw during startup or reload",
        symptom: "Dev only: a full-screen overlay shows the backend stack.",
        cause: "Uncaught exception in backend code or a failed import.",
        fix: "Read the stack and fix the code; the overlay clears on the next successful reload.",
        doc: None,
    },
    KnownError {
        code: "PROTOCOL_ERROR",
        title: "Wire decode failed",
        symptom: "TyndIpcError code=PROTOCOL_ERROR.",
        cause: "Payload serialization or decoding failed.",
        fix: "Use plain JSON-compatible values or registered reducers.",
        doc: Some("tynd://docs/ipc"),
    },
    KnownError {
        code: "PLUGIN_NOT_REGISTERED",
        title: "Plugin not registered with the host",
        symptom: "Plugin RPC returns UNKNOWN_FN although its crate exists.",
        cause: "Plugin is absent from config or the build.",
        fix: "Register the plugin, rebuild, and restart.",
        doc: Some("tynd://docs/architecture"),
    },
    KnownError {
        code: "IPC_ENVELOPE_MISMATCH",
        title: "IPC envelope shape mismatch",
        symptom: "Frontend cannot decode the IPC envelope.",
        cause: "Frontend generated types and backend protocol versions differ.",
        fix: "Regenerate types and rebuild both sides.",
        doc: Some("tynd://docs/build-pipeline"),
    },
    KnownError {
        code: "WEBVIEW_NAVIGATION_BLOCKED",
        title: "Webview navigation blocked by policy",
        symptom: "Navigation is ignored and security logs report a block.",
        cause: "Navigation policy denies the destination.",
        fix: "Add the required origin to the navigation allow-list.",
        doc: Some("tynd://docs/security"),
    },
    KnownError {
        code: "UPDATER_KEY_MISMATCH",
        title: "Updater public key mismatch",
        symptom: "Updater rejects a fetched manifest.",
        cause: "Manifest and application use different signing keys.",
        fix: "Sign with the matching key or rotate keys through a release.",
        doc: Some("tynd://docs/auto-updates"),
    },
    KnownError {
        code: "DEEP_LINK_NOT_REGISTERED",
        title: "Deep link scheme not registered",
        symptom: "Opening a custom scheme does not focus the app.",
        cause: "OS scheme registration is absent or stale.",
        fix: "Register the scheme in dev or rebuild the packaged app.",
        doc: Some("tynd://docs/deep-linking"),
    },
    KnownError {
        code: "CODESIGN_MISSING",
        title: "App is not codesigned",
        symptom: "macOS Gatekeeper rejects the bundle.",
        cause: "Build lacks a valid signing identity.",
        fix: "Configure signing and rebuild with signing enabled.",
        doc: Some("tynd://docs/code-signing"),
    },
    KnownError {
        code: "DEBUG_SOCKET_MISSING",
        title: "Debug socket not running",
        symptom: "MCP remains in static mode while the app runs.",
        cause: "Dev process did not publish a discovery file.",
        fix: "Use a current Tynd CLI and run tynd dev.",
        doc: None,
    },
    KnownError {
        code: "GPU_FALLBACK",
        title: "GPU acceleration unavailable",
        symptom: "Renderer reports a device fallback and low frame rate.",
        cause: "No supported GPU adapter was selected.",
        fix: "Update drivers or adjust GPU power preference.",
        doc: Some("tynd://docs/troubleshooting"),
    },
];

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fs, path::PathBuf};

    use super::{embedded, Catalog};

    const MAX_ASSET_BYTES: usize = 2 * 1024 * 1024;

    fn generated_dir(kind: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packages/mcp/catalog")
            .join(kind)
    }

    /// The embedded tables are exactly the generated files, byte for byte.
    fn assert_embeds(kind: &str, assets: &[(&str, &str)]) {
        let on_disk: BTreeSet<String> = fs::read_dir(generated_dir(kind))
            .expect("read generated catalog")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let embedded: BTreeSet<String> =
            assets.iter().map(|(name, _)| (*name).to_owned()).collect();
        assert_eq!(
            embedded, on_disk,
            "{kind}: embedded list differs from the generated one"
        );
        for (name, body) in assets {
            let file = fs::read_to_string(generated_dir(kind).join(name)).expect("read asset");
            assert_eq!(*body, file, "{kind}/{name} is not the generated copy");
            assert!(
                !body.is_empty() && body.len() <= MAX_ASSET_BYTES,
                "{kind}/{name} must be between 1 byte and 2 MiB"
            );
        }
    }

    #[test]
    fn the_catalog_is_compiled_into_the_binary() {
        assert_embeds("framework-docs", embedded::FRAMEWORK_DOCS);
        assert_embeds("plugin-schemas", embedded::PLUGIN_SCHEMAS);

        let catalog = Catalog::embedded();
        let docs = catalog.list_docs().expect("docs");
        assert_eq!(docs.len(), embedded::FRAMEWORK_DOCS.len());
        let ipc = catalog.find_doc("ipc").expect("lookup").expect("ipc doc");
        assert_ne!(
            ipc.title, "ipc",
            "the title comes from the level-one heading"
        );
        let schemas = catalog.list_plugin_schemas().expect("schemas");
        assert_eq!(schemas.len(), embedded::PLUGIN_SCHEMAS.len());
        assert!(catalog
            .find_plugin_schema("audio")
            .expect("lookup")
            .is_some());
        assert!(catalog.find_doc("../ipc").expect("lookup").is_none());
    }

    #[test]
    fn framework_docs_have_frontmatter_and_a_heading() {
        for (name, source) in embedded::FRAMEWORK_DOCS {
            let source = source.replace("\r\n", "\n");
            let rest = source
                .strip_prefix("---\n")
                .unwrap_or_else(|| panic!("{name} needs YAML frontmatter"));
            let (frontmatter, body) = rest
                .split_once("\n---\n")
                .unwrap_or_else(|| panic!("{name} needs closed YAML frontmatter"));
            for key in ["title:", "description:"] {
                assert!(
                    frontmatter.lines().any(|line| line
                        .strip_prefix(key)
                        .is_some_and(|value| !value.trim().is_empty())),
                    "{name} frontmatter needs {key}"
                );
            }
            assert!(
                body.lines().any(|line| line
                    .strip_prefix("# ")
                    .is_some_and(|title| !title.trim().is_empty())),
                "{name} needs a level-one heading"
            );
        }
    }

    #[test]
    fn plugin_schemas_export_a_declaration() {
        for (name, source) in embedded::PLUGIN_SCHEMAS {
            assert!(
                source.lines().any(|line| {
                    line.trim_start()
                        .strip_prefix("export ")
                        .is_some_and(|rest| {
                            [
                                "type ",
                                "interface ",
                                "const ",
                                "enum ",
                                "class ",
                                "function ",
                            ]
                            .iter()
                            .any(|keyword| rest.starts_with(keyword))
                        })
                }),
                "{name} needs an exported TypeScript declaration"
            );
        }
    }
}
