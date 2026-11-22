use super::*;

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|i| (*i).to_string()).collect()
}

fn policy(
    fs_allow: &[&str],
    fs_deny: &[&str],
    http_allow: &[&str],
    http_deny: &[&str],
    default_deny: bool,
) -> SecurityPolicy {
    SecurityPolicy::from_input(
        PolicyInput {
            fs: ScopeInput {
                allow: Vec::new(),
                deny: s(fs_deny),
            },
            http: ScopeInput {
                allow: Vec::new(),
                deny: s(http_deny),
            },
            default_deny,
            ..PolicyInput::default()
        },
        s(fs_allow),
        s(http_allow),
    )
    .expect("valid policy")
}

type Case<'a> = (&'a [&'a str], &'a [&'a str], bool, &'a str, bool);

#[test]
fn unconfigured_policy_allows_fs_http_and_capabilities() {
    let p = SecurityPolicy::default();
    assert!(p.check_fs("/any/path").is_ok());
    assert!(p.check_http("https://example.com").is_ok());
    assert!(p.check_sensitive_capability("anything").is_ok());
}

#[test]
fn unconfigured_policy_rejects_when_configured_variants_used() {
    let p = SecurityPolicy::default();
    assert!(p.check_fs_configured("/x").is_err());
    assert!(p.check_fs_resolved_configured("/x").is_err());
    assert!(p.check_http_configured("https://x.test").is_err());
}

#[test]
fn unconfigured_policy_denies_process_and_shell() {
    let p = SecurityPolicy::default();
    assert!(p.check_process("git").is_err());
    assert!(p.check_shell_exec("echo hi").is_err());
}

#[test]
fn opt_in_errors_point_to_app_start_security() {
    let p = SecurityPolicy::default();
    let process = p.check_process("git").unwrap_err();
    assert_eq!(
        process,
        "process: not permitted - set `app.start({ security: { processAllowedBinaries: [...] } })` to opt in"
    );
    let shell = p.check_shell_exec("echo hi").unwrap_err();
    assert!(
        shell.contains("app.start({ security: { shellExecAllowedCommands"),
        "{shell}"
    );
    let configured = policy(&[], &[], &[], &[], false);
    let sensitive = configured
        .check_sensitive_capability("screenCapture")
        .unwrap_err();
    assert!(
        sensitive.contains("app.start({ security: { allowedSensitiveCapabilities"),
        "{sensitive}"
    );
    for message in [process, shell, sensitive] {
        assert!(!message.contains("security.configure"), "{message}");
    }
}

#[test]
fn fs_matrix_allow_deny_default_deny() {
    let cases: &[Case<'_>] = &[
        (&[], &[], false, "/proj/a.txt", true),
        (&[], &[], true, "/proj/a.txt", false),
        (&["/proj/**"], &[], false, "/proj/a.txt", true),
        (&["/proj/**"], &[], false, "/proj/sub/deep/a.txt", true),
        (&["/proj/**"], &[], false, "/other/a.txt", false),
        (
            &["/proj/**"],
            &["/proj/secret/**"],
            false,
            "/proj/secret/k",
            false,
        ),
        (
            &["/proj/**"],
            &["/proj/secret/**"],
            false,
            "/proj/public/k",
            true,
        ),
        (&[], &["/proj/secret/**"], false, "/proj/secret/k", false),
        (&[], &["/proj/secret/**"], false, "/proj/ok", true),
        (&["/proj/*.txt"], &[], false, "/proj/a.txt", true),
        (&["/proj/*.txt"], &[], false, "/proj/sub/a.txt", false),
    ];
    for (allow, deny, default_deny, path, ok) in cases {
        let p = policy(allow, deny, &[], &[], *default_deny);
        assert_eq!(
            p.check_fs(path).is_ok(),
            *ok,
            "allow={allow:?} deny={deny:?} dd={default_deny} path={path}"
        );
    }
}

#[test]
fn fs_deny_wins_over_allow() {
    let p = policy(&["/proj/**"], &["/proj/**"], &[], &[], false);
    let err = p.check_fs("/proj/a").unwrap_err();
    assert!(err.contains("denied by security policy"), "{err}");
}

#[test]
fn fs_paths_are_lexically_cleaned_before_matching() {
    let p = policy(&["/proj/**"], &["/proj/secret/**"], &[], &[], false);
    assert!(p.check_fs("/proj/public/../secret/k").is_err());
    assert!(p.check_fs("/proj/./public/k").is_ok());
    assert!(p.check_fs("/proj/../etc/passwd").is_err());
}

#[test]
fn fs_default_deny_error_mentions_default_deny() {
    let p = policy(&[], &[], &[], &[], true);
    let err = p.check_fs("/x").unwrap_err();
    assert!(err.contains("defaultDeny"), "{err}");
}

#[test]
fn http_matrix() {
    let cases: &[Case<'_>] = &[
        (&[], &[], false, "https://a.test/x", true),
        (&[], &[], true, "https://a.test/x", false),
        (
            &["https://a.test/**"],
            &[],
            false,
            "https://a.test/x/y",
            true,
        ),
        (
            &["https://a.test/**"],
            &[],
            false,
            "https://b.test/x",
            false,
        ),
        (&["https://a.test/**"], &[], false, "http://a.test/x", false),
        (
            &["https://*.test/**"],
            &["https://evil.test/**"],
            false,
            "https://evil.test/x",
            false,
        ),
        (
            &["https://*.test/**"],
            &["https://evil.test/**"],
            false,
            "https://good.test/x",
            true,
        ),
    ];
    for (allow, deny, default_deny, url, ok) in cases {
        let p = policy(&[], &[], allow, deny, *default_deny);
        assert_eq!(
            p.check_http(url).is_ok(),
            *ok,
            "allow={allow:?} deny={deny:?} dd={default_deny} url={url}"
        );
    }
}

#[test]
fn websocket_urls_match_http_rules() {
    let p = policy(&[], &[], &["https://a.test/**"], &[], false);
    assert!(p.check_http("wss://a.test/socket").is_ok());
    assert!(p.check_http("ws://a.test/socket").is_err());
    let p = policy(&[], &[], &["http://a.test/**"], &[], false);
    assert!(p.check_http("ws://a.test/socket").is_ok());
}

#[test]
fn normalize_http_scheme_maps_websocket_schemes() {
    assert_eq!(normalize_http_scheme("wss://h/p"), "https://h/p");
    assert_eq!(normalize_http_scheme("ws://h/p"), "http://h/p");
    assert_eq!(normalize_http_scheme("https://h/p"), "https://h/p");
    assert_eq!(normalize_http_scheme("ftp://h"), "ftp://h");
}

#[test]
fn invalid_glob_is_rejected_by_try_new_and_from_input() {
    assert!(Scope::try_new(s(&["[unclosed"]), Vec::new()).is_err());
    assert!(Scope::try_new(Vec::new(), s(&["[unclosed"])).is_err());
    let res = SecurityPolicy::from_input(PolicyInput::default(), s(&["{a,"]), Vec::new());
    assert!(res.is_err());
    assert!(validate_glob_pattern("[x").is_err());
    assert!(validate_glob_pattern("/ok/**").is_ok());
}

#[test]
fn invalid_glob_in_lenient_scope_is_treated_as_empty() {
    let scope = Scope::new(s(&["[unclosed"]), Vec::new());
    // Allow list is non-empty but the compiled set is empty: nothing matches.
    assert!(scope.check("/x", false, "fs").is_err());
}

#[test]
fn sensitive_capability_requires_opt_in_once_configured() {
    let mut p = policy(&[], &[], &[], &[], false);
    assert!(p.check_sensitive_capability("clipboard").is_err());
    p.allowed_sensitive_capabilities = s(&["clipboard"]);
    assert!(p.check_sensitive_capability("clipboard").is_ok());
    assert!(p.check_sensitive_capability("screen").is_err());
}

#[test]
fn process_allowlist_matrix() {
    let mut p = policy(&[], &[], &[], &[], false);
    // Configured but no allowlist: denied.
    assert!(p.check_process("git").is_err());
    p.process_allowed_binaries = Some(s(&["git"]));
    assert!(p.check_process("git").is_ok());
    assert!(p.check_process("curl").is_err());
    p.process_allowed_binaries = Some(Vec::new());
    assert!(p.check_process("git").is_err());
}

#[test]
fn shell_exec_matrix() {
    let mut p = SecurityPolicy::default();
    assert!(p.check_shell_exec("echo hi").is_err());
    p.shell_exec_allowed = s(&["echo", "ls"]);
    assert!(p.check_shell_exec("echo hi").is_ok());
    assert!(p.check_shell_exec("   ls -la").is_ok());
    assert!(p.check_shell_exec("rm -rf /").is_err());
    assert!(p.check_shell_exec("").is_err());
    assert!(p.check_shell_exec("   ").is_err());
    for injected in [
        "echo hi; rm -rf /",
        "echo hi && whoami",
        "echo hi | sh",
        "echo $(whoami)",
        "echo `whoami`",
        "echo hi > /etc/passwd",
        "echo \"hi\"",
        "echo hi\nrm x",
        "echo hi\\n",
        "echo;whoami",
        ";echo",
        "echo %X%",
        "echo ^&whoami",
        "echo !X!",
    ] {
        assert!(p.check_shell_exec(injected).is_err(), "{injected:?}");
    }
}

#[test]
fn env_denylist_is_case_insensitive() {
    for key in [
        "PATH",
        "path",
        "Ld_Preload",
        "NODE_OPTIONS",
        "DYLD_INSERT_LIBRARIES",
    ] {
        assert!(env_key_denied(key), "{key}");
    }
    for key in [
        "HOSTNAME",
        "MY_PATH",
        "NODE_ENV",
        "PYTHONUNBUFFERED",
        "LANG",
        "GITHUB_TOKEN",
        "",
    ] {
        assert!(!env_key_denied(key), "{key}");
    }
}

#[test]
fn literal_subtree_glob_escapes_metacharacters() {
    assert_eq!(literal_subtree_glob("/a/b"), "/a/b/**");
    assert_eq!(literal_subtree_glob("/a/b/"), "/a/b/**");
    let glob = literal_subtree_glob("/weird[1]/*dir");
    assert!(matches_pattern(&glob, "/weird[1]/*dir/file"));
    assert!(!matches_pattern(&glob, "/weird1/xdir/file"));
}

#[test]
fn matches_pattern_respects_literal_separator() {
    assert!(matches_pattern("/a/*.txt", "/a/b.txt"));
    assert!(!matches_pattern("/a/*.txt", "/a/b/c.txt"));
    assert!(matches_pattern("/a/**", "/a/b/c.txt"));
    assert!(!matches_pattern("[bad", "x"));
}

#[test]
fn lex_clean_str_collapses_dot_segments() {
    let sep = std::path::MAIN_SEPARATOR;
    assert_eq!(lex_clean_str("a/./b/../c"), format!("a{sep}c"));
    assert_eq!(lex_clean_str(""), "");
}

#[test]
fn canonicalize_fs_normalizes_separators_and_relative_paths() {
    assert_eq!(canonicalize_fs("/a/b/../c"), "/a/c");
    let relative = canonicalize_fs("some/rel/path");
    let cwd = canonicalize_fs(&std::env::current_dir().unwrap().to_string_lossy());
    assert!(relative.starts_with(&cwd), "{relative} vs {cwd}");
    assert!(!relative.contains('\\'));
}

#[test]
fn canonicalize_process_binary_falls_back_to_normalized_input() {
    let missing = canonicalize_process_binary("definitely-not-a-real-binary-xyz");
    assert!(missing.ends_with("definitely-not-a-real-binary-xyz"));
    assert!(!missing.contains('\\'));
}

#[test]
fn expand_tilde_only_expands_home_prefix() {
    assert_eq!(expand_tilde("/abs/path"), "/abs/path");
    assert_eq!(expand_tilde("~user/x"), "~user/x");
    if let Some(home) = dirs::home_dir() {
        assert_eq!(expand_tilde("~"), home.to_string_lossy());
        assert!(expand_tilde("~/x").starts_with(home.to_string_lossy().as_ref()));
    }
}

#[test]
fn http_deny_not_bypassed_by_host_case_port_or_dot() {
    let p = policy(&[], &[], &[], &["https://evil.test/**"], false);
    for url in [
        "https://evil.test/x",
        "https://EVIL.test/x",
        "HTTPS://Evil.Test/x",
        "https://evil.test:443/x",
        "https://evil.test./x",
        "https://evil.test",
        "wss://evil.test/socket",
        "https://evil.test/a/../x",
        "https://evil.test/x?q=1#frag",
    ] {
        assert!(p.check_http(url).is_err(), "{url}");
    }
    assert!(p.check_http("https://good.test/x").is_ok());
    // A different port is a different origin and is not covered by the rule.
    assert!(p.check_http("https://evil.test:8443/x").is_ok());
}

#[test]
fn http_userinfo_is_rejected() {
    let p = policy(&[], &[], &[], &["https://internal.corp/**"], false);
    assert!(p.check_http("https://u@internal.corp/x").is_err());
    assert!(p.check_http("https://u:p@internal.corp/x").is_err());
    let p = policy(&[], &[], &["https://*.example.com/**"], &[], false);
    assert!(p.check_http("https://evil.test@a.example.com/x").is_err());
}

#[test]
fn http_wildcard_host_cannot_be_confused_by_query_fragment_or_userinfo() {
    let p = policy(&[], &[], &["https://*.example.com/**"], &[], false);
    assert!(p.check_http("https://a.example.com/x").is_ok());
    for url in [
        "https://evil.test?.example.com/x",
        "https://evil.test#.example.com/x",
        "https://evil.test/?.example.com/x",
        "https://evil.test/#.example.com/x",
        "https://evil.test\\.example.com/x",
        "https://evil.test@.example.com/x",
        "https://evil.test:80.example.com/x",
        "https://example.com.evil.test/x",
    ] {
        assert!(p.check_http(url).is_err(), "{url}");
    }
}

#[test]
fn http_allow_default_port_and_case_normalized_both_ways() {
    let p = policy(&[], &[], &["HTTPS://API.Example.com:443/v1/**"], &[], false);
    assert!(p.check_http("https://api.example.com/v1/x").is_ok());
    assert!(p.check_http("https://API.EXAMPLE.COM:443/v1/x").is_ok());
    assert!(p.check_http("https://api.example.com:8443/v1/x").is_err());
    let p = policy(&[], &[], &["https://a.test"], &[], false);
    assert!(p.check_http("https://a.test").is_ok());
    assert!(p.check_http("https://a.test/").is_ok());
    assert!(p.check_http("https://a.test/x").is_err());
}

#[test]
fn http_allow_path_is_case_sensitive_and_deny_path_is_not() {
    let p = policy(&[], &[], &["https://api.test/Users/**"], &[], false);
    assert!(p.check_http("https://api.test/Users/Bob").is_ok());
    assert!(p.check_http("https://api.test/users/bob").is_err());
    let p = policy(&[], &[], &[], &["https://api.test/Admin/**"], false);
    assert!(p.check_http("https://api.test/Admin/x").is_err());
    // Deny rules fold case (deny wins over servers that ignore case).
    assert!(p.check_http("https://api.test/admin/x").is_err());
    assert!(p.check_http("https://api.test/other/x").is_ok());
}

#[test]
fn http_query_is_ignored_for_path_rules_but_matchable() {
    let p = policy(&[], &[], &["https://a.test/search"], &[], false);
    assert!(p.check_http("https://a.test/search?q=1").is_ok());
    let p = policy(&[], &[], &[], &["https://a.test/x"], false);
    assert!(p.check_http("https://a.test/x?y=1").is_err());
    let p = policy(&[], &[], &[], &["https://a.test/x?admin=*"], false);
    assert!(p.check_http("https://a.test/x?admin=1").is_err());
    assert!(p.check_http("https://a.test/x?user=1").is_ok());
}

#[test]
fn http_malformed_urls_are_rejected_when_configured() {
    let p = policy(&[], &[], &[], &[], false);
    for url in [
        "not a url",
        "",
        " https://a.test/",
        "https://a.test/\n",
        "https://a.test/\tx",
        "https://",
    ] {
        assert!(p.check_http(url).is_err(), "{url:?}");
    }
    // Unconfigured policies stay permissive.
    assert!(SecurityPolicy::default().check_http("not a url").is_ok());
}

#[test]
fn http_scope_uses_url_patterns() {
    let p = policy(&[], &[], &["https://A.test:443/Api/**"], &[], false);
    assert!(p.check_http("https://a.test/Api/x").is_ok());
    assert!(p.check_http("https://a.test/api/x").is_err());
}

#[test]
fn canonicalize_http_url_forms() {
    assert_eq!(
        canonicalize_http_url("WSS://Ex.Com.:443/A/../B?q=1#f").unwrap(),
        "https://ex.com/B?q=1"
    );
    assert_eq!(
        canonicalize_http_url("ws://h:81/p").unwrap(),
        "http://h:81/p"
    );
    assert!(canonicalize_http_url("https://u@h/").is_err());
}

#[test]
fn shell_exec_rejects_cmd_expansion_characters() {
    let p = SecurityPolicy {
        shell_exec_allowed: s(&["git"]),
        ..SecurityPolicy::default()
    };
    for c in ["%", "^", "!"] {
        let cmd = format!("git a{c}b");
        assert!(p.check_shell_exec(&cmd).is_err(), "{cmd}");
    }
    assert!(p.check_shell_exec("git status").is_ok());
}

#[test]
fn shell_exec_with_env_refuses_any_caller_env() {
    let p = SecurityPolicy {
        shell_exec_allowed: s(&["git"]),
        ..SecurityPolicy::default()
    };
    assert!(p.check_shell_exec_with_env("git status", []).is_ok());
    let err = p
        .check_shell_exec_with_env("git status", ["FOO"])
        .unwrap_err();
    assert!(err.contains("FOO"), "{err}");
    // The command check still applies.
    assert!(p.check_shell_exec_with_env("rm x", []).is_err());
}

#[test]
fn env_denylist_covers_allowlisted_binary_hijacks() {
    for key in [
        "GIT_SSH_COMMAND",
        "GIT_SSH",
        "git_ssh_variant",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_KEY_0",
        "GIT_CONFIG_VALUE_0",
        "GIT_CONFIG_GLOBAL",
        "GIT_EXEC_PATH",
        "PYTHONPATH",
        "PYTHONSTARTUP",
        "PERL5LIB",
        "PERL5OPT",
        "RUBYOPT",
        "RUBYLIB",
        "NODE_OPTIONS",
        "NODE_PATH",
        "BASH_ENV",
        "ENV",
        "LD_PRELOAD",
        "LD_ANYTHING_ELSE",
        "ld_library_path",
        "DYLD_INSERT_LIBRARIES",
        "DYLD_SOMETHING",
        "JAVA_TOOL_OPTIONS",
        "_JAVA_OPTIONS",
        "ComSpec",
        "PATHEXT",
        "ELECTRON_RUN_AS_NODE",
        "npm_config_script_shell",
    ] {
        assert!(env_key_denied(key), "{key}");
    }
    // Multi-byte keys must not panic the prefix check.
    assert!(!env_key_denied("éé"));
    assert!(!env_key_denied("L"));
}

#[test]
fn http_deny_not_bypassed_by_percent_encoded_unreserved_chars() {
    let p = policy(&[], &[], &[], &["https://api.corp/admin/**"], false);
    for url in [
        "https://api.corp/admin/x",
        "https://api.corp/%61dmin/x",
        "https://api.corp/%61%64%6d%69%6e/x",
        "https://api.corp/admin/%78",
        "https://api.corp/%2e/admin/x",
        "https://api.corp/a/%2e%2e/admin/x",
    ] {
        assert!(p.check_http(url).is_err(), "{url}");
    }
    // Deny rules fold case, so a decoded uppercase letter is still denied.
    assert!(p.check_http("https://api.corp/%41dmin/x").is_err());
}

#[test]
fn http_deny_not_bypassed_by_slashes_params_or_case() {
    let p = policy(&[], &[], &[], &["https://api.corp/admin/**"], false);
    for url in [
        "https://api.corp//admin/users",
        "https://api.corp/admin//users",
        "https://api.corp///admin///users",
        "https://api.corp/ADMIN/users",
        "https://api.corp/Admin/Users",
        "https://api.corp/admin;x/users",
        "https://api.corp/admin/users;jsessionid=1",
        "https://api.corp/admin%2Fusers",
        "https://api.corp/admin%5Cusers",
        "https://API.CORP/admin/users",
    ] {
        assert!(p.check_http(url).is_err(), "{url}");
    }
    assert!(p.check_http("https://api.corp/public/x").is_ok());
}

#[test]
fn http_allow_rules_stay_case_sensitive_and_merge_slashes() {
    let allow = policy(&[], &[], &["https://api.corp/public/**"], &[], false);
    assert!(allow.check_http("https://api.corp/public//x").is_ok());
    assert!(allow.check_http("https://api.corp//public/x").is_ok());
    assert!(allow.check_http("https://api.corp/PUBLIC/x").is_err());
    // `;` path parameters and `..;` are refused, not stripped.
    assert!(allow.check_http("https://api.corp/public/a;x").is_err());
    assert!(allow
        .check_http("https://api.corp/public/..;/admin")
        .is_err());
    // Patterns with doubled slashes behave like their collapsed form.
    let doubled = policy(&[], &[], &["https://api.corp//public//*"], &[], false);
    assert!(doubled.check_http("https://api.corp/public/x").is_ok());
    // A rule that spells `;` out opts in to it.
    let literal = policy(&[], &[], &["https://api.corp/a;v=1/*"], &[], false);
    assert!(literal.check_http("https://api.corp/a;v=1/x").is_ok());
}

#[test]
fn http_percent_encoded_slash_is_not_a_separator() {
    let p = policy(&[], &[], &[], &["https://api.corp/admin/**"], false);
    // %2F is never a separator and is refused outright once a policy is set.
    assert!(p.check_http("https://api.corp/admin%2Fx").is_err());
    let allow = policy(&[], &[], &["https://api.corp/a/*"], &[], false);
    assert!(allow.check_http("https://api.corp/a/b%2Fc").is_err());
    assert!(allow.check_http("https://api.corp/a/b/c").is_err());
    assert!(allow.check_http("https://api.corp/a/b").is_ok());
    assert_eq!(
        canonicalize_http_url("https://h/a%2fb%7e").unwrap(),
        "https://h/a%2Fb~"
    );
}

#[test]
fn http_allow_prefix_cannot_be_escaped_with_encoded_traversal() {
    let allow = policy(&[], &[], &["https://api.corp/public/*"], &[], false);
    assert!(allow.check_http("https://api.corp/public/doc").is_ok());
    for url in [
        "https://api.corp/public/%2e%2e%2fadmin",
        "https://api.corp/public/%2E%2E%2Fadmin",
        "https://api.corp/public/..%2fadmin",
        "https://api.corp/public/..%5cadmin",
        "https://api.corp/public/%2e%2e%5cadmin",
        "https://api.corp/public/%252e%252e%252fadmin",
        "https://api.corp/public/%252e%252e/admin",
        "https://api.corp/public/a%2Fb",
    ] {
        assert!(allow.check_http(url).is_err(), "{url}");
    }
    // Literal and %2e dot segments are resolved by URL parsing.
    assert!(allow
        .check_http("https://api.corp/public/%2e%2e/admin")
        .is_err());
    let wide = policy(&[], &[], &["https://api.corp/**"], &[], false);
    assert!(wide
        .check_http("https://api.corp/public/%252e%252e/x")
        .is_err());
    // A rule that spells the encoding out opts in to it.
    let literal = policy(&[], &[], &["https://api.corp/a%2Fb"], &[], false);
    assert!(literal.check_http("https://api.corp/a%2fb").is_ok());
}

#[test]
fn http_encoded_separator_opt_in_comes_from_the_matching_allow_rule() {
    // GitLab-style rule that spells %2F keeps working.
    let gitlab = policy(
        &[],
        &[],
        &["https://gitlab.corp/api/v4/projects/grp%2Fproj/**"],
        &[],
        false,
    );
    assert!(gitlab
        .check_http("https://gitlab.corp/api/v4/projects/grp%2Fproj/issues")
        .is_ok());
    // Another rule spelling %2F (or `;`) must not open the gate for a URL it
    // does not match.
    let mixed = policy(
        &[],
        &[],
        &[
            "https://gitlab.corp/api/v4/projects/grp%2Fproj/**",
            "https://api.corp/public/*",
            "https://other.corp/a;v=1/*",
        ],
        &["https://x.corp/deny%2Fme"],
        false,
    );
    assert!(mixed.check_http("https://api.corp/public/doc").is_ok());
    for url in [
        "https://api.corp/public/..%2Fadmin",
        "https://api.corp/public/..%2Fadmin;x",
        "https://api.corp/public/a;b",
    ] {
        assert!(mixed.check_http(url).is_err(), "{url}");
    }
    // A matching rule must spell every marker the candidate carries.
    let partial = policy(&[], &[], &["https://api.corp/a%2Fb/*"], &[], false);
    assert!(partial.check_http("https://api.corp/a%2Fb/c").is_ok());
    assert!(partial.check_http("https://api.corp/a%2Fb/c;x").is_err());
}

#[test]
fn http_ipv4_mapped_ipv6_hosts_match_ipv4_rules() {
    let deny = policy(&[], &[], &[], &["http://127.0.0.1/**"], false);
    for url in [
        "http://[::ffff:127.0.0.1]/x",
        "http://[::ffff:7f00:1]/x",
        "http://[::ffff:7F00:1]:80/x",
        "http://[::127.0.0.1]/x",
        "http://[0:0:0:0:0:ffff:7f00:1]/x",
    ] {
        assert!(deny.check_http(url).is_err(), "{url}");
    }
    assert_eq!(
        canonicalize_http_url("http://[::ffff:7f00:1]:8080/p").unwrap(),
        "http://127.0.0.1:8080/p"
    );
    // Real IPv6 hosts, including ::1, are left alone.
    assert_eq!(
        canonicalize_http_url("http://[::1]/p").unwrap(),
        "http://[::1]/p"
    );
    assert!(deny.check_http("http://[::1]/x").is_ok());
    assert!(deny.check_http("http://[2001:db8::1]/x").is_ok());
    // Patterns written in mapped form are mapped too.
    let deny_v6 = policy(&[], &[], &[], &["http://[::ffff:127.0.0.1]:8080/**"], false);
    assert!(deny_v6.check_http("http://127.0.0.1:8080/x").is_err());
    assert!(deny_v6.check_http("http://[::ffff:7f00:1]:8080/x").is_err());
    let allow = policy(&[], &[], &["http://127.0.0.1/**"], &[], false);
    assert!(allow.check_http("http://[::ffff:7f00:1]/x").is_ok());
}

#[test]
fn http_malformed_percent_and_nul_are_ambiguous() {
    let deny = policy(&[], &[], &[], &["https://iis.corp/admin/**"], false);
    let wide = policy(&[], &[], &["https://iis.corp/**"], &[], false);
    for url in [
        "https://iis.corp/%u0061dmin/x",
        "https://iis.corp/a%zzb",
        "https://iis.corp/a%",
        "https://iis.corp/a%4",
        "https://iis.corp/a%00b",
        "https://iis.corp/a%00",
    ] {
        assert!(wide.check_http(url).is_err(), "{url}");
    }
    assert!(deny.check_http("https://iis.corp/%u0061dmin/x").is_err());
    // A rule that spells the marker opts in, but only for that marker.
    let nul = policy(&[], &[], &["https://iis.corp/a%00*"], &[], false);
    assert!(nul.check_http("https://iis.corp/a%00b").is_ok());
    assert!(nul.check_http("https://iis.corp/a%u0062").is_err());
    let malformed = policy(&[], &[], &["https://iis.corp/%u*"], &[], false);
    assert!(malformed.check_http("https://iis.corp/%u0061").is_ok());
    assert!(malformed.check_http("https://iis.corp/a%00").is_err());
}

#[test]
fn http_well_formed_percent_escapes_still_pass() {
    let wide = policy(&[], &[], &["https://api.corp/**"], &[], false);
    for url in [
        "https://api.corp/a%20b",
        "https://api.corp/a%20b?q=1%2F2",
        "https://api.corp/search?q=100%25&r=%E2%9C%93",
        "https://api.corp/%C3%A9",
        "https://api.corp/a%2Bb",
        "https://api.corp/x?bad=%zz",
    ] {
        assert!(wide.check_http(url).is_ok(), "{url}");
    }
}

#[test]
fn http_idn_patterns_match_punycode_candidates() {
    let deny = policy(&[], &[], &[], &["https://b\u{fc}cher.example/**"], false);
    assert!(deny.check_http("https://b\u{fc}cher.example/x").is_err());
    assert!(deny.check_http("https://xn--bcher-kva.example/x").is_err());
    assert!(deny.check_http("https://B\u{dc}CHER.example/x").is_err());
    let allow = policy(
        &[],
        &[],
        &["https://*.b\u{fc}cher.example:8443/**"],
        &[],
        false,
    );
    assert!(allow
        .check_http("https://shop.b\u{fc}cher.example:8443/x")
        .is_ok());
    assert!(allow
        .check_http("https://shop.other.example:8443/x")
        .is_err());
}

#[test]
fn websocket_patterns_match_websocket_candidates() {
    let deny = policy(&[], &[], &[], &["wss://evil.test/**"], false);
    assert!(deny.check_http("wss://evil.test/socket").is_err());
    assert!(deny.check_http("https://evil.test/socket").is_err());
    let allow = policy(&[], &[], &["wss://chat.corp:443/**"], &[], false);
    assert!(allow.check_http("wss://chat.corp/room").is_ok());
    assert!(allow.check_http("ws://chat.corp/room").is_err());
    let allow = policy(&[], &[], &["ws://chat.corp/**"], &[], false);
    assert!(allow.check_http("ws://chat.corp/room").is_ok());
}

#[test]
fn http_patterns_with_non_ascii_or_spaces_match_encoded_candidates() {
    let deny = policy(&[], &[], &[], &["https://h.test/caf\u{e9}/**"], false);
    assert!(deny.check_http("https://h.test/caf%C3%A9/x").is_err());
    assert!(deny.check_http("https://h.test/caf%c3%a9/x").is_err());
    assert!(deny.check_http("https://h.test/caf\u{e9}/x").is_err());
    let allow = policy(&[], &[], &["https://h.test/my dir/*"], &[], false);
    assert!(allow.check_http("https://h.test/my%20dir/f").is_ok());
    assert!(allow.check_http("https://h.test/my dir/f").is_ok());
}

#[test]
fn fold_path_case_is_unicode_aware_for_both_sides() {
    assert_eq!(
        fold_path_case("C:/Users/\u{c9}lodie"),
        "c:/users/\u{e9}lodie"
    );
    assert!(
        globset::Glob::new(&fold_path_case("C:/Users/\u{c9}lodie/**"))
            .unwrap()
            .compile_matcher()
            .is_match(fold_path_case("C:/Users/\u{c9}lodie/Documents/a.txt"))
    );
}

#[cfg(windows)]
#[test]
fn windows_fs_policy_folds_non_ascii_case_on_both_sides() {
    let allow = policy(&["C:/Users/\u{c9}lodie/Documents/**"], &[], &[], &[], false);
    assert!(allow
        .check_fs("C:\\Users\\\u{c9}LODIE\\Documents\\a.txt")
        .is_ok());
    assert!(allow
        .check_fs("C:\\Users\\\u{c9}lodie\\Documents\\a.txt")
        .is_ok());
    let deny = policy(&[], &["C:/Users/\u{c9}lodie/secret/**"], &[], &[], false);
    assert!(deny.check_fs("C:/Users/\u{c9}lodie/secret/k").is_err());
    assert!(matches_pattern("C:/\u{c9}x/**", "c:/\u{e9}x/a"));
    assert_eq!(canonicalize_fs("C:/\u{c9}X/a"), "c:/\u{e9}x/a");
}

#[test]
fn env_denylist_covers_dotnet_bash_func_and_config_redirection() {
    for key in [
        "DOTNET_STARTUP_HOOKS",
        "dotnet_startup_hooks",
        "DOTNET_ADDITIONAL_DEPS",
        "COR_ENABLE_PROFILING",
        "COR_PROFILER",
        "COR_PROFILER_PATH",
        "CORECLR_ENABLE_PROFILING",
        "CORECLR_PROFILER_PATH_64",
        "BASH_FUNC_foo%%",
        "BASH_FUNC_x",
        "PYTHONWARNINGS",
        "BROWSER",
        "OPENSSL_CONF",
        "ZDOTDIR",
        "HOME",
        "home",
        "USERPROFILE",
        "XDG_CONFIG_HOME",
        "APPDATA",
        "LOCALAPPDATA",
    ] {
        assert!(env_key_denied(key), "{key}");
    }
    assert!(!env_key_denied("FOO"));
    assert!(!env_key_denied("HOMEBREW_NO_AUTO_UPDATE_X1"));
}
