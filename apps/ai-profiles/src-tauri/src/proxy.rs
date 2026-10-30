//! Local desktop proxy settings. No background traffic supervision.
use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::{TcpStream, ToSocketAddrs},
    process::Command,
    time::Duration,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySettings {
    pub enabled: bool,
    pub url: String,
    pub no_proxy: String,
}

impl ProxySettings {
    fn endpoint(&self) -> AppResult<reqwest::Url> {
        let url = reqwest::Url::parse(&self.url).map_err(|_| {
            AppError::Validation("Enter an HTTP proxy URL, such as http://host:3128".into())
        })?;
        if url.scheme() != "http"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(AppError::Validation(
                "Use an HTTP proxy URL without credentials, a path or query".into(),
            ));
        }
        if self
            .no_proxy
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, ';' | '<' | '>'))
        {
            return Err(AppError::Validation(
                "Use comma-separated hosts in the bypass list".into(),
            ));
        }
        Ok(url)
    }

    pub fn check(&self) -> AppResult<()> {
        if !self.enabled {
            return Ok(());
        }
        let url = self.endpoint()?;
        let host = url.host_str().unwrap().trim_matches(['[', ']']);
        let port = url.port_or_known_default().unwrap();
        let peers = (host, port).to_socket_addrs()?;
        for peer in peers.take(4) {
            if TcpStream::connect_timeout(&peer, Duration::from_secs(3)).is_ok() {
                return Ok(());
            }
        }
        Err(AppError::Validation(format!("Proxy {host}:{port} is unreachable. Connect to the network/VPN or change Settings → Desktop proxy.")))
    }

    pub fn apply(&self, command: &mut Command) {
        if !self.enabled {
            return;
        }
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env(key, &self.url);
        }
        command
            .env("NO_PROXY", &self.no_proxy)
            .env("no_proxy", &self.no_proxy)
            .env("NODE_USE_ENV_PROXY", "1")
            .arg(format!("--proxy-server={}", self.url))
            .arg(format!(
                "--proxy-bypass-list={}",
                self.no_proxy.replace(',', ";")
            ));
    }
}

/// Imported multiprofile policies pin a native egress URL as well as env vars.
/// Keep only these explicitly imported copies aligned with the launch settings.
pub fn prepare_imported(data_dir: &std::path::Path, settings: &ProxySettings) -> AppResult<()> {
    let Some(root) = data_dir.parent() else {
        return Ok(());
    };
    if !root.join("multiprofile-import.json").is_file() {
        return Ok(());
    }
    let library = root.join("gui-data-3p/configLibrary");
    let meta: serde_json::Value = serde_json::from_slice(&fs::read(library.join("_meta.json"))?)?;
    let id = meta["appliedId"]
        .as_str()
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .ok_or_else(|| AppError::Validation("Invalid imported Claude policy".into()))?;
    let file = library.join(format!("{id}.json"));
    let mut policy: serde_json::Value = serde_json::from_slice(&fs::read(&file)?)?;
    let object = policy
        .as_object_mut()
        .ok_or_else(|| AppError::Validation("Invalid Claude policy object".into()))?;
    object.insert("disableAutoUpdates".into(), true.into());
    if settings.enabled {
        object.insert("egressProxyUrl".into(), settings.url.clone().into());
    } else {
        object.remove("egressProxyUrl");
    }
    crate::profiles::atomic_write(&file, &serde_json::to_vec_pretty(&policy)?)?;
    let file = root.join("cli-config/settings.json");
    let mut config: serde_json::Value = serde_json::from_slice(&fs::read(&file)?)?;
    let env = config
        .as_object_mut()
        .ok_or_else(|| AppError::Validation("Invalid Code settings".into()))?
        .entry("env")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| AppError::Validation("Invalid Code environment".into()))?;
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
        "NODE_USE_ENV_PROXY",
    ] {
        if settings.enabled {
            let value = match key {
                "NO_PROXY" | "no_proxy" => &settings.no_proxy,
                "NODE_USE_ENV_PROXY" => "1",
                _ => &settings.url,
            };
            env.insert(key.into(), value.into());
        } else {
            env.remove(key);
        }
    }
    crate::profiles::atomic_write(&file, &serde_json::to_vec_pretty(&config)?)
}

