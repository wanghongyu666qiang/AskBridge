mod cdp;
mod chrome;
mod desktop_pwa;
mod endpoint;
mod profile;
mod worker;

pub(crate) use cdp::FileInputResult;
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
        env, fs,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::adapter::{GenericProviderAdapter, PageSession, ProviderAdapter};
    use askbridge_core::{
        AppConfig, CapturedImage, DispatchMode, DispatchOutcome, DispatchRequest,
        PreparationPolicy, RecoveryHint, ScreenRect,
    };

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
                        let request_text = String::from_utf8_lossy(&request);
                        let body: &[u8] = if request_text.starts_with("GET /login-shell ") {
                            b"<!doctype html><title>AskBridge Login Shell</title><main><a href='https://accounts.google.com/ServiceLogin?continue=https%3A%2F%2Fgemini.google.com%2Fapp'>Sign in</a></main>"
                        } else {
                            br#"<!doctype html><title>AskBridge Phase 6</title><header><textarea aria-label='Search'></textarea></header><main><div id='prompt-textarea' contenteditable='true' aria-label='Message'></div><input type='file' accept='application/pdf'><input type='file' accept='image/*' onchange="const receipt=document.createElement('span');receipt.textContent=this.files[0].name;this.parentElement.appendChild(receipt)"></main>"#
                        };
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
        let test_result = run_cdp_round_trip(&endpoint, address.port(), &profile_path, &cancelled);
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

    #[test]
    #[ignore = "opens a real provider in the existing AskBridge profile and prepares test text"]
    fn dedicated_chrome_live_provider_preparation() {
        let chrome_path =
            env::var("ASKBRIDGE_TEST_CHROME").expect("ASKBRIDGE_TEST_CHROME must name chrome.exe");
        let profile_path = PathBuf::from(
            env::var("ASKBRIDGE_TEST_PROFILE")
                .expect("ASKBRIDGE_TEST_PROFILE must name the managed profile directory"),
        );
        let provider_id = env::var("ASKBRIDGE_TEST_PROVIDER")
            .expect("ASKBRIDGE_TEST_PROVIDER must name a built-in provider");
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

        let test_result =
            run_live_provider_preparation(&endpoint, &profile_path, &provider_id, &cancelled);
        let close_result =
            CdpClient::close_managed_endpoint(endpoint, Duration::from_secs(5), &cancelled);
        let exited = manager
            .wait_for_managed_exit(Duration::from_secs(10))
            .expect("wait for Chrome");

        test_result.expect("live provider preparation");
        close_result.expect("normal Browser.close");
        assert!(exited, "managed Chrome did not close normally");
    }

    #[test]
    #[ignore = "holds a managed Chrome session open for external performance sampling"]
    fn dedicated_chrome_performance_hold() {
        let chrome_path =
            env::var("ASKBRIDGE_TEST_CHROME").expect("ASKBRIDGE_TEST_CHROME must name chrome.exe");
        let profile_path = PathBuf::from(
            env::var("ASKBRIDGE_TEST_PROFILE")
                .expect("ASKBRIDGE_TEST_PROFILE must name the managed profile directory"),
        );
        let hold_seconds = env::var("ASKBRIDGE_TEST_HOLD_SECONDS")
            .expect("ASKBRIDGE_TEST_HOLD_SECONDS must be set")
            .parse::<u64>()
            .expect("ASKBRIDGE_TEST_HOLD_SECONDS must be an integer");
        assert!((30..=900).contains(&hold_seconds));
        let target_url = env::var("ASKBRIDGE_TEST_HOLD_URL")
            .unwrap_or_else(|_| "https://chatgpt.com/".to_owned());

        let profile = ManagedProfile::open(&profile_path.to_string_lossy(), &profile_path)
            .expect("managed profile");
        let installation =
            ChromeInstallation::discover(Some(&chrome_path)).expect("Chrome installation");
        let mut manager = ChromeManager::new(installation, profile);
        let cancelled = AtomicBool::new(false);
        let endpoint = manager
            .launch_and_wait(Duration::from_secs(15), &cancelled)
            .expect("dynamic endpoint");
        let client =
            CdpClient::connect(endpoint.clone(), Duration::from_secs(5), &cancelled).expect("CDP");
        let target = client
            .create_target(&target_url)
            .expect("performance target");
        client.activate_target(&target.id).expect("activate target");
        client
            .wait_until_ready(&target, Duration::from_secs(20), &cancelled)
            .expect("target ready");
        eprintln!(
            "Dedicated Chrome performance session ready: pid={} hold_seconds={hold_seconds}",
            manager.managed_process_id().expect("managed pid")
        );
        thread::sleep(Duration::from_secs(hold_seconds));

        CdpClient::close_managed_endpoint(endpoint, Duration::from_secs(5), &cancelled)
            .expect("normal Browser.close");
        assert!(
            manager
                .wait_for_managed_exit(Duration::from_secs(10))
                .expect("wait for Chrome"),
            "managed Chrome did not close normally"
        );
    }

    #[test]
    #[ignore = "measures cold browser launch and two real preparations without sending"]
    fn dedicated_chrome_preparation_timings() {
        let chrome_path =
            env::var("ASKBRIDGE_TEST_CHROME").expect("ASKBRIDGE_TEST_CHROME must name chrome.exe");
        let profile_path = PathBuf::from(
            env::var("ASKBRIDGE_TEST_PROFILE")
                .expect("ASKBRIDGE_TEST_PROFILE must name the managed profile directory"),
        );
        let provider_id = env::var("ASKBRIDGE_TEST_PROVIDER")
            .expect("ASKBRIDGE_TEST_PROVIDER must name a built-in provider");
        let output_path = PathBuf::from(
            env::var("ASKBRIDGE_TEST_TIMINGS_OUTPUT")
                .expect("ASKBRIDGE_TEST_TIMINGS_OUTPUT must be an explicit path"),
        );
        assert!(output_path.is_absolute(), "timings output must be absolute");

        let profile = ManagedProfile::open(&profile_path.to_string_lossy(), &profile_path)
            .expect("managed profile");
        let installation =
            ChromeInstallation::discover(Some(&chrome_path)).expect("Chrome installation");
        let mut manager = ChromeManager::new(installation, profile);
        let cancelled = AtomicBool::new(false);
        let launch_started = Instant::now();
        let endpoint = manager
            .launch_and_wait(Duration::from_secs(15), &cancelled)
            .expect("dynamic endpoint");
        let browser_launch_ms = launch_started.elapsed().as_secs_f64() * 1_000.0;

        let measurement = (|| -> askbridge_core::Result<(f64, f64)> {
            let first_started = Instant::now();
            run_live_provider_preparation(&endpoint, &profile_path, &provider_id, &cancelled)?;
            let first_preparation_ms = first_started.elapsed().as_secs_f64() * 1_000.0;
            let continuous_started = Instant::now();
            run_live_provider_preparation(&endpoint, &profile_path, &provider_id, &cancelled)?;
            let continuous_preparation_ms = continuous_started.elapsed().as_secs_f64() * 1_000.0;
            Ok((first_preparation_ms, continuous_preparation_ms))
        })();
        let close_result =
            CdpClient::close_managed_endpoint(endpoint, Duration::from_secs(5), &cancelled);
        let exited = manager
            .wait_for_managed_exit(Duration::from_secs(10))
            .expect("wait for Chrome");

        let (first_preparation_ms, continuous_preparation_ms) =
            measurement.expect("real preparation timings");
        close_result.expect("normal Browser.close");
        assert!(exited, "managed Chrome did not close normally");
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).expect("timings output directory");
        }
        let report = serde_json::json!({
            "measured_at_unix_ms": SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_millis(),
            "provider": provider_id,
            "browser_launch_ms": browser_launch_ms,
            "first_preparation_ms": first_preparation_ms,
            "continuous_preparation_ms": continuous_preparation_ms,
            "auto_submit": false,
            "managed_browser_closed": true
        });
        fs::write(
            &output_path,
            serde_json::to_vec_pretty(&report).expect("serialize timings"),
        )
        .expect("write timings report");
        eprintln!("Preparation timings written to {}", output_path.display());
    }

    fn run_cdp_round_trip(
        endpoint: &DevToolsEndpoint,
        port: u16,
        profile_path: &std::path::Path,
        cancelled: &AtomicBool,
    ) -> askbridge_core::Result<()> {
        let client = CdpClient::connect(endpoint.clone(), Duration::from_secs(5), cancelled)?;
        eprintln!("Phase 6 integration: browser CDP connected");
        let page_url = format!("http://127.0.0.1:{port}/phase6");
        let target = client.create_target(&page_url)?;
        eprintln!("Phase 6 integration: target created");
        client.activate_target(&target.id)?;
        eprintln!("Phase 6 integration: target activated");
        client.wait_until_ready(&target, Duration::from_secs(10), cancelled)?;
        eprintln!("Phase 6 integration: target interactive");
        if !client
            .list_targets()?
            .iter()
            .any(|candidate| candidate.id == target.id && candidate.url == page_url)
        {
            return Err(askbridge_core::AppError::TargetNotFound);
        }
        let request = DispatchRequest::new(
            "phase5-integration".to_owned(),
            DispatchMode::CaptureWithPrompt,
            "test-provider".to_owned(),
            "Verify generic preparation".to_owned(),
            Some(CapturedImage::new(
                1,
                1,
                vec![10, 20, 30, 255],
                ScreenRect::new(0, 0, 1, 1),
            )?),
            1,
        )?;
        let adapter = GenericProviderAdapter::for_provider(
            "test-provider",
            Some("chatgpt"),
            vec![format!("http://127.0.0.1:{port}/phase6")],
        )?;
        let policy = PreparationPolicy::new(5_000)?;
        let temp_root = profile_path.join("Temp");
        let mut page = PageSession::DedicatedChrome {
            client: &client,
            target: &target,
            temp_root: &temp_root,
            cancelled,
        };
        let preparation = adapter.prepare(&mut page, &request, &policy)?;
        let outcome = DispatchOutcome::from_preparation(&request, preparation)?;
        if !matches!(outcome, DispatchOutcome::PreparedForUser(_)) {
            return Err(askbridge_core::AppError::InvalidPreparation(
                "Phase 6 integration did not prepare both inputs".to_owned(),
            ));
        }

        let login_url = format!("http://127.0.0.1:{port}/login-shell");
        let login_target = client.create_target(&login_url)?;
        client.activate_target(&login_target.id)?;
        client.wait_until_ready(&login_target, Duration::from_secs(10), cancelled)?;
        let login_request = DispatchRequest::new(
            "phase6-login-shell".to_owned(),
            DispatchMode::TextOnlyPrompt,
            "test-provider".to_owned(),
            "Verify login classification".to_owned(),
            None,
            1,
        )?;
        let login_adapter =
            GenericProviderAdapter::for_provider("test-provider", Some("gemini"), vec![login_url])?;
        let mut login_page = PageSession::DedicatedChrome {
            client: &client,
            target: &login_target,
            temp_root: &temp_root,
            cancelled,
        };
        let login_preparation = login_adapter.prepare(&mut login_page, &login_request, &policy)?;
        let login_outcome = DispatchOutcome::from_preparation(&login_request, login_preparation)?;
        if !matches!(
            login_outcome,
            DispatchOutcome::ManualFallbackReady(ref fallback)
                if fallback.recovery_hint == Some(RecoveryHint::LoginInBrowser)
        ) {
            return Err(askbridge_core::AppError::InvalidPreparation(
                "login shell was not classified as LoginInBrowser".to_owned(),
            ));
        }
        Ok(())
    }

    fn run_live_provider_preparation(
        endpoint: &DevToolsEndpoint,
        profile_path: &std::path::Path,
        provider_id: &str,
        cancelled: &AtomicBool,
    ) -> askbridge_core::Result<()> {
        let provider = AppConfig::default()
            .merged_providers()?
            .into_iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| askbridge_core::AppError::InvalidProvider(provider_id.to_owned()))?;
        let client = CdpClient::connect(endpoint.clone(), Duration::from_secs(5), cancelled)?;
        let target = client.create_target(&provider.start_url)?;
        client.activate_target(&target.id)?;
        client.wait_until_ready(&target, Duration::from_secs(20), cancelled)?;
        thread::sleep(Duration::from_secs(3));

        if env::var_os("ASKBRIDGE_TEST_STRUCTURE_DIAGNOSTIC").is_some() {
            let structure = client.evaluate_in_target(
                &target,
                r#"(() => ({
                    origin: location.origin,
                    path: location.pathname,
                    ready: document.readyState,
                    contenteditables: Array.from(document.querySelectorAll('[contenteditable]')).slice(0, 20).map((element) => ({
                        tag: element.tagName,
                        contenteditable: element.getAttribute('contenteditable'),
                        role: element.getAttribute('role'),
                        ariaLabel: element.getAttribute('aria-label'),
                        testId: element.getAttribute('data-testid'),
                        classes: String(element.className || '').split(/\s+/).filter(Boolean).slice(0, 8)
                    })),
                    textareas: Array.from(document.querySelectorAll('textarea')).slice(0, 20).map((element) => ({
                        role: element.getAttribute('role'),
                        ariaLabel: element.getAttribute('aria-label'),
                        testId: element.getAttribute('data-testid'),
                        classes: String(element.className || '').split(/\s+/).filter(Boolean).slice(0, 8)
                    }))
                }))()"#,
                cancelled,
                Duration::from_secs(5),
            )?;
            eprintln!(
                "Phase 6 structure diagnostic: provider={provider_id} structure={}",
                structure.pointer("/result/value").unwrap_or(&structure)
            );
        }

        let expects_attachment = env::var_os("ASKBRIDGE_TEST_WITH_IMAGE").is_some();
        let request = DispatchRequest::new(
            format!("phase6-live-{provider_id}"),
            if expects_attachment {
                DispatchMode::CaptureWithPrompt
            } else {
                DispatchMode::TextOnlyPrompt
            },
            provider.id.clone(),
            "AskBridge Phase 6 validation - do not send".to_owned(),
            expects_attachment
                .then(|| {
                    CapturedImage::new(1, 1, vec![10, 20, 30, 255], ScreenRect::new(0, 0, 1, 1))
                })
                .transpose()?,
            1,
        )?;
        let adapter = GenericProviderAdapter::for_provider(
            &provider.id,
            provider.adapter_override.as_deref(),
            provider.url_patterns.clone(),
        )?;
        let policy = PreparationPolicy::new(20_000)?;
        let temp_root = profile_path.join("Temp");
        let mut page = PageSession::DedicatedChrome {
            client: &client,
            target: &target,
            temp_root: &temp_root,
            cancelled,
        };
        let preparation = adapter.prepare(&mut page, &request, &policy)?;
        let outcome = DispatchOutcome::from_preparation(&request, preparation)?;
        match outcome {
            DispatchOutcome::PreparedForUser(prepared)
                if prepared.text_inserted
                    && (!expects_attachment || prepared.attachment_prepared) =>
            {
                eprintln!(
                    "Phase 6 live provider: provider={provider_id} text_inserted=true attachment_prepared={}",
                    prepared.attachment_prepared
                );
                Ok(())
            }
            DispatchOutcome::PreparedForUser(_) => {
                Err(askbridge_core::AppError::InvalidPreparation(format!(
                    "provider '{provider_id}' reported prepared without verified text"
                )))
            }
            DispatchOutcome::ManualFallbackReady(fallback)
                if fallback.recovery_hint == Some(RecoveryHint::LoginInBrowser) =>
            {
                Err(askbridge_core::AppError::InvalidPreparation(format!(
                    "provider '{provider_id}' requires login; live preparation was not accepted"
                )))
            }
            DispatchOutcome::ManualFallbackReady(fallback) => {
                Err(askbridge_core::AppError::InvalidPreparation(format!(
                    "provider '{provider_id}' required fallback at {:?} with {:?}",
                    fallback.failure_stage, fallback.recovery_hint
                )))
            }
            DispatchOutcome::Cancelled => Err(askbridge_core::AppError::BrowserCancelled),
        }
    }

    fn unique_integration_profile() -> PathBuf {
        env::temp_dir().join(format!(
            "askbridge-phase6-chrome-{}-{}",
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
