use crate::{PluginError, PluginResult};

/// Decode a rkyv-archived `args` buffer into an owned `T`.
pub fn rkyv_decode_args<T>(buf: &[u8]) -> PluginResult<T>
where
    T: ::rkyv::Archive,
    T::Archived: ::rkyv::Deserialize<T, ::rkyv::api::high::HighDeserializer<::rkyv::rancor::Error>>
        + for<'a> ::rkyv::bytecheck::CheckBytes<
            ::rkyv::api::high::HighValidator<'a, ::rkyv::rancor::Error>,
        >,
{
    ::rkyv::from_bytes::<T, ::rkyv::rancor::Error>(buf)
        .map_err(|e| PluginError::Custom(format!("rkyv decode: {e}")))
}

/// Encode `T` into rkyv bytes for return through [`crate::TyndPlugin::dispatch_rkyv`].
pub fn rkyv_encode_result<T>(value: &T) -> PluginResult<Vec<u8>>
where
    T: for<'a> ::rkyv::Serialize<
        ::rkyv::api::high::HighSerializer<
            ::rkyv::util::AlignedVec,
            ::rkyv::ser::allocator::ArenaHandle<'a>,
            ::rkyv::rancor::Error,
        >,
    >,
{
    ::rkyv::to_bytes::<::rkyv::rancor::Error>(value)
        .map(|b| b.to_vec())
        .map_err(|e| PluginError::Custom(format!("rkyv encode: {e}")))
}
