// Copyright (c) Microsoft Corporation
// License: MIT OR Apache-2.0
//! Module that handles low-level driver packaging operations.
//! This module defines the `PackageTask` struct and its associated methods
//! for packaging driver projects.  It handles file system
//! operations and interacting with WDK tools to generate the driver package. It
//! includes functions that invoke various WDK Tools involved in signing,
//! validating, verifying and generating artefacts for the driver package.

use std::{
    io::{self, BufRead, BufReader, Read},
    ops::RangeFrom,
    path::{Path, PathBuf},
    result::Result,
};

use mockall_double::double;
use tracing::{debug, info};
use wdk_build::{CpuArchitecture, DriverConfig};

#[double]
use crate::providers::{exec::CommandExec, fs::Fs, wdk_build::WdkBuild};
use crate::{actions::build::error::PackageTaskError, providers::error::FileError};

// FIXME: This range is inclusive of 25798. Update with range end after /sample
// flag is added to InfVerif CLI
const MISSING_SAMPLE_FLAG_WDK_BUILD_NUMBER_RANGE: RangeFrom<u32> = 25798..;
const WDR_TEST_CERT_STORE: &str = "WDRTestCertStore";
const WDR_LOCAL_TEST_CERT: &str = "WDRLocalTestCert";

#[derive(Debug)]
pub struct PackageTaskParams<'a> {
    pub package_name: &'a str,
    pub working_dir: &'a Path,
    pub target_dir: &'a Path,
    pub target_arch: &'a CpuArchitecture,
    pub verify_signature: bool,
    pub driver_model: DriverConfig,
}

/// Suports low level driver packaging operations
pub struct PackageTask<'a> {
    package_name: String,
    verify_signature: bool,

    // src paths
    src_inx_file_path: PathBuf,
    src_driver_binary_file_path: PathBuf,
    src_renamed_driver_binary_file_path: PathBuf,
    src_pdb_file_path: PathBuf,
    src_map_file_path: PathBuf,
    src_cert_file_path: PathBuf,

    // destination paths
    dest_root_package_folder: PathBuf,
    dest_inf_file_path: PathBuf,
    dest_driver_binary_path: PathBuf,
    dest_pdb_file_path: PathBuf,
    dest_map_file_path: PathBuf,
    dest_cert_file_path: PathBuf,
    dest_cat_file_path: PathBuf,

    arch: &'a CpuArchitecture,
    os_mapping: &'a str,
    driver_model: DriverConfig,

    // Injected deps
    wdk_build: &'a WdkBuild,
    command_exec: &'a CommandExec,
    fs: &'a Fs,
}

impl<'a> PackageTask<'a> {
    /// Creates a new instance of `PackageTask`.
    /// # Arguments
    /// * `params` - Struct containing the parameters for the package task.
    /// * `wdk_build` - The provider for WDK build related methods.
    /// * `command_exec` - The provider for command execution.
    /// * `fs` - The provider for file system operations.
    /// # Returns
    /// * `Result<Self, PackageTaskError>` - A result containing the new
    ///   instance or an error.
    /// # Errors
    /// * `PackageTaskError::Io` - If there is an IO error while creating the
    ///   final package directory.
    pub fn new(
        params: PackageTaskParams<'a>,
        wdk_build: &'a WdkBuild,
        command_exec: &'a CommandExec,
        fs: &'a Fs,
    ) -> Result<Self, PackageTaskError> {
        debug!("Package task params: {params:?}");
        let package_name = params.package_name.replace('-', "_");
        // src paths
        let src_driver_binary_extension = "dll";
        let src_inx_file_path = params.working_dir.join(format!("{package_name}.inx"));

        // all paths inside target directory
        let src_driver_binary_file_path = params
            .target_dir
            .join(format!("{package_name}.{src_driver_binary_extension}"));
        let src_pdb_file_path = params.target_dir.join(format!("{package_name}.pdb"));
        let src_map_file_path = params
            .target_dir
            .join("deps")
            .join(format!("{package_name}.map"));
        let src_cert_file_path = params.target_dir.join(format!("{WDR_LOCAL_TEST_CERT}.cer"));

        // destination paths
        let dest_driver_binary_extension = match params.driver_model {
            DriverConfig::Kmdf(_) | DriverConfig::Wdm => "sys",
            DriverConfig::Umdf(_) => "dll",
        };

        let src_renamed_driver_binary_file_path = params
            .target_dir
            .join(format!("{package_name}.{dest_driver_binary_extension}"));
        let dest_root_package_folder: PathBuf =
            params.target_dir.join(format!("{package_name}_package"));
        let dest_inf_file_path = dest_root_package_folder.join(format!("{package_name}.inf"));
        let dest_driver_binary_path =
            dest_root_package_folder.join(format!("{package_name}.{dest_driver_binary_extension}"));
        let dest_pdb_file_path = dest_root_package_folder.join(format!("{package_name}.pdb"));
        let dest_map_file_path = dest_root_package_folder.join(format!("{package_name}.map"));
        let dest_cert_file_path =
            dest_root_package_folder.join(format!("{WDR_LOCAL_TEST_CERT}.cer"));
        let dest_cat_file_path = dest_root_package_folder.join(format!("{package_name}.cat"));

        if !fs.exists(&dest_root_package_folder) {
            fs.create_dir(&dest_root_package_folder)?;
        }
        let os_mapping = match params.target_arch {
            CpuArchitecture::Amd64 => "10_x64",
            CpuArchitecture::Arm64 => "Server10_arm64",
        };

        Ok(Self {
            package_name,
            verify_signature: params.verify_signature,
            src_inx_file_path,
            src_driver_binary_file_path,
            src_renamed_driver_binary_file_path,
            src_pdb_file_path,
            src_map_file_path,
            src_cert_file_path,
            dest_root_package_folder,
            dest_inf_file_path,
            dest_driver_binary_path,
            dest_pdb_file_path,
            dest_map_file_path,
            dest_cert_file_path,
            dest_cat_file_path,
            arch: params.target_arch,
            os_mapping,
            driver_model: params.driver_model,
            wdk_build,
            command_exec,
            fs,
        })
    }

