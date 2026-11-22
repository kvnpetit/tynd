use anyhow::{bail, Result};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::fsx::{atomic_write, write_json_file};
use crate::output::{display_path, success};

use super::{
    shared::{
        native_crate_ident, native_crate_name, parse_plugin_name, validate_native_crate_name,
    },
    PluginNewRequest,
};

pub(in crate::commands) fn plugin_new(request: &PluginNewRequest) -> Result<PathBuf> {
    let identity = parse_plugin_name(&request.name)?;
    let directory = request.parent.join(identity.local_name);
    if directory.exists() {
        if !request.force {
            bail!(
                "{} already exists (use --force to overwrite)",
                directory.display()
            );
        }
        fs::remove_dir_all(&directory)?;
    }
    fs::create_dir_all(&directory)?;
    let local_package_name = if identity.local_name.starts_with("plugin-") {
        identity.local_name.to_owned()
    } else {
        format!("plugin-{}", identity.local_name)
    };
    let package_name = if let Some(scope) = identity.scope {
        format!("@{scope}/{local_package_name}")
    } else {
        local_package_name
    };
    let runtime_name = identity
        .local_name
        .strip_prefix("plugin-")
        .unwrap_or(identity.local_name);
    let crate_name = native_crate_name(&package_name);
    validate_native_crate_name(&crate_name)?;
    let generated_name = format!("{runtime_name}.gen.ts");
    let manifest = if request.native {
        json!({
            "name": package_name,
            "version": "0.1.0",
            "private": true,
            "license": "Apache-2.0",
            "type": "module",
            "exports": { ".": "./client.ts", "./client": "./client.ts" },
            "files": ["client.ts", generated_name, "native"],
            "peerDependencies": { "@tynd/core": "^1.0.0", "@tynd/plugin-sdk": "^1.0.0" },
            "peerDependenciesMeta": {
                "@tynd/core": { "optional": true },
                "@tynd/plugin-sdk": { "optional": true }
            },
            "devDependencies": { "@tynd/plugin-sdk": "^1.0.0" },
            "tynd": {
                "native": { "path": "./native", "crateName": crate_name }
            }
        })
    } else {
        json!({
            "name": package_name,
            "version": "0.1.0",
            "private": true,
            "license": "Apache-2.0",
            "type": "module",
            "exports": {
                ".": "./client.ts",
                "./client": "./client.ts",
                "./backend": "./backend.ts"
            },
            "files": ["backend.ts", "client.ts"],
            "peerDependencies": { "@tynd/core": "^1.0.0" },
            "tynd": { "backend": true }
        })
    };
    write_json_file(&directory.join("package.json"), &manifest)?;
    atomic_write(
        &directory.join("README.md"),
        format!("# {package_name}\n\nTynd plugin.\n"),
    )?;
    let quoted_name = serde_json::to_string(runtime_name)?;
    if request.native {
        atomic_write(
            &directory.join("client.ts"),
            format!(
                "import {{ defineTypedPlugin }} from \"@tynd/plugin-sdk\"\nimport type {{ CommandsCatalog }} from \"./{runtime_name}.gen\"\n\nexport const plugin = defineTypedPlugin<CommandsCatalog>({quoted_name})\nexport default plugin\n",
            ),
        )?;
        atomic_write(
            &directory.join(&generated_name),
            "export type PingArgs = null\nexport type PingResult = string\n\nexport const _commands = {\n  ping: {\n    args: undefined as unknown as PingArgs,\n    result: undefined as unknown as PingResult,\n  },\n} as const\nexport type CommandsCatalog = typeof _commands\n",
        )?;
        scaffold_native_plugin(&directory, &crate_name, runtime_name)?;
    } else {
        atomic_write(
            &directory.join("backend.ts"),
            format!(
                "import {{ definePlugin }} from \"@tynd/core/plugin\"\n\nexport const backend = definePlugin({{\n  name: {quoted_name},\n  methods: {{\n    async ping(): Promise<string> {{\n      return \"pong\"\n    }},\n  }},\n}})\nexport default backend\n",
            ),
        )?;
        atomic_write(
            &directory.join("client.ts"),
            format!(
                "import {{ createPluginClient }} from \"@tynd/core/client\"\nimport type backend from \"./backend\"\n\nexport const plugin = createPluginClient<typeof backend>({quoted_name})\nexport default plugin\n",
            ),
        )?;
    }
    success(format!("Plugin -> {}", display_path(&directory)));
    Ok(directory)
}

