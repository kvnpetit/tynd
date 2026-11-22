use std::fs;

use super::{
    ensure_backend_reference, ensure_typescript_dx, frontend_reference, includes_backend,
    patch_flat_tsconfig_backend, patch_tsconfig_dx, tsconfig_references, BACKEND_TSCONFIG,
};

/// Characterization cases: `(input tsconfig, patched output)`.
const CASES: &[(&str, &str)] = &[
        (
            "{}",
            "{\n  \"compilerOptions\": {\n    \"types\": [\"@tynd/core/env\"]\n  },}",
        ),
        (
            "{\n  \"compilerOptions\": { \"strict\": true }\n}\n",
            "{\n  \"compilerOptions\": {\n    \"types\": [\"@tynd/core/env\"], \"strict\": true }\n}\n",
        ),
        // Comments and user paths are preserved.
        (
            "{\n  // comment { with braces }\n  \"compilerOptions\": {\n    \"types\": [\"bun\"],\n    \"paths\": {\n      \"x\": [\"y\"] }\n  },\n  \"include\": [\"src\"]\n}\n",
            "{\n  // comment { with braces }\n  \"compilerOptions\": {\n    \"types\": [\"bun\",\n      \"@tynd/core/env\",\n    ],\n    \"paths\": {\n      \"x\": [\"y\"] }\n  },\n  \"include\": [\"src\"]\n}\n",
        ),
        // A user-defined path mapping is left untouched.
        (
            "{\n  \"compilerOptions\": {\n    \"types\": ['@tynd/core/env'],\n    \"paths\": { \"@tynd/core/client\": [\"z\"] }\n  }\n}\n",
            "{\n  \"compilerOptions\": {\n    \"types\": ['@tynd/core/env'],\n    \"paths\": { \"@tynd/core/client\": [\"z\"] }\n  }\n}\n",
        ),
        (
            "{ \"compilerOptions\": { \"types\": [], /* \"paths\": {} */ \"x\": \"a{b\", \"paths\": { \"x\": [\"y\"] } } }",
            "{ \"compilerOptions\": { \"types\": [\n      \"@tynd/core/env\",\n    ], /* \"paths\": {} */ \"x\": \"a{b\", \"paths\": { \"x\": [\"y\"] } } }",
        ),
    ];

#[test]
fn tsconfig_patch_adds_tynd_types() {
    for (input, expected) in CASES {
        let patched = patch_tsconfig_dx(input).expect("tsconfig patch succeeds");
        assert_eq!(&patched, expected, "input: {input}");
    }
}

const VITE_SOLUTION: &str = "{
  \"files\": [],
  \"references\": [
    { \"path\": \"./tsconfig.app.json\" },
    { \"path\": \"./tsconfig.node.json\" }
  ]
}
";

#[test]
fn solution_tsconfig_references_pick_the_app_project() {
    let references = tsconfig_references(VITE_SOLUTION)
        .expect("parse references")
        .expect("references present");
    assert_eq!(references, ["./tsconfig.app.json", "./tsconfig.node.json"]);
    assert_eq!(frontend_reference(&references), Some("./tsconfig.app.json"));

    let angular_like = [
        "./tsconfig.spec.json".to_owned(),
        "./tsconfig.web.json".to_owned(),
    ];
    assert_eq!(
        frontend_reference(&angular_like),
        Some("./tsconfig.web.json")
    );
    assert_eq!(
        frontend_reference(&["./tsconfig.node.json".to_owned()]),
        None
    );

    let commented =
        "{ \"references\": [ /* { \"path\": \"./x.json\" } */ { \"path\": \"./a.json\" } ] }";
    assert_eq!(
        tsconfig_references(commented)
            .expect("parse")
            .expect("present"),
        ["./a.json"]
    );
    assert_eq!(
        tsconfig_references("{ \"include\": [\"src\"] }").expect("parse"),
        None
    );
}