    /// Entry point method to run the low level driver packaging operations.
    /// # Returns
    /// * `Result<(), PackageTaskError>` - A result indicating success or
    ///   failure.
    /// # Errors
    /// * `PackageTaskError::CopyFile` - If there is an error copying artifacts
    ///   to final package directory.
    /// * `PackageTaskError::CertGenerationInStoreCommand` - If there is an
    ///   error generating a certificate in the store.
    /// * `PackageTaskError::CreateCertFileFromStoreCommand` - If there is an
    ///   error creating a certificate file from the store.
    /// * `PackageTaskError::DriverBinarySignCommand` - If there is an error
    ///   signing the driver binary.
    /// * `PackageTaskError::DriverBinarySignVerificationCommand` - If there is
    ///   an error verifying the driver binary signature.
    /// * `PackageTaskError::Inf2CatCommand` - If there is an error running the
    ///   inf2cat command to generate the cat file.
    /// * `PackageTaskError::InfVerificationCommand` - If there is an error
    ///   verifying the inf file.
    /// * `PackageTaskError::MissingInxSrcFile` - If the .inx source file is
    ///   missing.
    /// * `PackageTaskError::StampinfCommand` - If there is an error running the
    ///   stampinf command to generate the inf file from the .inx template file.
    /// * `PackageTaskError::VerifyCertExistsInStoreCommand` - If there is an
    ///   error verifying if the certificate exists in the store.
    /// * `PackageTaskError::VerifyCertExistsInStoreInvalidCommandOutput`
    ///   - If the command output is invalid when verifying if the certificate
    ///     exists in the store.
    /// * `PackageTaskError::WdkBuildConfig` - If there is an error detecting
    ///   the WDK build number.
    /// * `PackageTaskError::Io` - Wraps all possible IO errors.
    pub fn run(&self) -> Result<(), PackageTaskError> {
        self.check_inx_exists()?;
        info!(
            "Copying files to target package folder: {}",
            self.dest_root_package_folder.to_string_lossy()
        );
        self.rename_driver_binary_extension()?;
        self.copy(
            &self.src_renamed_driver_binary_file_path,
            &self.dest_driver_binary_path,
        )?;
        self.copy(&self.src_pdb_file_path, &self.dest_pdb_file_path)?;
        self.copy(&self.src_inx_file_path, &self.dest_inf_file_path)?;
        self.copy(&self.src_map_file_path, &self.dest_map_file_path)?;
        self.run_stampinf()?;
        self.run_inf2cat()?;
        self.generate_certificate()?;
        self.copy(&self.src_cert_file_path, &self.dest_cert_file_path)?;
        self.run_signtool_sign(
            &self.dest_driver_binary_path,
            WDR_TEST_CERT_STORE,
            WDR_LOCAL_TEST_CERT,
        )?;
        self.run_signtool_sign(
            &self.dest_cat_file_path,
            WDR_TEST_CERT_STORE,
            WDR_LOCAL_TEST_CERT,
        )?;
        self.run_infverif()?;
        // Verify signatures only when --verify-signature flag = true is passed
        if self.verify_signature {
            info!("Verifying signatures for driver binary and cat file using signtool");
            self.run_signtool_verify(&self.dest_driver_binary_path)?;
            self.run_signtool_verify(&self.dest_cat_file_path)?;
        }
        Ok(())
    }