pub fn load() -> AppResult<ProxySettings> {
    let path = crate::paths::app_data_dir()?.join("desktop-proxy.json");
    match fs::read(&path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ProxySettings::default()),
        Err(e) => Err(e.into()),
    }
}

/// Read by the wrapper shim on every Dock/Finder launch. A failed preflight
/// returns no launch configuration, so the shim cannot silently launch direct.
pub fn prepare_profile_launch(id: &str) -> Result<(), String> {
    let profiles = crate::profiles::load().map_err(|e| e.message())?;
    let profile = profiles
        .iter()
        .find(|p| p.id == id && p.surfaces.gui)
        .ok_or_else(|| "Unknown desktop profile".to_string())?;
    crate::desktop_updates::require_not_installing(profile.app).map_err(|e| e.message())?;
    let settings = load().map_err(|e| e.message())?;
    settings.check().map_err(|e| e.message())?;
    let data = crate::paths::profile_dir(&profile.id)
        .map_err(|e| e.message())?
        .join("gui-data");
    prepare_imported(&data, &settings).map_err(|e| e.message())?;
    let mut command = Command::new("unused");
    if profile.app == crate::app_kind::AppKind::Codex {
        command.env("CODEX_SPARKLE_ENABLED", "false");
    }
    settings.apply(&mut command);
    let mut config = plist::Dictionary::new();
    config.insert(
        "args".into(),
        plist::Value::Array(
            command
                .get_args()
                .map(|a| plist::Value::String(a.to_string_lossy().into_owned()))
                .collect(),
        ),
    );
    let env: plist::Dictionary = command
        .get_envs()
        .filter_map(|(k, v)| {
            v.map(|v| {
                (
                    k.to_string_lossy().into_owned(),
                    plist::Value::String(v.to_string_lossy().into_owned()),
                )
            })
        })
        .collect();
    config.insert("env".into(), plist::Value::Dictionary(env));
    plist::Value::Dictionary(config)
        .to_writer_xml(std::io::stdout())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_desktop_proxy() -> AppResult<ProxySettings> {
    load()
}

#[tauri::command]
pub async fn save_desktop_proxy(settings: ProxySettings) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        if settings.enabled {
            settings.endpoint()?;
        }
        crate::profiles::atomic_write(
            &crate::paths::app_data_dir()?.join("desktop-proxy.json"),
            &serde_json::to_vec_pretty(&settings)?,
        )
    })
    .await
    .map_err(|e| AppError::Validation(e.to_string()))?
}

#[tauri::command]
pub async fn test_desktop_proxy(settings: ProxySettings) -> AppResult<()> {
    tauri::async_runtime::spawn_blocking(move || settings.check())
        .await
        .map_err(|e| AppError::Validation(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_credentials_and_invalid_bypass() {
        for url in [
            "http://user:pass@localhost:3128",
            "socks5://localhost:1080",
            "http://localhost/path",
        ] {
            assert!(ProxySettings {
                enabled: true,
                url: url.into(),
                no_proxy: "localhost".into()
            }
            .endpoint()
            .is_err());
        }
    }
    #[test]
    fn unreachable_proxy_refuses_launch() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let settings = ProxySettings {
            enabled: true,
            url: format!("http://127.0.0.1:{port}"),
            no_proxy: "localhost,127.0.0.1,::1".into(),
        };
        assert!(settings.check().is_ok());
        drop(listener);
        assert!(settings.check().is_err());
        let mut command = Command::new("open");
        settings.apply(&mut command);
        assert!(command
            .get_args()
            .any(|a| a == "--proxy-bypass-list=localhost;127.0.0.1;::1"));
    }
}
