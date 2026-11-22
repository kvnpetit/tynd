use std::str::FromStr;

use nutype::nutype;

/// Window label as set by the backend / frontend (`"main"`, `"settings"`).
/// Must start with a letter, then up to 63 letters/digits/`_`/`-`.
#[nutype(
    sanitize(trim),
    validate(not_empty, regex = r"^[a-zA-Z][a-zA-Z0-9_-]{0,63}$"),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        Display,
        AsRef,
        Serialize,
        Deserialize
    )
)]
pub struct WindowLabel(String);

/// Plugin name (`"fs"`, `"vault"`, `"http-server"`). Lowercase kebab.
#[nutype(
    sanitize(trim, lowercase),
    validate(not_empty, regex = r"^[a-z][a-z0-9-]{0,30}$"),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        Display,
        AsRef,
        Serialize,
        Deserialize
    )
)]
pub struct PluginName(String);

/// Plugin method name. Mixed case allowed (snake/camel/kebab) up to 64 chars,
/// must start with a letter or `_`.
#[nutype(
    sanitize(trim),
    validate(not_empty, regex = r"^[A-Za-z_][A-Za-z0-9_-]{0,63}$"),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        Display,
        AsRef,
        Serialize,
        Deserialize
    )
)]
pub struct MethodName(String);

/// OS API namespace (`"window"`, `"tray"`, `"menu"`, `"fs"`). camelCase.
#[nutype(
    sanitize(trim, lowercase),
    validate(not_empty, regex = r"^[a-z][a-zA-Z0-9]*$"),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        Display,
        AsRef,
        Serialize,
        Deserialize
    )
)]
pub struct ApiName(String);

/// Capability identifier: kebab-case, starts with a letter.
#[nutype(
    sanitize(trim, lowercase),
    validate(not_empty, regex = r"^[a-z][a-z0-9-]{0,63}$"),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        Display,
        AsRef,
        Serialize,
        Deserialize
    )
)]
pub struct CapabilityId(String);

/// Event name forwarded to the frontend bus (`"oauth:callback"`).
#[nutype(
    sanitize(trim),
    validate(not_empty, len_char_max = 256),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        Display,
        AsRef,
        Serialize,
        Deserialize
    )
)]
pub struct EventName(String);

/// Parsed `"plugin:method"` capability command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub plugin: PluginName,
    pub method: MethodName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandParseError(pub String);

impl std::fmt::Display for CommandParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CommandParseError {}

impl FromStr for Command {
    type Err = CommandParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (plugin, method) = s
            .split_once(':')
            .ok_or_else(|| CommandParseError(format!("missing ':' in command '{s}'")))?;
        let plugin = PluginName::try_new(plugin)
            .map_err(|e| CommandParseError(format!("bad plugin '{plugin}': {e}")))?;
        let method = MethodName::try_new(method)
            .map_err(|e| CommandParseError(format!("bad method '{method}': {e}")))?;
        Ok(Self { plugin, method })
    }
}
