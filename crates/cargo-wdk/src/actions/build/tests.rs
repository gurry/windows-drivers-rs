// Copyright (c) Microsoft Corporation
// License: MIT OR Apache-2.0
#![allow(clippy::too_many_lines)] // Package tests are longer and splitting them into sub functions can make the code less readable
#![allow(clippy::ref_option_ref)] // This is suppressed for mockall as it generates mocks with env_vars: &Option
use std::{
    collections::HashMap,
    os::windows::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{ExitStatus, Output},
    result::Result::Ok,
};

use mockall::predicate::eq;
use mockall_double::double;
use wdk_build::{
    metadata::{TryFromCargoMetadataError, Wdk},
    CpuArchitecture,
    DriverConfig,
};

#[double]
use crate::providers::{
    exec::CommandExec,
    fs::Fs,
    metadata::Metadata as MetadataProvider,
    wdk_build::WdkBuild,
};
use crate::{
    actions::{
        build::{BuildAction, BuildActionError, BuildActionParams},
        to_target_triple,
        Profile,
        TargetArch,
    },
    providers::error::{CommandError, FileError},
};

////////////////////////////////////////////////////////////////////////////////
/// Standalone driver project tests
////////////////////////////////////////////////////////////////////////////////
mod standalone_project {
    use super::*;

    #[test]
    pub fn for_default_args_build_succeeds() {
        test_successful_build(|_| {});
    }

    #[test]
    pub fn for_release_profile_build_succeeds() {
        test_successful_build(|c| c.build_args.profile = Some(Profile::Release));
    }

    #[test]
    pub fn for_target_arch_arm64_build_succeeds() {
        test_successful_build(|c| {
            c.build_args.target_arch = TargetArch::Selected(CpuArchitecture::Arm64)
        });
    }

    #[test]
    pub fn for_release_profile_and_target_arch_arm64_build_succeeds() {
        test_successful_build(|c| {
            c.build_args.profile = Some(Profile::Release);
            c.build_args.target_arch = TargetArch::Selected(CpuArchitecture::Arm64);
        });
    }

    #[test]
    pub fn for_sample_class_true_build_succeeds() {
        test_successful_build(|c| {
            c.build_args.profile = Some(Profile::Release);
            c.build_args.sample_class = true;
        });
    }

    #[test]
    pub fn for_verify_signature_false_build_succeeds() {
        test_successful_build(|c| c.build_args.verify_signature = true);
    }

    #[test]
    pub fn if_self_signed_cert_exists_build_succeeds() {
        test_successful_build(|c| {
            c.project.as_standalone_package().cert_status = CertStatus::ExistsInPackageDir
        });
    }

    #[test]
    pub fn if_final_package_dir_exists_build_succeeds() {
        test_successful_build(|c| c.project.as_standalone_package().package_dir_exists = true);
    }

    #[test]
    pub fn if_inx_file_is_missing_build_fails() {
        test_failed_build(|c| c.project.as_standalone_package().inx_file_exists = false);
    }

    #[test]
    pub fn if_copy_operation_fails_build_fails() {
        test_command_failure(Command::Copy);
    }

    #[test]
    pub fn if_stampinf_fails_build_fails() {
        test_command_failure(Command::StampInf);
    }

    #[test]
    pub fn if_inf2cat_fails_build_fails() {
        test_command_failure(Command::Inf2Cat);
    }

    #[test]
    pub fn if_certmgr_fails_build_fails() {
        test_command_failure(Command::CertMgr);
    }

    #[test]
    pub fn if_makecert_fails_build_fails() {
        test_command_failure(Command::MakeCert);
    }

    #[test]
    pub fn if_signtool_fails_build_fails() {
        test_command_failure(Command::SignTool);
    }

    #[test]
    pub fn if_infverif_fails_build_fails() {
        test_command_failure(Command::InfVerif);
    }

    #[test]
    pub fn for_non_driver_project_with_no_wdk_metadata_build_succeeds() {
        test_successful_build(|c| c.project.as_standalone_package().wdk_metadata = None);
    }

    #[test]
    pub fn for_project_with_partial_wdk_metadata_build_fails() {
        let mut context = TestContext::create_for_standalone_project();
        context.project = Project::RawCargoMetadata(invalid_driver_cargo_metadata());
        context.build_args.cwd = PathBuf::from("C:\\tmp\\sample-driver");
        context.set_expectations();

        let run_result = run_build_action(context);

        assert!(matches!(
            run_result.as_ref().expect_err("expected error"),
            BuildActionError::WdkMetadataParse(
                TryFromCargoMetadataError::WdkMetadataDeserialization {
                    metadata_source: _,
                    error_source: _
                }
            )
        ));
    }

    fn test_successful_build<F: FnMut(&mut TestContext)>(mut modify_context: F) {
        let mut context = TestContext::create_for_standalone_project();
        modify_context(&mut context);
        context.set_expectations();

        let run_result = run_build_action(context);

        assert!(run_result.is_ok());
    }

    fn test_failed_build<F: FnMut(&mut TestContext)>(mut modify_context: F) {
        let mut context = TestContext::create_for_standalone_project();
        modify_context(&mut context);
        context.set_expectations();

        let run_result = run_build_action(context);

        assert!(matches!(
            run_result.as_ref().expect_err("expected error"),
            BuildActionError::OneOrMoreWorkspaceMembersFailedToBuild(_)
        ));
    }

    fn test_command_failure(failing_command: Command) {
        test_failed_build(|c| {
            c.project.as_standalone_package().failing_command = Some(failing_command.clone())
        });
    }
}
////////////////////////////////////////////////////////////////////////////////
/// Workspace tests
////////////////////////////////////////////////////////////////////////////////
mod workspace {
    use super::*;

    mod mix_of_driver_and_non_driver_members {
        use super::*;

        #[test]
        pub fn for_default_args_build_succeeds() {
            test_successful_build(|_| {});
        }

