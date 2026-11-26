//! Security authorities of the host: the capability ACL, the security policy
//! (fs / network / shell rules) and the path-scope checks behind them.

pub(crate) mod acl;
pub(crate) mod grants;
pub(crate) mod path_scope;
pub(crate) mod policy;
pub(crate) mod sensitive;
