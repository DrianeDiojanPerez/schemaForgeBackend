//! The generated wire contract.
//!
//! It sits outside `module` on purpose: it is the boundary itself, not a
//! feature, and every module's adapter layer maps onto it rather than owning
//! part of it.

pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/schemaforge.v1.rs"));

    /// Served by the reflection service so `grpcurl` and the frontend codegen
    /// can discover the contract without a copy of the `.proto` files.
    pub const FILE_DESCRIPTOR_SET: &[u8] =
        include_bytes!(concat!(env!("OUT_DIR"), "/schemaforge_descriptor.bin"));
}