        #[test]
        pub fn if_cwd_is_a_driver_member_only_that_member_is_built() {
            let driver_member_index = 0;
            test_successful_build_of_member(driver_member_index, |c| {
                let driver_member_root_dir = c.project.as_workspace().members[driver_member_index]
                    .root_dir
                    .clone();
                c.build_args.cwd = driver_member_root_dir
            });
        }

        #[test]
        pub fn if_cwd_is_a_non_driver_member_only_that_member_is_built() {
            let non_driver_member_index = 2; // The third member is a non driver crate
            test_successful_build_of_member(non_driver_member_index, |c| {
                let non_driver_member_root_dir = c.project.as_workspace().members
                    [non_driver_member_index]
                    .root_dir
                    .clone();
                c.build_args.cwd = non_driver_member_root_dir
            });
        }

        #[test]
        pub fn if_verify_signature_is_false_verify_tasks_are_skipped() {
            test_successful_build(|c| {
                c.build_args.verify_signature = false;
            });
        }

        #[test]
        pub fn if_two_workspace_members_have_different_wdk_configs_build_fails() {
            let mut context = TestContext::create_for_workspace();
            let wdk_metadata_1 = WdkMetadata::new("KMDF", (1, 33));
            let wdk_metadata_2 = WdkMetadata::new("UMDF", (2, 33));

            context.project.as_workspace().members[0].wdk_metadata = Some(wdk_metadata_1);
            context.project.as_workspace().members[1].wdk_metadata = Some(wdk_metadata_2);

            context.set_base_expectations();

            for member in context.project.as_workspace().members.clone().iter() {
                context.expect_cargo_build(&member.name, &member.root_dir, None);
            }

            let run_result = run_build_action(context);

            assert!(matches!(
                run_result.expect_err("expected error"),
                BuildActionError::WdkMetadataParse(
                    TryFromCargoMetadataError::MultipleWdkConfigurationsDetected {
                        wdk_metadata_configurations: _
                    }
                )
            ));
        }

        #[test]
        pub fn if_workspace_root_and_a_member_have_different_wdk_configs_build_fails() {
            let mut context = TestContext::create_for_workspace();
            let different_root_metadata = WdkMetadata::new("UMDF", (2, 33));

            context.project.as_workspace().wdk_metadata = Some(different_root_metadata);
            let workspace_root_dir = context.project.as_workspace().root_dir.clone();

            context.set_base_expectations();

            for member in context.project.as_workspace().members.clone().iter() {
                context
                    .expect_package_dir_exists(&member.name, &workspace_root_dir, false)
                    .expect_cargo_build(&member.name, &member.root_dir, None);
            }

            let run_result = run_build_action(context);

            println!("Run result: {:?}", run_result);

            assert!(matches!(
                run_result.as_ref().expect_err("expected error"),
                BuildActionError::WdkMetadataParse(
                    TryFromCargoMetadataError::MultipleWdkConfigurationsDetected {
                        wdk_metadata_configurations: _
                    }
                )
            ));
        }
    }

    mod only_non_driver_members {
        use super::*;

        #[test]
        pub fn if_cwd_is_workspace_root_build_succeeds() {
            test_successful_build(|c| {
                c.project = Project::Workspace(Workspace::create_for_only_non_drivers());
            });
        }

        #[test]
        pub fn if_cwd_is_a_member_only_that_member_is_built() {
            let first_member_index = 0;
            test_successful_build_of_member(first_member_index, |c| {
                c.project = Project::Workspace(Workspace::create_for_only_non_drivers());
                let member_root_dir = c.project.as_workspace().members[first_member_index]
                    .root_dir
                    .clone();
                c.build_args.cwd = member_root_dir
            });
        }
    }

    fn test_successful_build<F: FnMut(&mut TestContext)>(modify_context: F) {
        let member_indexes = [0, 1, 2];
        test_successful_build_of_members(&member_indexes, modify_context);
    }

    fn test_successful_build_of_member<F: FnMut(&mut TestContext)>(
        member_index: usize,
        modify_context: F,
    ) {
        test_successful_build_of_members(&[member_index], modify_context);
    }

    fn test_successful_build_of_members<F: FnMut(&mut TestContext)>(
        member_indexes: &[usize],
        mut modify_context: F,
    ) {
        let mut context = TestContext::create_for_workspace();
        modify_context(&mut context);
        context.set_workspace_expectations(member_indexes);

        let run_result = run_build_action(context);
        assert!(run_result.is_ok());
    }
}

////////////////////////////////////////////////////////////////////////////////
/// Helper functions
////////////////////////////////////////////////////////////////////////////////
struct TestContext {
    project: Project,
    build_args: BuildArgs,

    // mocks
    mock_run_command: CommandExec,
    mock_wdk_build_provider: WdkBuild,
    mock_fs_provider: Fs,
    mock_metadata_provider: MetadataProvider,
}

const WDK_BUILD_NUMBER: u32 = 25100;

// Presence of method ensures specific mock expectation is set
// Dir argument in any method means to operate on the relevant dir
// Output argument in any method means to override return output from default
// success with no stdout/stderr
impl TestContext {
    fn create_for_standalone_project() -> Self {
        Self::create_for_project(Project::Standalone(Package::default()))
    }

    fn create_for_workspace() -> Self {
        Self::create_for_project(Project::Workspace(Workspace::default()))
    }

    fn create_for_project(project: Project) -> Self {
        let mut context = Self {
            project,
            build_args: BuildArgs::default(),
            mock_run_command: CommandExec::default(),
            mock_wdk_build_provider: WdkBuild::default(),
            mock_fs_provider: Fs::default(),
            mock_metadata_provider: MetadataProvider::default(),
        };

        context
            .mock_fs_provider
            .expect_canonicalize_path()
            .returning(move |input| Ok(input.to_path_buf()));

        context
    }

    fn target_dir(&self, crate_root_path: &Path) -> PathBuf {
        let mut target_dir = crate_root_path.join("target");

        if let TargetArch::Selected(target_arch) = self.build_args.target_arch {
            target_dir = target_dir.join(to_target_triple(target_arch));
        }

        target_dir = match self.build_args.profile {
            Some(Profile::Release) => target_dir.join("release"),
            _ => target_dir.join("debug"),
        };
        target_dir
    }

