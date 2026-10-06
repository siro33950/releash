pub(crate) struct MacCliInstall;
impl crate::usecase::cli_install::CliInstallGateway for MacCliInstall {
    fn install(&self) -> Result<String, String> {
        crate::infrastructure::platform::cli_install::install_cli()
    }
}

#[cfg(all(test, target_os = "macos", debug_assertions))]
#[path = "cli_install_test.rs"]
mod cli_install_tests;
