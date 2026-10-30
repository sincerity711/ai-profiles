//! Official desktop updates. Only the app-owned vendor directory is replaced;
//! profile data and externally installed applications are never overwritten.
mod feed;

use crate::{app_kind::AppKind, paths};
use feed::Release;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const MAX_ARCHIVE: u64 = 2 * 1024 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cache {
    latest: Option<Release>,
    checked_at: Option<String>,
    phase: String,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Transaction {
    incoming: String,
    backup: String,
    rollback: bool,
}

fn rotate_backup(dir: &Path, backup: &Path) -> Result<()> {
    if !backup.exists() {
        return Ok(());
    }
    let previous = dir.join("previous");
    let retired = dir.join(format!("retired-{}", uuid::Uuid::new_v4()));
    if previous.exists() {
        fs::rename(&previous, &retired).map_err(|e| e.to_string())?;
    }
    if let Err(e) = fs::rename(backup, &previous) {
        if retired.exists() {
            let _ = fs::rename(&retired, &previous);
        }
        return Err(e.to_string());
    }
    if retired.exists() {
        let _ = fs::remove_dir_all(retired);
    }
    Ok(())
}

/// Recover a crash between the two renames before another operation can run.
fn recover_transaction(app: AppKind) -> Result<()> {
    let dir = root(app)?;
    let journal = dir.join("transaction.json");
    let Some(tx): Option<Transaction> = read_json(&journal)? else {
        return Ok(());
    };
    if !tx.incoming.starts_with(".incoming-")
        || !tx.backup.starts_with("backup-")
        || [&tx.incoming, &tx.backup]
            .iter()
            .any(|n| n.contains('/') || n.contains('\\'))
    {
        return Err("Invalid update recovery record".into());
    }
    let destination = target(app)?;
    let incoming = destination.parent().unwrap().join(tx.incoming);
    let backup = dir.join(tx.backup);
    let mut state = cache(app)?;
    if !destination.exists() && backup.join("App.app").exists() {
        fs::rename(backup.join("App.app"), &destination).map_err(|e| e.to_string())?;
        state.phase = "error".into();
        state.error = Some("An interrupted installation was recovered. The previous app is restored; retry when ready.".into());
    } else if destination.exists() && !incoming.exists() {
        rotate_backup(&dir, &backup)?;
        if !tx.rollback && dir.join("ready").exists() {
            fs::remove_dir_all(dir.join("ready")).map_err(|e| e.to_string())?;
        }
        state.phase = if tx.rollback { "restored" } else { "installed" }.into();
        state.error = None;
    } else {
        state.phase = "error".into();
        state.error =
            Some("Installation was interrupted before replacement. Retry when ready.".into());
    }
    save(app, &state)?;
    fs::remove_file(journal).map_err(|e| e.to_string())?;
    if incoming.exists() {
        let _ = fs::remove_dir_all(incoming);
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopUpdate {
    app: AppKind,
    installed_version: Option<String>,
    installed_path: Option<String>,
    managed: bool,
    latest_version: Option<String>,
    update_available: bool,
    ready_version: Option<String>,
    previous_version: Option<String>,
    running: bool,
    busy: bool,
    phase: String,
    downloaded: u64,
    total: Option<u64>,
    checked_at: Option<String>,
    error: Option<String>,
}

fn root(app: AppKind) -> Result<PathBuf> {
    Ok(paths::app_data_dir()
        .map_err(|e| e.message())?
        .join("desktop-updates")
        .join(match app {
            AppKind::Claude => "claude",
            AppKind::Codex => "codex",
        }))
}
fn target(app: AppKind) -> Result<PathBuf> {
    Ok(paths::app_data_dir()
        .map_err(|e| e.message())?
        .join("vendor")
        .join(match app {
            AppKind::Claude => "Claude.app",
            AppKind::Codex => "ChatGPT.app",
        }))
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    crate::profiles::atomic_write(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.message())
}
fn cache(app: AppKind) -> Result<Cache> {
    Ok(read_json(&root(app)?.join("status.json"))?.unwrap_or_default())
}
fn save(app: AppKind, cache: &Cache) -> Result<()> {
    write_json(&root(app)?.join("status.json"), cache)
}
fn lock_file(app: AppKind) -> Result<File> {
    let dir = root(app)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("operation.lock"))
        .map_err(|e| e.to_string())
}
fn lock(app: AppKind) -> Result<File> {
    let file = lock_file(app)?;
    file.try_lock_exclusive()
        .map_err(|_| "An update operation is already running for this app".to_string())?;
    Ok(file)
}
fn busy(app: AppKind) -> Result<bool> {
    let file = lock_file(app)?;
    Ok(file.try_lock_exclusive().is_err())
}

fn info(bundle: &Path) -> Result<plist::Dictionary> {
    plist::Value::from_file(bundle.join("Contents/Info.plist"))
        .map_err(|e| e.to_string())?
        .into_dictionary()
        .ok_or_else(|| "Invalid app Info.plist".into())
}
fn field(info: &plist::Dictionary, key: &str) -> Result<String> {
    info.get(key)
        .and_then(plist::Value::as_string)
        .map(str::to_owned)
        .ok_or_else(|| format!("App is missing {key}"))
}
fn version(bundle: &Path) -> Option<String> {
    info(bundle)
        .ok()
        .and_then(|i| field(&i, "CFBundleShortVersionString").ok())
}
fn build(bundle: &Path) -> Option<String> {
    info(bundle)
        .ok()
        .and_then(|i| field(&i, "CFBundleVersion").ok())
}

fn running_in(app: AppKind, processes: &str) -> bool {
    processes.lines().any(|line| {
        let Some((_, exec)) = line.trim().rsplit_once("/Contents/MacOS/") else {
            return false;
        };
        app.spec().gui_bundle_candidates.iter().any(|c| {
            exec == c.macos_exec
                || exec == format!("{}.bin", c.macos_exec)
                || exec.starts_with(&format!("{} Helper", c.macos_exec))
        })
    })
}
fn running(app: AppKind) -> Result<bool> {
    let output = Command::new("ps")
        .args(["-axo", "comm="])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("Cannot check running apps; installation is blocked".into());
    }
    Ok(running_in(app, &String::from_utf8_lossy(&output.stdout)))
}
fn require_closed(app: AppKind) -> Result<()> {
    if running(app)? {
        Err(format!(
            "Quit {} and all its profile instances, then try again. Your download is kept.",
            app.spec().display_name
        ))
    } else {
        Ok(())
    }
}

fn view(app: AppKind) -> Result<DesktopUpdate> {
    if root(app)?.join("transaction.json").exists() {
        if let Ok(_guard) = lock(app) {
            recover_transaction(app)?;
        }
    }
    let c = cache(app)?;
    let installed = paths::resolve_gui_app(app.spec()).map(|a| a.bundle_path);
    let latest = c.latest.as_ref();
    let installed_build = installed.as_deref().and_then(build);
    let available = latest
        .map(|r| {
            installed_build
                .as_deref()
                .map(|v| feed::newer(&r.build, v))
                .transpose()
        })
        .transpose()?
        .flatten()
        .unwrap_or(latest.is_some());
    let ready: Option<Release> = read_json(&root(app)?.join("ready/release.json"))?;
    let is_busy = busy(app)?;
    let interrupted = !is_busy
        && matches!(
            c.phase.as_str(),
            "checking" | "downloading" | "verifying" | "installing" | "rolling-back"
        );
    Ok(DesktopUpdate {
        app,
        installed_version: installed.as_deref().and_then(version),
        installed_path: installed.as_ref().map(|p| p.display().to_string()),
        managed: installed.as_ref() == Some(&target(app)?),
        latest_version: latest.map(|r| r.version.clone()),
        update_available: available,
        ready_version: ready.map(|r| r.version),
        previous_version: version(&root(app)?.join("previous/App.app")),
        running: running(app)?,
        busy: is_busy,
        phase: if interrupted {
            "interrupted".into()
        } else {
            c.phase
        },
        downloaded: c.downloaded,
        total: c.total,
        checked_at: c.checked_at,
        error: c.error.or_else(|| {
            interrupted.then(|| {
                "The last operation was interrupted. Check again or retry the download.".into()
            })
        }),
    })
}

fn client(app: AppKind) -> Result<reqwest::blocking::Client> {
    let settings = crate::proxy::load().map_err(|e| e.message())?;
    settings.check().map_err(|e| e.message())?;
    let mut builder = reqwest::blocking::Client::builder()
        .no_proxy()
        .user_agent("ai-profiles/1.5 desktop-updates")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(1800))
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 || !feed::allowed_url(app, attempt.url()) {
                attempt.error("Unexpected update redirect")
            } else {
                attempt.follow()
            }
        }));
    if settings.enabled {
        builder = builder.proxy(reqwest::Proxy::all(&settings.url).map_err(|e| e.to_string())?);
    }
    builder.build().map_err(|e| e.to_string())
}
fn os_version() -> Result<String> {
    let output = Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("Cannot determine macOS version".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}
fn check(app: AppKind, state: &mut Cache) -> Result<()> {
    state.phase = "checking".into();
    state.error = None;
    save(app, state)?;
    let client = client(app)?;
    let os = os_version()?;
    let request = match app {
        AppKind::Claude => {
            let device_file = root(app)?.join("device-id.json");
            let id: String =
                read_json(&device_file)?.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            write_json(&device_file, &id)?;
            // Ask for the stable full release, independently of the installed version.
            client.get(feed::CLAUDE_FEED).query(&[
                ("device_id", id.as_str()),
                ("version", "0.0.0"),
                ("os_version", os.as_str()),
            ])
        }
        AppKind::Codex => client.get(feed::CODEX_FEED),
    };
    let response = request
        .timeout(Duration::from_secs(30))
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let mut text = String::new();
    response
        .take(4 * 1024 * 1024)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    state.latest = Some(match app {
        AppKind::Claude => feed::claude(&text)?,
        AppKind::Codex => feed::codex(
            &text,
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x86_64"
            },
            &os,
        )?,
    });
    state.checked_at = Some(chrono::Utc::now().to_rfc3339());
    state.phase = "checked".into();
    save(app, state)
}

