// Copyright (c) Microsoft Corporation
// License: MIT OR Apache-2.0

//! Build script for the `wdf` crate.
//!
//! Based on the [`wdk_build::Config`] parsed from the build tree, this build
//! script will provide the `wdf` crate with `cfg` settings to conditionally
//! compile code.

fn main() -> Result<(), wdk_build::ConfigError> {
    wdk_build::configure_wdk_library_build()
}
