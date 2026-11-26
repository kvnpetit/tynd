use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::shared::walk_up;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct PackageJson {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub author: Option<PackageAuthor>,
    pub scripts: BTreeMap<String, String>,
    pub dependencies: BTreeMap<String, String>,
    pub dev_dependencies: BTreeMap<String, String>,
    pub peer_dependencies: BTreeMap<String, String>,
    pub optional_dependencies: BTreeMap<String, String>,
    pub trusted_dependencies: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl PackageJson {
    pub(crate) fn all_dependencies(&self) -> BTreeMap<String, String> {
        let mut dependencies = self.dependencies.clone();
        dependencies.extend(self.dev_dependencies.clone());
        dependencies
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum PackageAuthor {
    Text(String),
    Detailed(Author),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Author {
    pub name: String,
    pub email: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BuildTool {
    Vite,
    Cra,
    Angular,
    Parcel,
    Rsbuild,
    Webpack,
    None,
}

impl BuildTool {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Vite => "vite",
            Self::Cra => "cra",
            Self::Angular => "angular",
            Self::Parcel => "parcel",
            Self::Rsbuild => "rsbuild",
            Self::Webpack => "webpack",
            Self::None => "none",
        }
    }
}

impl fmt::Display for BuildTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct CommandPlan {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandPlan {
    pub(crate) fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

impl fmt::Display for CommandPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.program)?;
        for argument in &self.args {
            write!(formatter, " {argument}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrontendInfo {
    pub build_tool: BuildTool,
    /// Production output directory, relative to the project root.
    pub out_dir: PathBuf,
    pub build_command: Option<CommandPlan>,
    pub dev_command: Option<CommandPlan>,
    pub dev_url: Option<String>,
    pub blocked_by: Option<String>,
}

impl FrontendInfo {
    fn empty(blocked_by: Option<String>) -> Self {
        Self {
            build_tool: BuildTool::None,
            out_dir: PathBuf::from("frontend"),
            build_command: None,
            dev_command: None,
            dev_url: None,
            blocked_by,
        }
    }
}

pub(crate) fn load_package_json(cwd: &Path) -> Result<Option<PackageJson>> {
    find_package_json(cwd)
        .as_deref()
        .map(read_package_json)
        .transpose()
}

pub(crate) fn read_package_json(path: &Path) -> Result<PackageJson> {
    let source = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let package: PackageJson =
        serde_json::from_str(&source).with_context(|| format!("parse {}", path.display()))?;
    if package.extra.is_empty()
        && package.name.is_none()
        && package.scripts.is_empty()
        && package.dependencies.is_empty()
        && package.dev_dependencies.is_empty()
    {
        bail!("package.json is empty: {}", path.display());
    }
    Ok(package)
}

pub(crate) fn find_package_json(cwd: &Path) -> Option<PathBuf> {
    walk_up(cwd).find_map(|directory| {
        let path = directory.join("package.json");
        path.is_file().then_some(path)
    })
}

pub(crate) fn detect_frontend(cwd: &Path) -> Result<FrontendInfo> {
    let Some(package) = load_package_json(cwd)? else {
        return Ok(FrontendInfo::empty(None));
    };
    let dependencies = package.all_dependencies();
    if let Some(framework) = blocked_framework(&dependencies) {
        return Ok(FrontendInfo::empty(Some(framework.to_owned())));
    }

    let build_tool = detect_build_tool(&dependencies);
    let out_dir = resolve_output_directory(cwd, build_tool)?;
    let (dev_command, dev_url, build_command) = match build_tool {
        BuildTool::Vite => (
            Some(CommandPlan::new("bunx", ["--bun", "vite"])),
            Some("http://localhost:5173".to_owned()),
            Some(CommandPlan::new("bunx", ["--bun", "vite", "build"])),
        ),
        BuildTool::Cra => (
            Some(CommandPlan::new("bunx", ["react-scripts", "start"])),
            Some("http://localhost:3000".to_owned()),
            Some(CommandPlan::new("bunx", ["react-scripts", "build"])),
        ),
        BuildTool::Angular => (
            Some(CommandPlan::new("bunx", ["ng", "serve"])),
            Some("http://localhost:4200".to_owned()),
            Some(CommandPlan::new("bunx", ["ng", "build"])),
        ),
        BuildTool::Parcel => (
            Some(CommandPlan::new("bunx", ["parcel"])),
            Some("http://localhost:1234".to_owned()),
            Some(CommandPlan::new("bunx", ["parcel", "build"])),
        ),
        BuildTool::Rsbuild => (
            Some(CommandPlan::new("bunx", ["rsbuild", "dev"])),
            Some("http://localhost:3000".to_owned()),
            Some(CommandPlan::new("bunx", ["rsbuild", "build"])),
        ),
        BuildTool::Webpack => (
            Some(CommandPlan::new("bunx", ["webpack", "serve"])),
            Some("http://localhost:8080".to_owned()),
            Some(CommandPlan::new(
                "bunx",
                ["webpack", "--mode", "production"],
            )),
        ),
        BuildTool::None => (None, None, None),
    };
    Ok(FrontendInfo {
        build_tool,
        out_dir,
        build_command,
        dev_command,
        dev_url,
        blocked_by: None,
    })
}

pub(crate) fn resolve_package_dir(package: &str, cwd: &Path) -> Result<Option<PathBuf>> {
    for directory in walk_up(cwd) {
        let installed = directory.join("node_modules").join(package);
        if installed.join("package.json").is_file() {
            return Ok(Some(installed));
        }
        for workspace in ["packages", "plugins/host", "plugins/standalone"] {
            let root = directory.join(workspace);
            let Ok(entries) = fs::read_dir(root) else {
                continue;
            };
            for entry in entries.flatten() {
                if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let manifest = entry.path().join("package.json");
                let Ok(candidate) = read_package_json(&manifest) else {
                    continue;
                };
                if candidate.name.as_deref() == Some(package) {
                    return Ok(manifest.parent().map(Path::to_path_buf));
                }
            }
        }
    }
    Ok(None)
}

pub(crate) fn normalize_author(author: Option<&PackageAuthor>) -> Option<Author> {
    match author? {
        PackageAuthor::Detailed(author) if !author.name.trim().is_empty() => Some(author.clone()),
        PackageAuthor::Detailed(_) => None,
        PackageAuthor::Text(value) => {
            let pattern =
                Regex::new(r"^\s*([^<(]+?)\s*(?:<([^>]*)>)?\s*(?:\(([^)]*)\))?\s*$").ok()?;
            let captures = pattern.captures(value)?;
            let name = captures.get(1)?.as_str().trim().to_owned();
            if name.is_empty() {
                return None;
            }
            let email = captures
                .get(2)
                .map(|value| value.as_str().trim().to_owned())
                .filter(|value| !value.is_empty());
            let url = captures
                .get(3)
                .map(|value| value.as_str().trim().to_owned())
                .filter(|value| !value.is_empty());
            Some(Author { name, email, url })
        },
    }
}

fn blocked_framework(dependencies: &BTreeMap<String, String>) -> Option<&'static str> {
    const SERVER_FRAMEWORKS: &[(&str, &str)] = &[
        ("next", "Next.js"),
        ("@remix-run/react", "Remix"),
        ("@remix-run/node", "Remix"),
        ("@remix-run/cloudflare", "Remix"),
        ("@remix-run/deno", "Remix"),
        ("gatsby", "Gatsby"),
        ("@redwoodjs/core", "RedwoodJS"),
        ("blitz", "Blitz.js"),
        ("nuxt", "Nuxt"),
        ("@nuxtjs/bridge", "Nuxt Bridge"),
        ("@sveltejs/kit", "SvelteKit"),
        ("@solidjs/start", "SolidStart"),
        ("@angular/platform-server", "Angular Universal"),
        ("@nguniversal/express-engine", "Angular Universal"),
        ("@analogjs/core", "Analog"),
        ("@analogjs/platform", "Analog"),
        ("@builder.io/qwik-city", "Qwik City"),
        ("astro", "Astro"),
        ("@tanstack/start", "TanStack Start"),
        ("@tanstack/react-start", "TanStack Start"),
        ("vike", "Vike (vite-plugin-ssr)"),
        ("vite-plugin-ssr", "vite-plugin-ssr"),
    ];
    SERVER_FRAMEWORKS
        .iter()
        .find_map(|(package, name)| dependencies.contains_key(*package).then_some(*name))
}

fn detect_build_tool(dependencies: &BTreeMap<String, String>) -> BuildTool {
    if dependencies.contains_key("vite")
        || dependencies.keys().any(|name| name.starts_with("@vitejs/"))
    {
        BuildTool::Vite
    } else if dependencies.contains_key("react-scripts") {
        BuildTool::Cra
    } else if dependencies.contains_key("@angular/cli")
        || dependencies.contains_key("@angular-devkit/build-angular")
    {
        BuildTool::Angular
    } else if dependencies.contains_key("parcel") || dependencies.contains_key("parcel-bundler") {
        BuildTool::Parcel
    } else if dependencies.contains_key("@rsbuild/core") {
        BuildTool::Rsbuild
    } else if dependencies.contains_key("webpack") || dependencies.contains_key("webpack-cli") {
        BuildTool::Webpack
    } else {
        BuildTool::None
    }
}

fn resolve_output_directory(cwd: &Path, tool: BuildTool) -> Result<PathBuf> {
    let configured = match tool {
        BuildTool::Vite => vite_output_directory(cwd)?,
        BuildTool::Angular => angular_output_directory(cwd)?,
        BuildTool::Rsbuild => rsbuild_output_directory(cwd)?,
        _ => None,
    };
    Ok(configured.unwrap_or_else(|| {
        PathBuf::from(match tool {
            BuildTool::Cra => "build",
            BuildTool::None => "frontend",
            BuildTool::Vite
            | BuildTool::Angular
            | BuildTool::Parcel
            | BuildTool::Rsbuild
            | BuildTool::Webpack => "dist",
        })
    }))
}

fn vite_output_directory(cwd: &Path) -> Result<Option<PathBuf>> {
    let pattern = Regex::new(r#"outDir\s*:\s*['"]([^'"]{1,512})['"]"#)?;
    for name in [
        "vite.config.ts",
        "vite.config.js",
        "vite.config.mts",
        "vite.config.mjs",
    ] {
        let path = cwd.join(name);
        let Some(source) = read_small_text(&path)? else {
            continue;
        };
        let source = strip_javascript_comments(&source)?;
        if let Some(value) = pattern
            .captures(&source)
            .and_then(|captures| captures.get(1))
        {
            return Ok(Some(PathBuf::from(value.as_str())));
        }
    }
    Ok(None)
}

fn angular_output_directory(cwd: &Path) -> Result<Option<PathBuf>> {
    let path = cwd.join("angular.json");
    let Some(source) = read_small_text(&path)? else {
        return Ok(None);
    };
    if let Ok(value) = serde_json::from_str::<Value>(&source) {
        if let Some(projects) = value.get("projects").and_then(Value::as_object) {
            let selected = projects
                .iter()
                .find(|(_, project)| {
                    project.get("projectType").and_then(Value::as_str) == Some("application")
                })
                .or_else(|| projects.iter().next());
            if let Some((name, project)) = selected {
                let build = project
                    .pointer("/architect/build")
                    .or_else(|| project.pointer("/targets/build"));
                let output = build
                    .and_then(|build| build.pointer("/options/outputPath"))
                    .and_then(Value::as_str)
                    .map_or_else(|| format!("dist/{name}"), str::to_owned);
                let browser = build
                    .and_then(|build| build.get("builder"))
                    .and_then(Value::as_str)
                    .is_some_and(|builder| builder.contains("@angular/build:application"));
                return Ok(Some(PathBuf::from(if browser {
                    format!("{output}/browser")
                } else {
                    output
                })));
            }
        }
    }
    let pattern = Regex::new(r#""outputPath"\s*:\s*"([^"]+)""#)?;
    Ok(pattern
        .captures(&source)
        .and_then(|captures| captures.get(1))
        .map(|value| PathBuf::from(value.as_str())))
}

fn rsbuild_output_directory(cwd: &Path) -> Result<Option<PathBuf>> {
    let pattern = Regex::new(r#"(?s)distPath\s*:\s*\{[^}]*root\s*:\s*['"]([^'"]{1,512})['"]"#)?;
    for name in ["rsbuild.config.ts", "rsbuild.config.js"] {
        let path = cwd.join(name);
        let Some(source) = read_small_text(&path)? else {
            continue;
        };
        if let Some(value) = pattern
            .captures(&source)
            .and_then(|captures| captures.get(1))
        {
            return Ok(Some(PathBuf::from(value.as_str())));
        }
    }
    Ok(None)
}

fn strip_javascript_comments(source: &str) -> Result<String> {
    let block = Regex::new(r"(?s)/\*.*?\*/")?;
    Ok(block
        .replace_all(source, "")
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn read_small_text(path: &Path) -> Result<Option<String>> {
    let Ok(metadata) = path.metadata() else {
        return Ok(None);
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    if metadata.len() > 1024 * 1024 {
        bail!("refusing to inspect config over 1 MiB: {}", path.display());
    }
    fs::read_to_string(path)
        .map(Some)
        .with_context(|| format!("read {}", path.display()))
}