fn run(command: &mut Command) -> Result<()> {
    let output = command.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{}: {}",
            command.get_program().to_string_lossy(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}
fn verify_bundle(app: AppKind, bundle: &Path, release: Option<&Release>) -> Result<()> {
    let i = info(bundle)?;
    let (identifier, team) = match app {
        AppKind::Claude => ("com.anthropic.claudefordesktop", "Q6L2SF6YDW"),
        AppKind::Codex => ("com.openai.codex", "2DC432GLL2"),
    };
    if field(&i, "CFBundleIdentifier")? != identifier {
        return Err("Downloaded app has the wrong product identity".into());
    }
    if let Some(release) = release {
        if field(&i, "CFBundleVersion")? != release.build
            || field(&i, "CFBundleShortVersionString")? != release.version
        {
            return Err("Downloaded app version does not match the update feed".into());
        }
    }
    if let Some(min) = i
        .get("LSMinimumSystemVersion")
        .and_then(plist::Value::as_string)
    {
        if feed::newer(min, &os_version()?)? {
            return Err(format!("This release requires macOS {min} or later"));
        }
    }
    let exec = field(&i, "CFBundleExecutable")?;
    if !app
        .spec()
        .gui_bundle_candidates
        .iter()
        .any(|c| c.macos_exec == exec)
    {
        return Err("Unexpected app executable".into());
    }
    let requirement = format!("anchor apple generic and identifier \"{identifier}\" and certificate leaf[subject.OU] = \"{team}\"");
    verify_signature(bundle, &requirement)?;
    run(Command::new("lipo")
        .args([
            "-verify_arch",
            if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x86_64"
            },
        ])
        .arg(bundle.join("Contents/MacOS").join(exec)))
}