    fn set_expectations(&mut self) -> &mut Self {
        match self.project {
            Project::Standalone(_) => self.set_standalone_package_expectations(),
            Project::Workspace(_) => self.set_workspace_expectations(&[1, 2, 3]),
            Project::RawCargoMetadata(_) => self.set_raw_cargo_metadata_expectations(),
        }
    }

    fn set_standalone_package_expectations(&mut self) -> &mut Self {
        self.set_base_expectations();

        let package = self.project.as_standalone_package().clone();
        self.set_package_expectations(&package, None)
    }

    fn set_workspace_expectations(&mut self, member_indexes: &[usize]) -> &mut Self {
        self.set_base_expectations();

        let members = self.project.as_workspace().members.clone();
        let workspace_root = self.project.as_workspace().root_dir.clone();
        for index in member_indexes.iter() {
            if let Some(package) = members.get(*index) {
                self.set_package_expectations(package, Some(&workspace_root));
            } else {
                panic!("Member index {} out of bounds for workspace members", index);
            }
        }

        self
    }

    fn set_raw_cargo_metadata_expectations(&mut self) -> &mut Self {
        self.set_base_expectations();

        let cargo_metadata = self.project.to_cargo_metadata();
        let root_dir = cargo_metadata.workspace_root.as_std_path().to_owned();
        let package_name = cargo_metadata.packages.first().unwrap().name.clone();

        self.expect_cargo_build(&package_name, &root_dir, None)
    }

    fn set_base_expectations(&mut self) -> &mut Self {
        let cwd = &self.build_args.cwd.clone();
        self.expect_get_cargo_metadata()
            .expect_detect_wdk_build_number()
            .expect_root_manifest_exists(cwd, true)
            .expect_create_dir()
    }

    fn set_package_expectations(
        &mut self,
        package: &Package,
        workspace_root: Option<&Path>,
    ) -> &mut Self {
        self.expect_cargo_build(&package.name, &package.root_dir, None);

        let Some(ref wdk_metadata) = package.wdk_metadata else {
            return self;
        };

        let target_dir_parent = workspace_root.unwrap_or(&package.root_dir);
        self.expect_package_dir_exists(
            &package.name,
            target_dir_parent,
            package.package_dir_exists,
        );

        self.expect_inx_file_exists(&package.name, &package.root_dir, package.inx_file_exists);

        if !package.inx_file_exists {
            return self;
        }

        self.expect_rename_driver_binary_dll_to_sys(&package.name, target_dir_parent);

        let copy_fails = package.should_fail(Command::Copy);
        self.expect_copy_driver_binary_sys_to_package_folder(
            &package.name,
            target_dir_parent,
            !copy_fails,
        );

        if copy_fails {
            return self;
        }

        self.expect_copy_pdb_file_to_package_folder(&package.name, target_dir_parent, true)
            .expect_copy_inx_file_to_package_folder(
                &package.name,
                &package.root_dir,
                true,
                target_dir_parent,
            )
            .expect_copy_map_file_to_package_folder(&package.name, target_dir_parent, true);

        fn to_output(command_fails: bool) -> Option<Output> {
            if command_fails {
                Some(failure_output())
            } else {
                None
            }
        }

        let stampinf_fails = package.should_fail(Command::StampInf);
        self.expect_stampinf(&package.name, target_dir_parent, to_output(stampinf_fails));

        if stampinf_fails {
            return self;
        }

        let inf2cat_fails = package.should_fail(Command::Inf2Cat);
        self.expect_inf2cat(&package.name, target_dir_parent, to_output(inf2cat_fails));

        if inf2cat_fails {
            return self;
        }

        match package.cert_status {
            CertStatus::ExistsInPackageDir => {
                self.expect_cert_file_exists(target_dir_parent, true);
            }
            CertStatus::ExistsInStore => {
                self.expect_cert_file_exists(target_dir_parent, false);

                let certmgr_fails = package.should_fail(Command::CertMgr);
                let output = if certmgr_fails {
                    failure_output()
                } else {
                    certmgr_output_cert_exists()
                };

                self.expect_certmgr_cert_exists_in_store(Some(output));

                if certmgr_fails {
                    return self;
                }

                self.expect_certmgr_create_cert_from_store(target_dir_parent, None);
            }
            CertStatus::DoesNotExist => {
                self.expect_cert_file_exists(target_dir_parent, false);

                let certmgr_fails = package.should_fail(Command::CertMgr);
                let output = if certmgr_fails {
                    failure_output()
                } else {
                    certmgr_output_no_certs()
                };

                self.expect_certmgr_cert_exists_in_store(Some(output));

                if certmgr_fails {
                    return self;
                }

                let makecert_fails = package.should_fail(Command::MakeCert);

                if makecert_fails {
                    self.expect_makecert_generate_new_cert(
                        target_dir_parent,
                        to_output(makecert_fails),
                    );
                    return self;
                } else {
                    self.expect_makecert_generate_new_cert(target_dir_parent, None);
                }
            }
        };

        self.expect_copy_self_signed_cert_file_to_package_folder(
            &package.name,
            target_dir_parent,
            true,
        );

        let signtool_fails = package.should_fail(Command::SignTool);

        self.expect_signtool_sign_driver_binary_sys_file(
            &package.name,
            target_dir_parent,
            to_output(signtool_fails),
        );

        if signtool_fails {
            return self;
        }

        self.expect_signtool_sign_cat_file(&package.name, target_dir_parent, None);

        let infverif_fails = package.should_fail(Command::InfVerif);
        self.expect_infverif(
            &package.name,
            target_dir_parent,
            &wdk_metadata.driver_type,
            to_output(infverif_fails),
        );

        if infverif_fails {
            return self;
        }

        if self.build_args.verify_signature {
            self.expect_signtool_verify_driver_binary_sys_file(
                &package.name,
                target_dir_parent,
                None,
            )
            .expect_signtool_verify_cat_file(&package.name, target_dir_parent, None);
        }

        self
    }