fn scaffold_native_plugin(directory: &Path, crate_name: &str, runtime_name: &str) -> Result<()> {
    let native = directory.join("native");
    fs::create_dir_all(native.join("src/bin"))?;
    atomic_write(
        &native.join("Cargo.toml"),
        format!(
            "[package]\nname = {crate_name:?}\nversion = \"0.1.0\"\npublish = false\nedition = \"2021\"\nrust-version = \"1.95\"\nlicense = \"Apache-2.0\"\n\n[lib]\ncrate-type = [\"rlib\"]\ndoctest = false\n\n[dependencies]\ntynd-plugin-sdk = {{ workspace = true }}\n\n[[bin]]\nname = \"gen-types\"\npath = \"src/bin/gen-types.rs\"\n"
        ),
    )?;
    atomic_write(
        &native.join("src/lib.rs"),
        format!(
            r#"use tynd_plugin_sdk::{{
    tynd_plugin::{{plugin, plugin_impl}},
    PluginResult,
}};

#[plugin(name = {runtime_name:?})]
#[derive(Debug, Default)]
pub struct Plugin;

#[plugin_impl]
impl Plugin {{
    #[command]
    pub fn ping(&self) -> PluginResult<String> {{
        Ok("pong".to_owned())
    }}
}}
"#
        ),
    )?;
    atomic_write(
        &native.join("src/bin/gen-types.rs"),
        format!(
            "tynd_plugin_sdk::tynd_plugin::gen_types_bin!({});
",
            native_crate_ident(crate_name)
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::Value;

    use super::{super::validation::plugin_check, plugin_new, PluginNewRequest};
    use crate::commands::plugins::PluginCheckRequest;

    fn scaffold(name: &str, native: bool) -> (tempfile::TempDir, std::path::PathBuf) {
        let parent = tempfile::tempdir().expect("tempdir");
        let directory = plugin_new(&PluginNewRequest {
            name: name.to_owned(),
            parent: parent.path().to_path_buf(),
            native,
            force: false,
        })
        .expect("scaffold plugin");
        (parent, directory)
    }

    fn manifest(directory: &std::path::Path) -> Value {
        serde_json::from_str(
            &fs::read_to_string(directory.join("package.json")).expect("read package.json"),
        )
        .expect("parse package.json")
    }

    fn assert_check_passes(directory: &std::path::Path) {
        let checks = plugin_check(&PluginCheckRequest {
            directory: directory.to_path_buf(),
            json: false,
        })
        .expect("scaffold passes plugin check");
        assert!(
            checks.iter().all(|check| check.status == "ok"),
            "{:?}",
            checks
                .iter()
                .filter(|check| check.status != "ok")
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn pure_js_scaffold_splits_backend_and_client() {
        let (_parent, directory) = scaffold("@acme/counter", false);
        let manifest = manifest(&directory);
        assert_eq!(manifest["name"], "@acme/plugin-counter");
        assert_eq!(manifest["exports"]["."], "./client.ts");
        assert_eq!(manifest["exports"]["./client"], "./client.ts");
        assert_eq!(manifest["exports"]["./backend"], "./backend.ts");
        assert_eq!(manifest["tynd"], serde_json::json!({ "backend": true }));
        assert!(manifest
            .pointer("/dependencies/@tynd~1plugin-sdk")
            .is_none());
        let backend = fs::read_to_string(directory.join("backend.ts")).expect("backend.ts");
        assert!(backend.contains("import { definePlugin } from \"@tynd/core/plugin\""));
        assert!(backend.contains("name: \"counter\""));
        let client = fs::read_to_string(directory.join("client.ts")).expect("client.ts");
        assert!(client.contains("createPluginClient<typeof backend>(\"counter\")"));
        assert!(!directory.join("index.ts").exists());
        assert_check_passes(&directory);
    }

    #[test]
    fn native_scaffold_uses_typed_client_over_generated_catalog() {
        let (_parent, directory) = scaffold("echo", true);
        let manifest = manifest(&directory);
        assert_eq!(manifest["exports"]["."], "./client.ts");
        assert_eq!(manifest["exports"]["./client"], "./client.ts");
        assert!(manifest.pointer("/exports/./backend").is_none());
        assert_eq!(
            manifest["tynd"],
            serde_json::json!({ "native": { "path": "./native", "crateName": "plugin-echo" } })
        );
        assert!(manifest.get("dependencies").is_none());
        assert_eq!(
            manifest["peerDependencies"]["@tynd/plugin-sdk"],
            manifest["devDependencies"]["@tynd/plugin-sdk"]
        );
        assert_eq!(
            manifest["peerDependenciesMeta"]["@tynd/plugin-sdk"]["optional"],
            true
        );
        let client = fs::read_to_string(directory.join("client.ts")).expect("client.ts");
        assert!(client.contains("defineTypedPlugin<CommandsCatalog>(\"echo\")"));
        assert!(client.contains("from \"./echo.gen\""));
        assert!(directory.join("echo.gen.ts").is_file());
        assert!(directory.join("native/src/lib.rs").is_file());
        assert!(!directory.join("index.ts").exists());
        assert_check_passes(&directory);
    }
}