fn verify_signature(bundle: &Path, requirement: &str) -> Result<()> {
    // -R <value> reads a requirement FILE; -R=<value> parses inline text.
    run(Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(format!("-R={requirement}"))
        .arg(bundle))
}

fn download(app: AppKind, state: &mut Cache) -> Result<()> {
    check(app, state)?;
    let release = state.latest.clone().ok_or("No release available")?;
    if release.size.is_some_and(|s| s > MAX_ARCHIVE) {
        return Err("Update archive exceeds the 2 GB download limit".into());
    }
    let work = root(app)?.join(format!("download-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut response = client(app)?
            .get(&release.url)
            .send()
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?;
        state.phase = "downloading".into();
        state.downloaded = 0;
        state.total = release.size.or(response.content_length());
        save(app, state)?;
        let archive = work.join("update.zip");
        let mut file = File::create(&archive).map_err(|e| e.to_string())?;
        let mut buffer = vec![0; 256 * 1024];
        let mut last = Instant::now();
        loop {
            let n = response.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            state.downloaded += n as u64;
            if state.downloaded > MAX_ARCHIVE {
                return Err("Update archive exceeds the download limit".into());
            }
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            if last.elapsed() > Duration::from_millis(400) {
                save(app, state)?;
                last = Instant::now();
            }
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        state.phase = "verifying".into();
        save(app, state)?;
        feed::verify_archive(
            app,
            &release,
            &fs::read(&archive).map_err(|e| e.to_string())?,
        )?;
        let unpacked = work.join("unpacked");
        fs::create_dir(&unpacked).map_err(|e| e.to_string())?;
        run(Command::new("ditto")
            .args(["-x", "-k"])
            .arg(&archive)
            .arg(&unpacked))?;
        let bundles: Vec<_> = fs::read_dir(&unpacked)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "app"))
            .collect();
        if bundles.len() != 1 {
            return Err("Update must contain exactly one app bundle".into());
        }
        verify_bundle(app, &bundles[0], Some(&release))?;
        fs::rename(&bundles[0], work.join("App.app")).map_err(|e| e.to_string())?;
        fs::remove_file(&archive).map_err(|e| e.to_string())?;
        fs::remove_dir_all(&unpacked).map_err(|e| e.to_string())?;
        write_json(&work.join("release.json"), &release)?;
        let ready = root(app)?.join("ready");
        if ready.exists() {
            fs::remove_dir_all(&ready).map_err(|e| e.to_string())?;
        }
        fs::rename(&work, ready).map_err(|e| e.to_string())?;
        state.phase = "ready".into();
        save(app, state)
    })();
    if work.exists() {
        let _ = fs::remove_dir_all(&work);
    }
    result
}

