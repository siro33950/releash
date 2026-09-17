#[cfg(debug_assertions)]
mod acceptance {
    use releash_lib::client_api_acceptance::{
        spawn_desktop_successor, wait_for_desktop_predecessor,
    };
    use std::{
        path::Path,
        process::Command,
        time::{Duration, Instant},
    };

    fn wait_for_file(path: &Path) -> String {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "successor did not report its predecessor outcome"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::read_to_string(path).unwrap()
    }

    pub fn run() {
        if let Ok(directory) = std::env::var("RELEASH_RESTART_TEST_DIR") {
            let record = Path::new(&directory).join("successor");
            if std::env::args().any(|arg| arg == "--internal-restart") {
                let result = wait_for_desktop_predecessor();
                assert!(std::env::args().any(|arg| arg == "--hidden"));
                let message = match result {
                    Ok(true) => "ready".into(),
                    Ok(false) => panic!("successor arguments were lost"),
                    Err(error) => error,
                };
                let temporary = record.with_extension("tmp");
                std::fs::write(&temporary, message).unwrap();
                std::fs::rename(temporary, record).unwrap();
            } else {
                assert!(!wait_for_desktop_predecessor().unwrap());
                spawn_desktop_successor().unwrap();
                std::thread::sleep(Duration::from_millis(300));
                assert!(
                    !record.exists(),
                    "successor advanced while predecessor was alive"
                );
                if std::env::var("RELEASH_RESTART_TEST_WAIT").unwrap() == "true" {
                    assert_eq!(
                        wait_for_file(&record),
                        "Previous UI exit could not be confirmed."
                    );
                }
            }
            return;
        }
        for predecessor_stays_alive in [false, true] {
            // Given
            let directory = tempfile::tempdir().unwrap();
            // When: spawn_successor launches this executable, which runs the production wait.
            let status = Command::new(std::env::current_exe().unwrap())
                .arg("--hidden")
                .env("RELEASH_RESTART_TEST_DIR", directory.path())
                .env(
                    "RELEASH_RESTART_TEST_WAIT",
                    predecessor_stays_alive.to_string(),
                )
                .status()
                .unwrap();
            // Then
            assert!(status.success());
            assert_eq!(
                wait_for_file(&directory.path().join("successor")),
                if predecessor_stays_alive {
                    "Previous UI exit could not be confirmed."
                } else {
                    "ready"
                }
            );
        }
    }
}

fn main() {
    #[cfg(debug_assertions)]
    acceptance::run();
    #[cfg(not(debug_assertions))]
    eprintln!("desktop_restart acceptance requires a debug build");
}