#[test]
fn backend_reference_is_added_once() {
    let patched = ensure_backend_reference(VITE_SOLUTION).expect("patch");
    assert_eq!(
        patched,
        "{
  \"files\": [],
  \"references\": [
    { \"path\": \"./tsconfig.app.json\" },
    { \"path\": \"./tsconfig.node.json\" },
    { \"path\": \"./tsconfig.backend.json\" }
  ]
}
"
    );
    assert_eq!(
        ensure_backend_reference(&patched).expect("repatch"),
        patched
    );
    assert_eq!(
        ensure_backend_reference("{ \"include\": [\"src\"] }").expect("patch"),
        "{
  \"references\": [{ \"path\": \"./tsconfig.backend.json\" }], \"include\": [\"src\"] }"
    );
}

#[test]
fn backend_coverage_follows_include() {
    assert!(includes_backend("{ \"compilerOptions\": {} }").expect("parse"));
    assert!(includes_backend("{ \"include\": [\"frontend\", \"backend\"] }").expect("parse"));
    assert!(!includes_backend("{ \"include\": [\"src\"] }").expect("parse"));
}

#[test]
fn frontend_reference_ignores_paths_outside_the_project() {
    let outside = [
        "../shared/tsconfig.json".to_owned(),
        "./nested/../../x/tsconfig.app.json".to_owned(),
        "/abs/tsconfig.app.json".to_owned(),
        "./tsconfig.web.json".to_owned(),
    ];
    assert_eq!(frontend_reference(&outside), Some("./tsconfig.web.json"));
    assert_eq!(
        frontend_reference(&["../shared/tsconfig.app.json".to_owned()]),
        None
    );
}

#[test]
fn flat_tsconfig_includes_backend_and_bun_types() {
    let patched = patch_flat_tsconfig_backend(
        "{ \"compilerOptions\": { \"types\": [\"vite/client\"] }, \"include\": [\"src\"] }",
    )
    .expect("patch");
    assert!(!patched.contains("references"), "{patched}");
    assert!(includes_backend(&patched).expect("parse"), "{patched}");
    assert!(
        patched.contains("\"include\": [\"src\", \"backend\", \"tynd.config.ts\"]"),
        "{patched}"
    );
    assert!(patched.contains("\"bun\""), "{patched}");
    assert_eq!(
        patch_flat_tsconfig_backend(&patched).expect("repatch"),
        patched
    );
    // `files` without `include` gets an include for the backend.
    let files_only =
        patch_flat_tsconfig_backend("{ \"files\": [\"src/main.ts\"] }").expect("patch");
    assert!(
        includes_backend(&files_only).expect("parse"),
        "{files_only}"
    );
    // Without `include`/`files` everything is already checked.
    assert_eq!(
        patch_flat_tsconfig_backend("{ \"compilerOptions\": {} }").expect("patch"),
        "{ \"compilerOptions\": {} }"
    );
}

fn scaffold_tsconfigs(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, content) in files {
        fs::write(dir.path().join(name), content).expect("write fixture");
    }
    ensure_typescript_dx(dir.path()).expect("patch tsconfig");
    dir
}

fn read(dir: &tempfile::TempDir, name: &str) -> String {
    fs::read_to_string(dir.path().join(name)).expect("read")
}

#[test]
fn flat_tsconfig_gets_no_backend_reference() {
    let dir = scaffold_tsconfigs(&[(
        "tsconfig.json",
        "{ \"compilerOptions\": { \"strict\": true }, \"include\": [\"src\"] }",
    )]);
    let root = read(&dir, "tsconfig.json");
    assert!(!root.contains("references"), "{root}");
    assert!(includes_backend(&root).expect("parse"), "{root}");
    assert!(
        root.contains("@tynd/core/env") && root.contains("\"bun\""),
        "{root}"
    );
    assert!(!dir.path().join(BACKEND_TSCONFIG).exists());
}

#[test]
fn lit_single_tsconfig_type_checks_backend_from_root() {
    let lit = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "experimentalDecorators": true,
    "useDefineForClassFields": false,
    "module": "ESNext",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "types": ["vite/client"],
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "verbatimModuleSyntax": true,
    "noEmit": true,
    "strict": true
  },
  "include": ["src"]
}
"#;
    let dir = scaffold_tsconfigs(&[("tsconfig.json", lit)]);
    let root = read(&dir, "tsconfig.json");
    assert!(!root.contains("references"), "{root}");
    for expected in [
        "\"src\"",
        "\"backend\"",
        "\"tynd.config.ts\"",
        "\"vite/client\"",
        "\"bun\"",
        "\"@tynd/core/env\"",
    ] {
        assert!(root.contains(expected), "missing {expected}: {root}");
    }
    assert!(!dir.path().join(BACKEND_TSCONFIG).exists());
}

#[test]
fn solution_tsconfig_references_backend_and_patches_app() {
    let dir = scaffold_tsconfigs(&[
        ("tsconfig.json", VITE_SOLUTION),
        (
            "tsconfig.app.json",
            "{ \"compilerOptions\": {}, \"include\": [\"src\"] }",
        ),
        ("tsconfig.node.json", "{ \"compilerOptions\": {} }"),
    ]);
    assert!(read(&dir, "tsconfig.json").contains("./tsconfig.backend.json"));
    assert!(read(&dir, "tsconfig.app.json").contains("@tynd/core/env"));
    assert!(dir.path().join(BACKEND_TSCONFIG).is_file());
    assert!(!dir.path().join("global.d.ts").exists());
}