    fn check_inx_exists(&self) -> Result<(), PackageTaskError> {
        debug!(
            "Checking for .inx file, path: {}",
            self.src_inx_file_path.to_string_lossy()
        );
        if !self.fs.exists(&self.src_inx_file_path) {
            return Err(PackageTaskError::MissingInxSrcFile(
                self.src_inx_file_path.clone(),
            ));
        }
        Ok(())
    }

    fn rename_driver_binary_extension(&self) -> Result<(), FileError> {
        debug!("Renaming driver binary extension from .dll to .sys");
        self.fs.rename(
            &self.src_driver_binary_file_path,
            &self.src_renamed_driver_binary_file_path,
        )
    }

    fn copy(&self, src_file_path: &'a Path, dest_file_path: &'a Path) -> Result<u64, FileError> {
        debug!(
            "Copying src file {} to dest folder {}",
            src_file_path.to_string_lossy(),
            dest_file_path.to_string_lossy()
        );
        self.fs.copy(src_file_path, dest_file_path)
    }

    fn run_stampinf(&self) -> Result<(), PackageTaskError> {
        info!("Running stampinf command.");
        let wdf_version_flags = match self.driver_model {
            DriverConfig::Kmdf(kmdf_config) => {
                vec![
                    "-k".to_string(),
                    format!(
                        "{}.{}",
                        kmdf_config.kmdf_version_major, kmdf_config.target_kmdf_version_minor
                    ),
                ]
            }
            DriverConfig::Umdf(umdf_config) => vec![
                "-u".to_string(),
                format!(
                    "{}.{}.0",
                    umdf_config.umdf_version_major, umdf_config.target_umdf_version_minor
                ),
            ],
            DriverConfig::Wdm => vec![],
        };
        // TODO: Does it generate cat file relative to inf file path or we need to
        // provide the absolute path?
        let cat_file_path = format!("{}.cat", self.package_name);
        let dest_inf_file_path = self.dest_inf_file_path.to_string_lossy();
        let arch = self.arch.to_string();
        let mut args: Vec<&str> = vec![
            "-f",
            &dest_inf_file_path,
            "-d",
            "*",
            "-a",
            &arch,
            "-c",
            &cat_file_path,
            "-v",
            "*",
        ];
        if !wdf_version_flags.is_empty() {
            args.append(&mut wdf_version_flags.iter().map(String::as_str).collect());
        }
        if let Err(e) = self.command_exec.run("stampinf", &args, None) {
            return Err(PackageTaskError::StampinfCommand(e));
        }
        Ok(())
    }

    fn run_inf2cat(&self) -> Result<(), PackageTaskError> {
        info!("Running inf2cat command.");
        let args = [
            &format!(
                "/driver:{}",
                self.dest_root_package_folder
                    .to_string_lossy()
                    .trim_start_matches("\\\\?\\")
            ),
            &format!("/os:{}", self.os_mapping),
            "/uselocaltime",
        ];

        if let Err(e) = self.command_exec.run("inf2cat", &args, None) {
            return Err(PackageTaskError::Inf2CatCommand(e));
        }

        Ok(())
    }

    fn generate_certificate(&self) -> Result<(), PackageTaskError> {
        debug!("Generating certificate.");
        if self.fs.exists(&self.src_cert_file_path) {
            return Ok(());
        }
        if self.is_self_signed_certificate_in_store()? {
            self.create_cert_file_from_store()?;
        } else {
            self.create_self_signed_cert_in_store()?;
        }
        Ok(())
    }