    fn expect_get_cargo_metadata(&mut self) -> &mut Self {
        let cargo_metadata = self.project.to_cargo_metadata();
        self.mock_metadata_provider
            .expect_get_cargo_metadata_at_path()
            .once()
            .returning(move |_| Ok(cargo_metadata.clone()));
        self
    }

    fn expect_root_manifest_exists(&mut self, root_dir: &Path, exists: bool) -> &mut Self {
        self.expect_path_exists(&root_dir.join("Cargo.toml"), exists)
    }

    fn expect_cert_file_exists(&mut self, driver_dir: &Path, exists: bool) -> &mut Self {
        let target_dir = self.target_dir(driver_dir);
        let src_driver_cert_path = target_dir.join("WDRLocalTestCert.cer");
        self.expect_path_exists(&src_driver_cert_path, exists)
    }

    fn expect_package_dir_exists(
        &mut self,
        driver_name: &str,
        cwd: &Path,
        exists: bool,
    ) -> &mut Self {
        let (_, package_dir) = self.normalized_name_and_package_dir(driver_name, cwd);
        self.expect_path_exists(&package_dir, exists)
    }

    fn expect_inx_file_exists(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        exists: bool,
    ) -> &mut Self {
        let driver_name = self.normalize(driver_name);
        let inx_file_path = driver_dir.join(format!("{driver_name}.inx"));
        self.expect_path_exists(&inx_file_path, exists)
    }

    fn expect_path_exists(&mut self, path: &Path, exists: bool) -> &mut Self {
        self.mock_fs_provider
            .expect_exists()
            .with(eq(path.to_owned()))
            .returning(move |_| exists);
        self
    }

    fn expect_create_dir(&mut self) -> &mut Self {
        self.mock_fs_provider
            .expect_create_dir()
            .returning(move |_| Ok(()));
        self
    }

    fn expect_rename_driver_binary_dll_to_sys(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
    ) -> &mut Self {
        let driver_name = self.normalize(driver_name);
        let target_dir = self.target_dir(driver_dir);
        let src_driver_dll_path = target_dir.join(format!("{driver_name}.dll"));
        let src_driver_sys_path = target_dir.join(format!("{driver_name}.sys"));
        self.mock_fs_provider
            .expect_rename()
            .with(eq(src_driver_dll_path), eq(src_driver_sys_path))
            .once()
            .returning(|_, _| Ok(()));
        self
    }

    fn expect_copy_driver_binary_sys_to_package_folder(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        is_success: bool,
    ) -> &mut Self {
        self.expect_copy_ext_from_target_to_package_dir(driver_name, driver_dir, "sys", is_success)
    }

    fn expect_copy_pdb_file_to_package_folder(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        is_success: bool,
    ) -> &mut Self {
        self.expect_copy_ext_from_target_to_package_dir(driver_name, driver_dir, "pdb", is_success)
    }

    fn expect_copy_inx_file_to_package_folder(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        is_success: bool,
        workspace_root_dir: &Path,
    ) -> &mut Self {
        let driver_name = self.normalize(driver_name);
        let target_dir = self.target_dir(workspace_root_dir);
        let package_dir = target_dir.join(format!("{driver_name}_package"));
        let src_path = driver_dir.join(format!("{driver_name}.inx"));
        let dest_path = package_dir.join(format!("{driver_name}.inf"));

        self.expect_copy(src_path, dest_path, is_success)
    }

    fn expect_copy_map_file_to_package_folder(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        is_success: bool,
    ) -> &mut Self {
        let target_dir = self.target_dir(driver_dir);
        let driver_name = self.normalize(driver_name);
        let src_path_in_target_dir = PathBuf::from(format!("deps/{driver_name}.map"));
        let dest_file_name = format!("{driver_name}.map");
        self.expect_copy_from_target_to_package_dir(
            &driver_name,
            &target_dir,
            &src_path_in_target_dir,
            &dest_file_name,
            is_success,
        )
    }

    fn expect_copy_self_signed_cert_file_to_package_folder(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        is_success: bool,
    ) -> &mut Self {
        let target_dir = self.target_dir(driver_dir);
        let driver_name = self.normalize(driver_name);
        let cert_file_name = "WDRLocalTestCert.cer";
        let src_path_in_target_dir = PathBuf::from(cert_file_name);
        self.expect_copy_from_target_to_package_dir(
            &driver_name,
            &target_dir,
            &src_path_in_target_dir,
            cert_file_name,
            is_success,
        )
    }

    /// Sets expectation that a file with a given extension is copied from
    /// target to package dir
    fn expect_copy_ext_from_target_to_package_dir(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        ext: &str,
        is_success: bool,
    ) -> &mut Self {
        let driver_name = self.normalize(driver_name);
        let target_dir = self.target_dir(driver_dir);
        let package_dir = target_dir.join(format!("{driver_name}_package"));
        let file_name = format!("{driver_name}.{ext}");
        let src_path = target_dir.join(&file_name);
        let dest_path = package_dir.join(&file_name);

        self.expect_copy(src_path.to_owned(), dest_path, is_success)
    }

    /// Sets expectation that the given file is copied from target directory to
    /// package directory
    fn expect_copy_from_target_to_package_dir(
        &mut self,
        driver_name: &str,
        target_dir: &Path,
        src_path_in_target_dir: &Path,
        dest_file_name: &str,
        is_success: bool,
    ) -> &mut Self {
        let package_dir = target_dir.join(format!("{driver_name}_package"));
        let src_path = target_dir.join(src_path_in_target_dir);
        let dest_path = package_dir.join(dest_file_name);
        self.expect_copy(src_path.to_owned(), dest_path, is_success)
    }

    /// Sets expectation that a file at given source path is copied to the given
    /// dest path
    fn expect_copy(
        &mut self,
        source_path: PathBuf,
        dest_path: PathBuf,
        is_success: bool,
    ) -> &mut Self {
        let bytes_copied = 1000u64;

        self.mock_fs_provider
            .expect_copy()
            .with(eq(source_path.clone()), eq(dest_path.clone()))
            .once()
            .returning(move |_, _| {
                if is_success {
                    Ok(bytes_copied)
                } else {
                    Err(FileError::CopyError(
                        source_path.clone(),
                        dest_path.clone(),
                        std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "copy error"),
                    ))
                }
            });