#[test]
fn root_with_inputs_and_references_stays_flat() {
    for root_config in [
            // Older vite templates: own inputs plus a tooling reference.
            "{ \"compilerOptions\": { \"strict\": true }, \"include\": [\"src\"], \"references\": [{ \"path\": \"./tsconfig.node.json\" }] }",
            "{ \"compilerOptions\": {}, \"include\": [\"src\"], \"references\": [] }",
            "{ \"compilerOptions\": {}, \"files\": [\"src/main.ts\"], \"references\": [] }",
        ] {
            let dir = scaffold_tsconfigs(&[
                ("tsconfig.json", root_config),
                ("tsconfig.node.json", "{ \"compilerOptions\": {} }"),
            ]);
            let root = read(&dir, "tsconfig.json");
            assert!(!root.contains(BACKEND_TSCONFIG), "{root}");
            assert!(includes_backend(&root).expect("parse"), "{root}");
            assert!(!dir.path().join(BACKEND_TSCONFIG).exists(), "{root}");
            assert_eq!(
                patch_flat_tsconfig_backend(&root).expect("repatch"),
                root,
                "{root}"
            );
        }
    // `files: []` without `include` is still a solution root.
    let dir = scaffold_tsconfigs(&[(
        "tsconfig.json",
        "{ \"files\": [ /* none */ ], \"references\": [] }",
    )]);
    assert!(read(&dir, "tsconfig.json").contains(BACKEND_TSCONFIG));
    assert!(dir.path().join(BACKEND_TSCONFIG).is_file());
    // An empty `include`, with or without `files: []`, adds no inputs.
    for root_config in [
        "{ \"files\": [], \"include\": [ // none
 ], \"references\": [] }",
        "{ \"include\": [], \"references\": [] }",
    ] {
        let dir = scaffold_tsconfigs(&[("tsconfig.json", root_config)]);
        assert!(read(&dir, "tsconfig.json").contains(BACKEND_TSCONFIG));
        assert!(dir.path().join(BACKEND_TSCONFIG).is_file());
    }
}

#[test]
fn comment_markers_inside_strings_are_not_comments() {
    let include = "{ \"include\": [\"src//x\", \"a/*b\" // app
] }";
    // `parse_jsonc` is string-unaware, so compare the text directly.
    let patched = patch_flat_tsconfig_backend(include).expect("patch");
    assert!(
        patched.contains("\"a/*b\", \"backend\", \"tynd.config.ts\" // app"),
        "{patched}"
    );
    assert_eq!(
        patch_flat_tsconfig_backend(&patched).expect("repatch"),
        patched
    );
}

/// Parses JSONC (comments, trailing commas) as tsc would accept it.
fn parse_jsonc(source: &str) -> serde_json::Value {
    let comments = regex::Regex::new(r"(?s)//[^\n]*|/\*.*?\*/").expect("regex");
    let trailing = regex::Regex::new(r",(\s*[\]}])").expect("regex");
    let stripped = comments.replace_all(source, "");
    let stripped = trailing.replace_all(&stripped, "$1");
    serde_json::from_str(&stripped).unwrap_or_else(|error| panic!("{error}: {source}"))
}

fn assert_patch_keeps_comments(
    patch: fn(&str) -> anyhow::Result<String>,
    input: &str,
    comment: &str,
) -> serde_json::Value {
    let patched = patch(input).expect("patch");
    assert!(patched.contains(comment), "{patched}");
    assert_eq!(patch(&patched).expect("repatch"), patched, "{patched}");
    parse_jsonc(&patched)
}

#[test]
fn trailing_comments_do_not_swallow_inserted_items() {
    for comment in ["// app", "/* app */"] {
        let include = format!("{{ \"include\": [\n    \"src\" {comment}\n  ] }}");
        let value = assert_patch_keeps_comments(patch_flat_tsconfig_backend, &include, comment);
        assert_eq!(
            value["include"],
            serde_json::json!(["src", "backend", "tynd.config.ts"])
        );

        let references = format!(
                "{{ \"files\": [], \"references\": [\n    {{ \"path\": \"./tsconfig.app.json\" }} {comment}\n  ] }}"
            );
        let value = assert_patch_keeps_comments(ensure_backend_reference, &references, comment);
        assert_eq!(
            value["references"][1]["path"],
            format!("./{BACKEND_TSCONFIG}")
        );

        let types = format!(
            "{{ \"compilerOptions\": {{ \"types\": [\n      \"vite/client\" {comment}\n    ] }} }}"
        );
        let value = assert_patch_keeps_comments(patch_tsconfig_dx, &types, comment);
        assert_eq!(
            value["compilerOptions"]["types"],
            serde_json::json!(["vite/client", "@tynd/core/env"])
        );
        let value = assert_patch_keeps_comments(patch_flat_tsconfig_backend, &types, comment);
        assert_eq!(
            value["compilerOptions"]["types"],
            serde_json::json!(["vite/client", "bun"])
        );
    }
}