    fn is_self_signed_certificate_in_store(&self) -> Result<bool, PackageTaskError> {
        debug!("Checking if self signed certificate exists in WDRTestCertStore store.");
        let args = ["-s", WDR_TEST_CERT_STORE];

        match self.command_exec.run("certmgr.exe", &args, None) {
            Ok(output) if output.status.success() => String::from_utf8(output.stdout).map_or_else(
                |e| Err(PackageTaskError::VerifyCertExistsInStoreInvalidCommandOutput(e)),
                |stdout| Ok(stdout.contains(WDR_LOCAL_TEST_CERT)),
            ),
            Ok(_) => Ok(false),
            Err(e) => Err(PackageTaskError::VerifyCertExistsInStoreCommand(e)),
        }
    }

    fn create_self_signed_cert_in_store(&self) -> Result<(), PackageTaskError> {
        info!("Creating self signed certificate in WDRTestCertStore store using makecert.");
        let cert_path = self.src_cert_file_path.to_string_lossy();
        let args = [
            "-r",
            "-pe",
            "-a",
            "SHA256",
            "-eku",
            "1.3.6.1.5.5.7.3.3",
            "-ss",
            WDR_TEST_CERT_STORE, // FIXME: this should be a parameter
            "-n",
            &format!("CN={WDR_LOCAL_TEST_CERT}"), // FIXME: this should be a parameter
            &cert_path,
        ];
        if let Err(e) = self.command_exec.run("makecert", &args, None) {
            return Err(PackageTaskError::CertGenerationInStoreCommand(e));
        }
        Ok(())
    }

    fn create_cert_file_from_store(&self) -> Result<(), PackageTaskError> {
        info!("Creating certificate file from WDRTestCertStore store using certmgr.");
        let cert_path = self.src_cert_file_path.to_string_lossy();
        let args = [
            "-put",
            "-s",
            WDR_TEST_CERT_STORE,
            "-c",
            "-n",
            WDR_LOCAL_TEST_CERT,
            &cert_path,
        ];
        if let Err(e) = self.command_exec.run("certmgr.exe", &args, None) {
            return Err(PackageTaskError::CreateCertFileFromStoreCommand(e));
        }
        Ok(())
    }

    /// Signs the specified file using signtool command using cerificate from
    /// certificate store.
    ///
    /// # Arguments
    ///
    /// * `file_path` - The path to the file to be signed.
    /// * `cert_store` - The certificate store to use for signing.
    /// * `cert_name` - The name of the certificate to use for signing. TODO:
    ///   Add parameters for certificate store and name
    fn run_signtool_sign(
        &self,
        file_path: &Path,
        cert_store: &str,
        cert_name: &str,
    ) -> Result<(), PackageTaskError> {
        info!(
            "Signing {} using signtool.",
            file_path
                .file_name()
                .expect("Unable to read file name from the path")
                .to_string_lossy()
        );
        let driver_binary_file_path = file_path.to_string_lossy();
        let args = [
            "sign",
            "/v",
            "/s",
            cert_store,
            "/n",
            cert_name,
            "/t",
            "http://timestamp.digicert.com",
            "/fd",
            "SHA256",
            &driver_binary_file_path,
        ];
        if let Err(e) = self.command_exec.run("signtool", &args, None) {
            return Err(PackageTaskError::DriverBinarySignCommand(e));
        }
        Ok(())
    }

    fn run_signtool_verify(&self, file_path: &Path) -> Result<(), PackageTaskError> {
        info!(
            "Verifying {} using signtool.",
            file_path
                .file_name()
                .expect("Unable to read file name from the path")
                .to_string_lossy()
        );
        let driver_binary_file_path = file_path.to_string_lossy();
        let args = ["verify", "/v", "/pa", &driver_binary_file_path];
        // TODO: Differentiate between command exec failure and signature verification
        // failure
        if let Err(e) = self.command_exec.run("signtool", &args, None) {
            return Err(PackageTaskError::DriverBinarySignVerificationCommand(e));
        }
        Ok(())
    }

    fn run_infverif(&self) -> Result<(), PackageTaskError> {
        info!("Running infverif command.");
        
        // Detect if this is a sample driver by parsing the .inx file
        let is_sample_driver = Self::inx_has_sample_class(&self.src_inx_file_path, self.fs)?;
        
        let additional_args = if is_sample_driver {
            let wdk_build_number = self.wdk_build.detect_wdk_build_number()?;
            if MISSING_SAMPLE_FLAG_WDK_BUILD_NUMBER_RANGE.contains(&wdk_build_number) {
                debug!(
                    "InfVerif in WDK Build {wdk_build_number} is bugged and does not contain the \
                     /samples flag."
                );
                info!("Skipping InfVerif for samples class. WDK Build: {wdk_build_number}");
                return Ok(());
            }
            "/msft"
        } else {
            ""
        };
        let mut args = vec![
            "/v",
            match self.driver_model {
                DriverConfig::Kmdf(_) | DriverConfig::Wdm => "/w",
                // TODO: This should be /u if WDK <= GE && DRIVER_MODEL == UMDF, otherwise it should
                // be /w
                DriverConfig::Umdf(_) => "/u",
            },
        ];
        let inf_path = self.dest_inf_file_path.to_string_lossy();

        if is_sample_driver {
            args.push(additional_args);
        }
        args.push(&inf_path);

        if let Err(e) = self.command_exec.run("infverif", &args, None) {
            return Err(PackageTaskError::InfVerificationCommand(e));
        }

        Ok(())
    }