/// The old bundle is restored immediately if the final rename fails.
fn replace_bundle(incoming: &Path, destination: &Path, previous: &Path) -> Result<()> {
    if previous.exists() {
        return Err("Backup destination already exists".into());
    }
    let had_old = destination.exists();
    if had_old {
        fs::rename(destination, previous).map_err(|e| e.to_string())?;
    }
    if let Err(error) = fs::rename(incoming, destination) {
        if had_old {
            fs::rename(previous, destination).map_err(|restore| {
                format!(
                    "Install failed: {error}. Restore failed: {restore}. Previous app is at {}",
                    previous.display()
                )
            })?;
        }
        return Err(error.to_string());
    }
    Ok(())
}

fn install(app: AppKind, state: &mut Cache, rollback: bool) -> Result<()> {
    require_closed(app)?;
    let dir = root(app)?;
    let source = dir.join(if rollback {
        "previous/App.app"
    } else {
        "ready/App.app"
    });
    let release: Option<Release> = if rollback {
        None
    } else {
        read_json(&dir.join("ready/release.json"))?
    };
    if !source.is_dir() || (!rollback && release.is_none()) {
        return Err("Download and verify a release before installing".into());
    }
    verify_bundle(app, &source, release.as_ref())?;
    if !rollback {
        if let (Some(current), Some(release)) = (
            paths::resolve_gui_app(app.spec()).and_then(|r| build(&r.bundle_path)),
            release.as_ref(),
        ) {
            if feed::newer(&current, &release.build)? {
                return Err(
                    "A newer version is already installed. Check for updates again.".into(),
                );
            }
        }
    }
    state.phase = if rollback {
        "rolling-back"
    } else {
        "installing"
    }
    .into();
    save(app, state)?;
    let destination = target(app)?;
    fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
    // Prepare a complete replacement while keeping ready/previous reusable on failure.
    let incoming = destination
        .parent()
        .unwrap()
        .join(format!(".incoming-{}.app", uuid::Uuid::new_v4()));
    let backup = dir.join(format!("backup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&backup).map_err(|e| e.to_string())?;
    let result = (|| {
        run(Command::new("ditto").arg(&source).arg(&incoming))?;
        verify_bundle(app, &incoming, release.as_ref())?;
        if !destination.exists() {
            if let Some(external) = paths::resolve_gui_app(app.spec()) {
                run(Command::new("ditto")
                    .arg(external.bundle_path)
                    .arg(backup.join("App.app")))?;
            }
        }
        require_closed(app)?;
        write_json(
            &dir.join("transaction.json"),
            &Transaction {
                incoming: incoming.file_name().unwrap().to_string_lossy().into_owned(),
                backup: backup.file_name().unwrap().to_string_lossy().into_owned(),
                rollback,
            },
        )?;
        replace_bundle(
            &incoming,
            &destination,
            &backup.join(if destination.exists() {
                "App.app"
            } else {
                "unused"
            }),
        )?;
        rotate_backup(&dir, &backup)?;
        if !rollback {
            fs::remove_dir_all(dir.join("ready")).map_err(|e| e.to_string())?;
        }
        state.phase = if rollback { "restored" } else { "installed" }.into();
        save(app, state)?;
        fs::remove_file(dir.join("transaction.json")).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        recover_transaction(app)?;
    }
    if incoming.exists() {
        let _ = fs::remove_dir_all(&incoming);
    }
    // A nonempty backup is intentionally retained if a later bookkeeping step fails.
    if backup.exists() && fs::read_dir(&backup).is_ok_and(|mut entries| entries.next().is_none()) {
        let _ = fs::remove_dir(&backup);
    }
    result
}

pub fn perform(action: &str, app: AppKind) -> Result<DesktopUpdate> {
    if action == "status" {
        return view(app);
    }
    let guard = lock(app)?;
    recover_transaction(app)?;
    let mut state = cache(app)?;
    state.error = None;
    let result = match action {
        "check" => check(app, &mut state),
        "download" => download(app, &mut state),
        "install" => install(app, &mut state, false),
        "rollback" => install(app, &mut state, true),
        _ => Err("Unknown desktop update action".into()),
    };
    if let Err(error) = result {
        state.error = Some(error.clone());
        state.phase = "error".into();
        let _ = save(app, &state);
        return Err(error);
    }
    drop(guard);
    view(app)
}

#[tauri::command]
pub fn get_desktop_updates() -> Result<Vec<DesktopUpdate>> {
    [AppKind::Claude, AppKind::Codex]
        .into_iter()
        .map(view)
        .collect()
}

#[tauri::command]
pub async fn desktop_update_action(app: AppKind, action: String) -> Result<DesktopUpdate> {
    tauri::async_runtime::spawn_blocking(move || perform(&action, app))
        .await
        .map_err(|e| e.to_string())?
}

pub fn cli(action: &str, app: &str) -> Result<()> {
    let app = match app {
        "claude" => AppKind::Claude,
        "codex" => AppKind::Codex,
        _ => return Err("Unknown desktop app".into()),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&perform(action, app)?).map_err(|e| e.to_string())?
    );
    Ok(())
}

pub fn start_checks() {
    std::thread::spawn(|| loop {
        for app in [AppKind::Claude, AppKind::Codex] {
            let recent = cache(app)
                .ok()
                .and_then(|c| c.checked_at)
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
                .is_some_and(|t| chrono::Utc::now().signed_duration_since(t).num_hours() < 6);
            if !recent && !busy(app).unwrap_or(true) {
                let _ = perform("check", app);
            }
        }
        std::thread::sleep(Duration::from_secs(6 * 60 * 60));
    });
}

pub(crate) fn require_not_installing(app: AppKind) -> crate::error::AppResult<()> {
    let blocked = cache(app)
        .ok()
        .is_some_and(|c| matches!(c.phase.as_str(), "installing" | "rolling-back"))
        && busy(app).unwrap_or(true);
    if blocked {
        Err(crate::error::AppError::Validation(
            "An app update is being installed. Wait for it to finish before opening a profile."
                .into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(target_os = "macos")]
    fn inline_signature_requirements_accept_valid_and_reject_wrong_identities() {
        let system_binary = Path::new("/bin/ls");
        verify_signature(system_binary, "anchor apple").unwrap();
        assert!(verify_signature(
            system_binary,
            "anchor apple and identifier \"not.the.system.binary\""
        )
        .is_err());
    }
    #[test]
    fn interrupted_replacement_restores_backup_and_releases_operation_lock() {
        let _test_guard = crate::test_support::APP_DIR_TEST_LOCK.lock().unwrap();
        let app = AppKind::Codex;
        let dir = root(app).unwrap();
        let destination = target(app).unwrap();
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        let backup = dir.join("backup-test");
        fs::create_dir_all(backup.join("App.app")).unwrap();
        fs::write(backup.join("App.app/version"), "old").unwrap();
        let incoming = destination.parent().unwrap().join(".incoming-test.app");
        fs::create_dir_all(&incoming).unwrap();
        write_json(
            &dir.join("transaction.json"),
            &Transaction {
                incoming: ".incoming-test.app".into(),
                backup: "backup-test".into(),
                rollback: false,
            },
        )
        .unwrap();
        let guard = lock(app).unwrap();
        assert!(busy(app).unwrap());
        recover_transaction(app).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join("version")).unwrap(),
            "old"
        );
        assert!(!dir.join("transaction.json").exists());
        assert!(!incoming.exists());
        drop(guard);
        assert!(!busy(app).unwrap());
        fs::remove_dir_all(&dir).unwrap();
        fs::remove_dir_all(destination).unwrap();
    }
    #[test]
    fn failure_restores_original_and_success_keeps_backup() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("Old.app");
        let previous = temp.path().join("Previous.app");
        let new = temp.path().join("New.app");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("version"), "old").unwrap();
        assert!(replace_bundle(&new, &old, &previous).is_err());
        assert_eq!(fs::read_to_string(old.join("version")).unwrap(), "old");
        fs::create_dir(&new).unwrap();
        fs::write(new.join("version"), "new").unwrap();
        replace_bundle(&new, &old, &previous).unwrap();
        assert_eq!(fs::read_to_string(old.join("version")).unwrap(), "new");
        assert_eq!(fs::read_to_string(previous.join("version")).unwrap(), "old");
    }
    #[test]
    fn detects_stock_wrappers_and_helpers_without_matching_other_apps() {
        assert!(running_in(
            AppKind::Claude,
            "/Users/a/Claude (friday).app/Contents/MacOS/Claude.bin"
        ));
        assert!(running_in(
            AppKind::Codex,
            "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"
        ));
        assert!(running_in(
            AppKind::Codex,
            "/Applications/Codex.app/Contents/MacOS/Codex Helper"
        ));
        assert!(!running_in(
            AppKind::Claude,
            "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"
        ));
    }
}