        self
    }

    fn expect_cargo_build(
        &mut self,
        driver_name: &str,
        cwd: &Path,
        output: Option<Output>,
    ) -> &mut Self {
        // cargo build on the package
        let manifest_path = cwd
            .join("Cargo.toml")
            .to_string_lossy()
            .trim_start_matches("\\\\?\\")
            .to_string();

        let mut args = vec![
            "build",
            "-p",
            &driver_name,
            "--manifest-path",
            &manifest_path,
        ]
        .into_iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>();

        if let Some(profile) = self.build_args.profile {
            args.push("--profile".to_string());
            args.push(profile.to_string());
        }

        if let TargetArch::Selected(target_arch) = self.build_args.target_arch {
            args.push("--target".to_string());
            args.push(to_target_triple(target_arch));
        }

        args.push("-v".to_string());
        let output = output.map_or_else(
            || Output {
                status: ExitStatus::default(),
                stdout: vec![],
                stderr: vec![],
            },
            |output| output,
        );

        self.expect_run_command("cargo", args, Some(output))
    }

    fn expect_stampinf(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        output: Option<Output>,
    ) -> &mut Self {
        // Run stampinf command
        let (driver_name, expected_final_package_dir_path) =
            self.normalized_name_and_package_dir(driver_name, driver_dir);
        let dest_driver_inf_path =
            expected_final_package_dir_path.join(format!("{driver_name}.inf"));

        let cargo_metadata = self.project.to_cargo_metadata();
        let wdk_metadata = Wdk::try_from(&cargo_metadata).unwrap();

        let target_arch = match self.build_args.target_arch {
            TargetArch::Default(target_arch) | TargetArch::Selected(target_arch) => target_arch,
        };

        if let DriverConfig::Kmdf(kmdf_config) = wdk_metadata.driver_model {
            let cat_file_name = format!("{driver_name}.cat");
            self.expect_run_command(
                "stampinf",
                vec![
                    "-f".to_string(),
                    dest_driver_inf_path.to_string_lossy().to_string(),
                    "-d".to_string(),
                    "*".to_string(),
                    "-a".to_string(),
                    target_arch.to_string(),
                    "-c".to_string(),
                    cat_file_name,
                    "-v".to_string(),
                    "*".to_string(),
                    "-k".to_string(),
                    format!(
                        "{}.{}",
                        kmdf_config.kmdf_version_major, kmdf_config.target_kmdf_version_minor
                    ),
                ],
                output,
            )
        } else {
            self
        }
    }

    fn expect_inf2cat(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        output: Option<Output>,
    ) -> &mut Self {
        // Run inf2cat command
        let (_, package_dir) = self.normalized_name_and_package_dir(driver_name, driver_dir);
        let target_arch = match self.build_args.target_arch {
            TargetArch::Default(target_arch) | TargetArch::Selected(target_arch) => target_arch,
        };

        let os = match target_arch {
            CpuArchitecture::Amd64 => "10_x64",
            CpuArchitecture::Arm64 => "Server10_arm64",
        };

        self.expect_run_command(
            "inf2cat",
            vec![
                format!("/driver:{}", package_dir.to_string_lossy()),
                format!("/os:{}", os),
                "/uselocaltime".to_string(),
            ],
            output,
        )
    }

    fn expect_certmgr_cert_exists_in_store(&mut self, output: Option<Output>) -> &mut Self {
        // check for cert in cert store using certmgr
        self.expect_run_command(
            "certmgr.exe",
            vec!["-s".to_string(), "WDRTestCertStore".to_string()],
            output,
        )
    }

    fn expect_certmgr_create_cert_from_store(
        &mut self,
        driver_dir: &Path,
        output: Option<Output>,
    ) -> &mut Self {
        // create cert from store using certmgr
        let target_dir = self.target_dir(driver_dir);
        let self_signed_cert_file_path = target_dir.join("WDRLocalTestCert.cer");

        self.expect_run_command(
            "certmgr.exe",
            vec![
                "-put".to_string(),
                "-s".to_string(),
                "WDRTestCertStore".to_string(),
                "-c".to_string(),
                "-n".to_string(),
                "WDRLocalTestCert".to_string(),
                self_signed_cert_file_path.to_string_lossy().to_string(),
            ],
            output,
        )
    }

    fn expect_makecert_generate_new_cert(
        &mut self,
        driver_dir: &Path,
        output: Option<Output>,
    ) -> &mut Self {
        // create self signed certificate using makecert
        let target_dir = self.target_dir(driver_dir);
        let src_driver_cert_path = target_dir.join("WDRLocalTestCert.cer");

        self.expect_run_command(
            "makecert",
            vec![
                "-r".to_string(),
                "-pe".to_string(),
                "-a".to_string(),
                "SHA256".to_string(),
                "-eku".to_string(),
                "1.3.6.1.5.5.7.3.3".to_string(),
                "-ss".to_string(),
                "WDRTestCertStore".to_string(),
                "-n".to_string(),
                "CN=WDRLocalTestCert".to_string(),
                src_driver_cert_path.to_string_lossy().to_string(),
            ],
            output,
        )
    }

    fn expect_signtool_sign_driver_binary_sys_file(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        output: Option<Output>,
    ) -> &mut Self {
        // sign driver binary using signtool
        let (driver_name, package_dir) =
            self.normalized_name_and_package_dir(driver_name, driver_dir);
        let dest_driver_binary_path = package_dir.join(format!("{driver_name}.sys"));

        self.expect_run_command(
            "signtool",
            vec![
                "sign".to_string(),
                "/v".to_string(),
                "/s".to_string(),
                "WDRTestCertStore".to_string(),
                "/n".to_string(),
                "WDRLocalTestCert".to_string(),
                "/t".to_string(),
                "http://timestamp.digicert.com".to_string(),
                "/fd".to_string(),
                "SHA256".to_string(),
                dest_driver_binary_path.to_string_lossy().to_string(),
            ],
            output,
        )
    }

    fn expect_signtool_sign_cat_file(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        output: Option<Output>,
    ) -> &mut Self {
        // sign driver cat file using signtool
        let (driver_name, package_dir) =
            self.normalized_name_and_package_dir(driver_name, driver_dir);
        let dest_cat_file_path = package_dir.join(format!("{driver_name}.cat"));

        self.expect_run_command(
            "signtool",
            vec![
                "sign".to_string(),
                "/v".to_string(),
                "/s".to_string(),
                "WDRTestCertStore".to_string(),
                "/n".to_string(),
                "WDRLocalTestCert".to_string(),
                "/t".to_string(),
                "http://timestamp.digicert.com".to_string(),
                "/fd".to_string(),
                "SHA256".to_string(),
                dest_cat_file_path.to_string_lossy().to_string(),
            ],
            output,
        )
    }

    fn expect_signtool_verify_driver_binary_sys_file(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        output: Option<Output>,
    ) -> &mut Self {
        // verify signed driver binary using signtool
        let (driver_name, package_dir) =
            self.normalized_name_and_package_dir(driver_name, driver_dir);
        let dest_driver_binary_path = package_dir.join(format!("{driver_name}.sys"));

        self.expect_run_command(
            "signtool",
            vec![
                "verify".to_string(),
                "/v".to_string(),
                "/pa".to_string(),
                dest_driver_binary_path.to_string_lossy().to_string(),
            ],
            output,
        )
    }

    fn expect_signtool_verify_cat_file(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        output: Option<Output>,
    ) -> &mut Self {
        // verify signed driver cat file using signtool
        let (driver_name, package_dir) =
            self.normalized_name_and_package_dir(driver_name, driver_dir);
        let dest_cat_file_path = package_dir.join(format!("{driver_name}.cat"));

        self.expect_run_command(
            "signtool",
            vec![
                "verify".to_string(),
                "/v".to_string(),
                "/pa".to_string(),
                dest_cat_file_path.to_string_lossy().to_string(),
            ],
            output,
        )
    }

    fn expect_detect_wdk_build_number(&mut self) -> &mut Self {
        self.mock_wdk_build_provider
            .expect_detect_wdk_build_number()
            .returning(move || Ok(WDK_BUILD_NUMBER));
        self
    }

    fn expect_infverif(
        &mut self,
        driver_name: &str,
        driver_dir: &Path,
        driver_type: &str,
        output: Option<Output>,
    ) -> &mut Self {
        let (driver_name, package_dir) =
            self.normalized_name_and_package_dir(driver_name, driver_dir);
        let dest_inf_file_path = package_dir.join(format!("{driver_name}.inf"));

        let mut args = vec!["/v".to_string()];
        if driver_type.eq_ignore_ascii_case("KMDF") || driver_type.eq_ignore_ascii_case("WDM") {
            args.push("/w".to_string());
        } else {
            args.push("/u".to_string());
        }
        if self.build_args.sample_class {
            args.push("/msft".to_string());
        }

        args.push(dest_inf_file_path.to_string_lossy().to_string());

        self.expect_run_command("infverif", args, output)
    }

    fn expect_run_command(
        &mut self,
        command: &str,
        args: Vec<String>,
        output: Option<Output>,
    ) -> &mut Self {
        let command = command.to_string();
        let command2 = command.clone();
        // let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        self.mock_run_command
            .expect_run()
            .withf(
                move |cmd: &str, a: &[&str], _env_vars: &Option<&HashMap<&str, &str>>| -> bool {
                    cmd == command && a == args
                },
            )
            .once()
            .returning(move |_, _, _| match output.clone() {
                Some(output) => match output.status.code() {
                    Some(0) => Ok(output),
                    _ => Err(CommandError::from_output(&command2, &[], &output)),
                },
                None => Ok(Output {
                    status: ExitStatus::default(),
                    stdout: vec![],
                    stderr: vec![],
                }),
            });
        self
    }

    fn normalized_name_and_package_dir(
        &self,
        driver_name: &str,
        driver_dir: &Path,
    ) -> (String, PathBuf) {
        let driver_name = self.normalize(driver_name);
        let target_dir = self.target_dir(driver_dir);
        let package_dir = target_dir.join(format!("{driver_name}_package"));

        (driver_name, package_dir)
    }

    fn normalize(&self, driver_name: &str) -> String {
        driver_name.replace('-', "_")
    }
}

