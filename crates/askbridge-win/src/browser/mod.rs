mod cdp;
mod chrome;
mod desktop_pwa;
mod endpoint;
mod profile;
mod worker;

pub use cdp::{CdpClient, CdpTarget};
pub use chrome::{ChromeInstallation, ChromeManager};
pub use desktop_pwa::DesktopPwaLauncher;
pub use endpoint::DevToolsEndpoint;
pub use profile::ManagedProfile;
pub use worker::{
    BrowserEvent, BrowserJob, BrowserLaunch, BrowserService, BrowserStage, BrowserSurface,
    BrowserWarmupJob, DedicatedChromeJob, DesktopPwaJob, WM_BROWSER_EVENT,
};

#[cfg(test)]
mod integration_tests {
    use std::{
        env,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    #[ignore = "launches an installed Chrome and opens a loopback test page"]
    fn dedicated_chrome_cdp_round_trip() {
        let chrome_path =
            env::var("ASKBRIDGE_TEST_CHROME").expect("ASKBRIDGE_TEST_CHROME must name chrome.exe");
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("listener address");
        let stop_server = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop_server);
        let server = thread::spawn(move || {
            while !server_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
                        let mut request = [0u8; 2048];
                        let _ = stream.read(&mut request);
                        let body =
                            b"<!doctype html><title>AskBridge Phase 4</title><main>ready</main>";
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.write_all(body);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        let profile_path = unique_integration_profile();
        let profile = ManagedProfile::open(&profile_path.to_string_lossy(), &profile_path)
            .expect("managed profile");
        let installation =
            ChromeInstallation::discover(Some(&chrome_path)).expect("Chrome installation");
        let mut manager = ChromeManager::new(installation, profile);
        let cancelled = AtomicBool::new(false);
        let endpoint = manager
            .launch_and_wait(Duration::from_secs(15), &cancelled)
            .expect("dynamic endpoint");
        assert!(manager.managed_process_id().is_some());
        let test_result = run_cdp_round_trip(&endpoint, address.port(), &cancelled);
        let close_result =
            CdpClient::close_managed_endpoint(endpoint, Duration::from_secs(5), &cancelled);
        let exited = manager
            .wait_for_managed_exit(Duration::from_secs(10))
            .expect("wait for Chrome");
        stop_server.store(true, Ordering::Release);
        server.join().expect("server");
        let removed = remove_profile_when_released(&profile_path);

        test_result.expect("CDP round trip");
        close_result.expect("normal Browser.close");
        assert!(exited, "managed Chrome did not close normally");
        assert!(removed, "temporary Chrome profile remained locked");
    }

    fn run_cdp_round_trip(
        endpoint: &DevToolsEndpoint,
        port: u16,
        cancelled: &AtomicBool,
    ) -> askbridge_core::Result<()> {
        let client = CdpClient::connect(endpoint.clone(), Duration::from_secs(5), cancelled)?;
        eprintln!("Phase 4 integration: browser CDP connected");
        let page_url = format!("http://127.0.0.1:{port}/phase4");
        let target = client.create_target(&page_url)?;
        eprintln!("Phase 4 integration: target created");
        client.activate_target(&target.id)?;
        eprintln!("Phase 4 integration: target activated");
        client.wait_until_ready(&target, Duration::from_secs(10), cancelled)?;
        eprintln!("Phase 4 integration: target interactive");
        if !client
            .list_targets()?
            .iter()
            .any(|candidate| candidate.id == target.id && candidate.url == page_url)
        {
            return Err(askbridge_core::AppError::TargetNotFound);
        }
        Ok(())
    }

    fn unique_integration_profile() -> PathBuf {
        env::temp_dir().join(format!(
            "askbridge-phase4-chrome-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn remove_profile_when_released(path: &PathBuf) -> bool {
        for _ in 0..40 {
            match std::fs::remove_dir_all(path) {
                Ok(()) => return true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        }
        false
    }
}