    /// Detects if a driver is a sample class driver by parsing the .inx file
    /// and looking for "Class=Sample" value under the "[Version]" section.
    pub fn inx_has_sample_class(inx_path: &Path, fs: &Fs) -> Result<bool, PackageTaskError> {
        debug!("Detecting sample class from .inx file: {}", inx_path.display());
        
        let file = fs.open_reader(inx_path)
            .map_err(|e| PackageTaskError::FileIo(e))?;

        Self::reader_has_sample_class(file)
            .map_err(|e| PackageTaskError::FileIo(FileError::ReadError(inx_path.to_owned(), e)))
    }

    /// Parses INX file content to detect if it contains "Class=Sample" under
    /// the "[Version]" section.
    /// 
    /// This function has been extracted out for testability.
    fn reader_has_sample_class<R: Read>(reader: R) -> Result<bool, io::Error> {
        let buf_reader = BufReader::with_capacity(512, reader);
        let mut in_version_section = false;
        
        for line in buf_reader.lines() {
            let line = line?;
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with(';') {
                continue;
            }
            
            // Check for [Version] section (case-insensitive)
            if trimmed.to_lowercase() == "[version]" {
                in_version_section = true;
                debug!("Found [Version] section");
                continue;
            }
            
            // Check if we've moved to a different section
            if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.to_lowercase() != "[version]" {
                if in_version_section {
                    debug!("Left [Version] section, entering: {}", trimmed);
                }
                in_version_section = false;
                continue;
            }
            
            // If we're in the [Version] section, look for Class=Sample
            if in_version_section && trimmed.contains('=') {
                let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
                if parts.len() == 2 {
                    let key = parts[0].trim();
                    let value = parts[1].trim();
                    
                    // Case-insensitive check for "Class" and "Sample"
                    if key.to_lowercase() == "class" && value.to_lowercase() == "sample" {
                        debug!("Found Class=Sample in [Version] section");
                        return Ok(true);
                    }
                }
            }
        }
        
        debug!("Did not find Class=Sample in [Version] section");
        Ok(false)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    mod reader_has_sample_class {
        use std::result::Result;
        use super::*;

        #[test]
        fn for_inx_containing_sample_class_returns_true() {
            const SAMPLE_CLASS_INX_FILES: &[&str] = &[
                // Basic sample class
                r#"[Version]
Signature   = "$WINDOWS NT$"
Class       = Sample
ClassGuid   = {78A1C341-4539-11d3-B88D-00C04FAD5171}
Provider    = %ProviderString%"#,
                
                // Case insensitive
                r#"[version]
Signature   = "$WINDOWS NT$"
CLASS       = SAMPLE
Provider    = %ProviderString%"#,
                
                // With whitespace
                r#"[Version]
Signature   = "$WINDOWS NT$"
  Class   =   Sample  
Provider    = %ProviderString%"#,
                
                // With comments and empty lines
                r#"; This is a comment
[Version]
; Another comment
Signature   = "$WINDOWS NT$"

Class       = Sample
; Final comment
Provider    = %ProviderString%"#,
                
                // Multiple sections - only check Version section
                r#"[SomeOtherSection]
Class = NotSample

[Version]
Signature   = "$WINDOWS NT$"
Class       = Sample
Provider    = %ProviderString%

[AnotherSection]
Class = AlsoNotSample"#,
                
                // Complex real-world example
                r#";===================================================================
; Sample KMDF Driver
; Copyright (c) Microsoft Corporation
;===================================================================

[Version]
Signature   = "$WINDOWS NT$"
Class       = Sample
ClassGuid   = {78A1C341-4539-11d3-B88D-00C04FAD5171}
Provider    = %ProviderString%
PnpLockDown = 1

[DestinationDirs]
DefaultDestDir = 13

[SourceDisksNames]
1 = %DiskId1%,,,""

[SourceDisksFiles]
sample_kmdf_driver.sys = 1,,"#,
            ];

            for (i, content) in SAMPLE_CLASS_INX_FILES.iter().enumerate() {
                run_test(content, i, Ok(true));
            }
        }

        #[test]
        fn for_inx_not_containing_sample_class_returns_false() {
            const NON_SAMPLE_CLASS_CONTENT: &[&str] = &[
                // Different class name
                r#"[Version]
Signature   = "$WINDOWS NT$"
Class       = Custom Sample Device Class
ClassGuid   = {C5D55F57-9A34-4E34-B1A0-8A10BDE938D6}
Provider    = %ManufacturerName%"#,
                
                // No version section
                r#"[SomeSection]
Signature   = "$WINDOWS NT$"
Class       = Sample
Provider    = %ProviderString%"#,
                
                // No class in version
                r#"[Version]
Signature   = "$WINDOWS NT$"
Provider    = %ProviderString%"#,
                
                // Empty content
                "",
                
                // Only comments
                r#";Only comments
; No actual content
; Just comments everywhere"#,
                
                // Class without value
                r#"[Version]
Signature   = "$WINDOWS NT$"
Class =
Provider    = %ProviderString%"#,
                
                // Class with 'Sample' as substring but not exact match
                r#"[Version]
Signature   = "$WINDOWS NT$"
Class = SampleDevice
Provider    = %ProviderString%"#,
                
                // Complex non-sample example
                r#";
; kmdf_driver.inf
;

[Version]
Signature   = "$WINDOWS NT$"
Class       = Custom Sample Device Class
ClassGuid   = {C5D55F57-9A34-4E34-B1A0-8A10BDE938D6}
Provider    = %ManufacturerName%
CatalogFile = kmdf_driver.cat
DriverVer   = ; TODO: set DriverVer in stampinf property pages
PnpLockdown = 1"#,
            ];

            for (i, content) in NON_SAMPLE_CLASS_CONTENT.iter().enumerate() {
                run_test(content, i, Ok(false));
            }
        }

        #[test]
        fn for_malformed_inx_returns_false() {
            const MALFORMED_CONTENT: &[&str] = &[
                // Malformed key-value but valid Class=Sample later
                r#"[Version]
Signature   = "$WINDOWS NT$"
Class
Provider    = %ProviderString%
Class = Sample"#,
                
                // Multiple equals signs (should not match because value becomes "Sample = Extra")
                r#"[Version]
Signature   = "$WINDOWS NT$"
Class = Sample = Extra
Provider    = %ProviderString%"#,
                
                // Nested brackets (malformed but should still parse)
                r#"[Version]
Signature   = "$WINDOWS NT$"
Class = Sample
[Nested]
Provider    = %ProviderString%"#,
                
                // Missing closing bracket
                r#"[Version
Signature   = "$WINDOWS NT$"
Class = Sample
Provider    = %ProviderString%"#,
                
                // Multiple Version sections (should use first one)
                r#"[Version]
Class = Sample

[Version]
Class = NotSample"#,
            ];

            // For malformed content, we expect specific behaviors:
            let expected_results = [
                true,  // Should find valid Class=Sample despite malformed line
                false, // Multiple equals should not match
                true,  // Should still find Class=Sample despite nested section
                true,  // Should still find Class=Sample despite malformed section
                true,  // Should use first Version section
            ];

            for (i, (content, expected)) in MALFORMED_CONTENT.iter().zip(expected_results.iter()).enumerate() {
                run_test(content, i, Ok(*expected));
            }
        }

        fn run_test(content: &str, i: usize, expected: Result<bool, io::Error>) {
            let reader = std::io::Cursor::new(content.as_bytes());
            let result = PackageTask::reader_has_sample_class(reader);
            assert!(
                are_eq(&result, &expected),
                "Expected {:?}, got {:?}. Test case: {}, content:\n{}",
                expected,
                result,
                i,
                content
            );

            fn are_eq(res1: &Result<bool, io::Error>, res2: &Result<bool, io::Error>) -> bool {
                match (res1, res2) {
                    (Ok(v1), Ok(v2)) => v1 == v2,
                    (Err(e1), Err(e2)) => e1.kind() == e2.kind() && e1.to_string() == e2.to_string(),
                    _ => false,
                }
            }
        }
    }
}