#[derive(Debug, Clone)]
enum Project {
    /// A project that is a standalone Rust crate
    Standalone(Package),

    /// A project that is a workspace with multiple
    /// member crates
    Workspace(Workspace),

    /// A project based on raw cargo metadata that
    /// can take any form. It does not necessarily
    /// have to be a valid package or workspace
    RawCargoMetadata(cargo_metadata::Metadata),
}

impl Project {
    fn as_standalone_package(&mut self) -> &mut Package {
        match self {
            Project::Standalone(package) => package,
            _ => panic!("Project is not a standalone package"),
        }
    }

    fn as_workspace(&mut self) -> &mut Workspace {
        match self {
            Project::Workspace(workspace) => workspace,
            _ => panic!("Project is not a workspace"),
        }
    }

    fn to_cargo_metadata(&self) -> cargo_metadata::Metadata {
        match self {
            Project::Standalone(package) => {
                let package_metadata = package_metadata(
                    &package.root_dir,
                    &package.name,
                    &package.version,
                    package.wdk_metadata.clone(),
                );

                cargo_metadata(
                    &package.root_dir,
                    vec![package_metadata.1],
                    &[package_metadata.0],
                    None,
                )
            }
            Project::Workspace(ws) => {
                let package_metadata_list = ws
                    .members
                    .iter()
                    .map(|package| {
                        let package_metadata = package_metadata(
                            &package.root_dir,
                            &package.name,
                            &package.version,
                            package.wdk_metadata.clone(),
                        );
                        (package_metadata.0, package_metadata.1)
                    })
                    .collect::<Vec<_>>();

                cargo_metadata(
                    &ws.root_dir,
                    package_metadata_list.iter().map(|p| p.1.clone()).collect(),
                    package_metadata_list
                        .into_iter()
                        .map(|p| p.0)
                        .collect::<Vec<_>>()
                        .as_slice(),
                    ws.wdk_metadata.clone(),
                )
            }
            Project::RawCargoMetadata(metadata) => metadata.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct BuildArgs {
    cwd: PathBuf,
    profile: Option<Profile>,
    target_arch: TargetArch,
    sample_class: bool,
    verify_signature: bool,
}

impl Default for BuildArgs {
    fn default() -> Self {
        Self {
            cwd: PathBuf::from("c:\\tmp"),
            profile: None,
            target_arch: TargetArch::Default(CpuArchitecture::Amd64),
            sample_class: false,
            verify_signature: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Command {
    CargoBuild,
    Copy,
    StampInf,
    Inf2Cat,
    CertMgr,
    MakeCert,
    SignTool,
    InfVerif,
}

#[derive(Debug, Clone)]
struct Package {
    name: String,
    version: String,
    root_dir: PathBuf,
    wdk_metadata: Option<WdkMetadata>,
    inx_file_exists: bool,
    cert_status: CertStatus,
    package_dir_exists: bool,
    failing_command: Option<Command>,
}

impl Default for Package {
    fn default() -> Self {
        Self {
            name: "sample-driver".to_string(),
            version: "0.0.1".to_string(),
            root_dir: PathBuf::from("C:\\tmp"),
            wdk_metadata: Some(WdkMetadata::default()),
            inx_file_exists: true,
            cert_status: CertStatus::DoesNotExist,
            package_dir_exists: false,
            failing_command: None,
        }
    }
}

impl Package {
    fn should_fail(&self, command: Command) -> bool {
        self.failing_command.as_ref().is_some_and(|c| *c == command)
    }
}

#[derive(Debug, Clone)]
struct Workspace {
    members: Vec<Package>,
    root_dir: PathBuf,
    wdk_metadata: Option<WdkMetadata>,
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace::create_for_a_mix_of_drivers_and_non_drivers()
    }
}

impl Workspace {
    fn create_for_a_mix_of_drivers_and_non_drivers() -> Self {
        let package_names = ["sample-kmdf-1", "sample-kmdf-2", "non-driver"];
        let mut members = package_names
            .iter()
            .enumerate()
            .map(|(i, name)| Package {
                name: name.to_string(),
                version: format!("0.0.{}", i + 1),
                root_dir: PathBuf::from("C:\\tmp").join(name),
                wdk_metadata: Some(WdkMetadata::default()),
                ..Package::default()
            })
            .collect::<Vec<_>>();

        members[2].wdk_metadata = None; // non-driver package does not have WDK metadata

        Self {
            members,
            root_dir: PathBuf::from("C:\\tmp"),
            wdk_metadata: Some(WdkMetadata::default()),
        }
    }

    fn create_for_only_non_drivers() -> Self {
        let members = [1, 2, 3]
            .iter()
            .map(|i| Package {
                name: format!("non-driver-{}", i),
                version: format!("0.0.{}", i),
                root_dir: PathBuf::from(format!("C:\\tmp\\non-driver-{}", i)),
                wdk_metadata: None,
                ..Default::default()
            })
            .collect();

        Self {
            members,
            wdk_metadata: None,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
struct WdkMetadata {
    driver_type: String,
    wdk_version: (u32, u32),
}

impl Default for WdkMetadata {
    fn default() -> Self {
        Self {
            driver_type: "KMDF".to_string(),
            wdk_version: (1, 33),
        }
    }
}

impl WdkMetadata {
    fn new(driver_type: &str, wdk_version: (u32, u32)) -> Self {
        Self {
            driver_type: driver_type.to_string(),
            wdk_version,
        }
    }

    fn to_json(&self) -> String {
        format!(
            r#"
            {{
                "wdk": {{
                    "driver-model": {{
                        "driver-type": "{}",
                        "{}-version-major": {},
                        "target-{}-version-minor": {}
                    }}
                }}
            }}
        "#,
            self.driver_type,
            self.driver_type.to_ascii_lowercase(),
            self.wdk_version.0,
            self.driver_type.to_ascii_lowercase(),
            self.wdk_version.1
        )
    }
}

fn create_build_action(context: &TestContext) -> BuildAction {
    let action = BuildAction::new(
        &BuildActionParams {
            working_dir: &context.build_args.cwd,
            profile: context.build_args.profile.as_ref(),
            target_arch: context.build_args.target_arch.clone(),
            verify_signature: context.build_args.verify_signature,
            is_sample_class: context.build_args.sample_class,
            verbosity_level: clap_verbosity_flag::Verbosity::new(1, 0),
        },
        &context.mock_wdk_build_provider,
        &context.mock_run_command,
        &context.mock_fs_provider,
        &context.mock_metadata_provider,
    );

    assert!(
        action.is_ok(),
        "Failed to create BuildAction: {}",
        action.err().unwrap()
    );

    action.unwrap()
}

fn run_build_action(context: TestContext) -> Result<(), BuildActionError> {
    let build_action = create_build_action(&context);
    let run_result = build_action.run();
    run_result
}

fn invalid_driver_cargo_metadata() -> cargo_metadata::Metadata {
    let metadata_json = r#"
        {
            "packages": [
                {
                    "name": "sample-driver",
                    "version": "0.0.1",
                    "id": "path+file:///C:/tmp/sample-driver#0.0.1",
                    "license": "MIT OR Apache-2.0",
                    "license_file": null,
                    "description": null,
                    "source": null,
                    "dependencies": [],
                    "targets": [
                        {
                            "kind": [
                                "cdylib"
                            ],
                            "crate_types": [
                                "cdylib"
                            ],
                            "name": "sample_driver",
                            "src_path": "C:\\tmp\\sample-driver\\src\\lib.rs",
                            "edition": "2021",
                            "doc": true,
                            "doctest": false,
                            "test": false
                        },
                        {
                            "kind": [
                                "custom-build"
                            ],
                            "crate_types": [
                                "bin"
                            ],
                            "name": "build-script-build",
                            "src_path": "C:\\tmp\\sample-driver\\build.rs",
                            "edition": "2021",
                            "doc": false,
                            "doctest": false,
                            "test": false
                        }
                    ],
                    "features": {
                        "default": [],
                        "nightly": [
                            "wdk/nightly",
                            "wdk-sys/nightly"
                        ]
                    },
                    "manifest_path": "C:\\tmp\\sample-driver\\Cargo.toml",
                    "metadata": {
                        "wdk": {}
                    },
                    "publish": [],
                    "authors": [],
                    "categories": [],
                    "keywords": [],
                    "readme": null,
                    "repository": null,
                    "homepage": null,
                    "documentation": null,
                    "edition": "2021",
                    "links": null,
                    "default_run": null,
                    "rust_version": null
                }
            ],
            "workspace_members": [
                "path+file:///C:/tmp/sample-driver#0.0.1"
            ],
            "target_directory": "C:\\tmp\\sample-driver\\target",
            "version": 1,
            "workspace_root": "C:\\tmp\\sample-driver",
            "metadata": {
                "wdk": {
                    "driver-model": {
                        "driver-type": "KMDF"
                    }
                }
            }
        }
    "#;

    serde_json::from_str::<cargo_metadata::Metadata>(metadata_json).unwrap()
}

#[derive(Debug, Clone)]
enum CertStatus {
    ExistsInPackageDir,
    ExistsInStore,
    DoesNotExist,
}

#[derive(Clone)]
struct TestMetadataPackage(String);
#[derive(Clone)]
struct TestMetadataWorkspaceMemberId(String);

fn cargo_metadata(
    root_dir: &Path,
    package_list: Vec<TestMetadataPackage>,
    workspace_member_list: &[TestMetadataWorkspaceMemberId],
    metadata: Option<WdkMetadata>,
) -> cargo_metadata::Metadata {
    let metadata_section = match metadata {
        Some(metadata) => metadata.to_json(),
        None => String::from("null"),
    };

    let metadata_json = format!(
        r#"
    {{
        "target_directory": "{}",
        "workspace_root": "{}",
        "packages": [
            {}
            ],
        "workspace_members": [{}],
        "metadata": {},
        "version": 1
    }}"#,
        root_dir.join("target").to_string_lossy().escape_default(),
        root_dir.to_string_lossy().escape_default(),
        package_list
            .into_iter()
            .map(|p| p.0)
            .collect::<Vec<String>>()
            .join(", "),
        // Require quotes around each member
        workspace_member_list
            .iter()
            .map(|s| format!("\"{}\"", s.0))
            .collect::<Vec<String>>()
            .join(", "),
        metadata_section
    );

    serde_json::from_str::<cargo_metadata::Metadata>(&metadata_json).unwrap()
}

fn package_metadata(
    root_dir: &Path,
    default_package_name: &str,
    default_package_version: &str,
    metadata: Option<WdkMetadata>,
) -> (TestMetadataWorkspaceMemberId, TestMetadataPackage) {
    let package_id = format!(
        "path+file:///{}#{}@{}",
        root_dir.to_string_lossy().escape_default(),
        default_package_name,
        default_package_version
    );
    let metadata_section = match metadata {
        Some(metadata) => metadata.to_json(),
        None => String::from("null"),
    };
    (
        TestMetadataWorkspaceMemberId(package_id.clone()),
        #[allow(clippy::format_in_format_args)]
        TestMetadataPackage(format!(
            r#"
            {{
            "name": "{}",
            "version": "{}",
            "id": "{}",
            "dependencies": [],
            "targets": [
                {{
                    "kind": [
                        "cdylib"
                    ],
                    "crate_types": [
                        "cdylib"
                    ],
                    "name": "{}",
                    "src_path": "{}",
                    "edition": "2021",
                    "doc": true,
                    "doctest": false,
                    "test": true
                }}
            ],
            "features": {{}},
            "manifest_path": "{}",
            "authors": [],
            "categories": [],
            "keywords": [],
            "edition": "2021",
            "metadata": {}
        }}
        "#,
            default_package_name,
            default_package_version,
            package_id,
            default_package_name,
            root_dir
                .join("src")
                .join("main.rs")
                .to_string_lossy()
                .escape_default(),
            root_dir
                .join("Cargo.toml")
                .to_string_lossy()
                .escape_default(),
            metadata_section
        )),
    )
}

fn certmgr_output_no_certs() -> Output {
    certmgr_output(
        r"==============No Certificates ==========
                        ==============No CTLs ==========
                        ==============No CRLs ==========
                        ==============================================
                        CertMgr Succeeded",
    )
}

fn certmgr_output_cert_exists() -> Output {
    certmgr_output(
        r"==============Certificate # 1 ==========
                Subject::
                [0,0] 2.5.4.3 (CN) WDRLocalTestCert
                Issuer::
                [0,0] 2.5.4.3 (CN) WDRLocalTestCert
                SerialNumber::
                5E 04 0D 63 35 20 76 A5 4A E1 96 BF CF 01 0F 96
                SHA1 Thumbprint::
                    FB972842 C63CD369 E07D0C71 88E17921 B5813C71
                MD5 Thumbprint::
                    832B3F18 707EA3F6 54465207 345A93F1
                Provider Type:: 1 Provider Name:: Microsoft Strong Cryptographic Provider Container: 68f79a6e-6afa-4ec7-be5b-16d6656edd3f KeySpec: 2
                NotBefore::
                Tue Jan 28 13:51:04 2025
                NotAfter::
                Sun Jan 01 05:29:59 2040
                ==============No CTLs ==========
                ==============No CRLs ==========
                ==============================================
                CertMgr Succeeded",
    )
}

fn certmgr_output(stdout: &str) -> Output {
    Output {
        status: ExitStatus::default(),
        stdout: stdout.as_bytes().to_vec(),
        stderr: vec![],
    }
}

fn failure_output() -> Output {
    Output {
        status: ExitStatus::from_raw(1), // 1 is failure exit code
        stdout: vec![],
        stderr: vec![],
    }
}
