//! GitHub transport adapter for Velopack 1.2.161 (MIT), based on its GithubSource.
//! Keep patching, verification and installation in Velopack. Bound network time and
//! include /latest so many preview releases cannot hide the latest stable release.
use semver::Version;
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::Path,
    sync::{Mutex, mpsc::Sender},
};
use velopack::{
    Error, HttpHeader, HttpOptions, VelopackAsset, VelopackAssetFeed, bundle::Manifest, download,
    sources::UpdateSource,
};
pub const REPOSITORY: &str = "https://github.com/jiazengp/powerdock";
pub const STABLE: &str = "win-x64-stable";
pub const PREVIEW: &str = "win-x64-preview";
pub const PACKAGE_ID: &str = "PowerDock";

pub(super) fn http_status(error: &Error) -> Option<u16> {
    if let Error::Network(network) = error
        && let velopack::NetworkError::Http(http) = network.as_ref()
    {
        // The pinned SDK wraps the HTTP client error without exposing a status accessor.
        return http.to_string().strip_prefix("http status: ")?.parse().ok();
    }
    None
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    prerelease: bool,
    draft: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}
pub struct Github {
    preview: bool,
    urls: Mutex<HashMap<String, String>>,
}
impl Github {
    pub fn new(preview: bool) -> Self {
        Self {
            preview,
            urls: Mutex::new(HashMap::new()),
        }
    }
    fn options(timeout: u64) -> HttpOptions {
        HttpOptions {
            TimeoutMilliseconds: timeout,
            Headers: vec![
                HttpHeader {
                    Name: "Accept".into(),
                    Value: "application/vnd.github+json".into(),
                },
                HttpHeader {
                    Name: "User-Agent".into(),
                    Value: "PowerDock-updater".into(),
                },
            ],
        }
    }
    fn get(url: &str) -> Result<String, Error> {
        let value = download::download_url_as_string(url, Some(&Self::options(10_000)))?;
        if value.len() > 8 * 1024 * 1024 {
            return Err(Error::Other("source_response_too_large".into()));
        }
        Ok(value)
    }
}
pub fn valid_asset(asset: &VelopackAsset) -> bool {
    asset.PackageId == PACKAGE_ID
        && Version::parse(&asset.Version).is_ok()
        && !asset.FileName.is_empty()
        && asset.FileName.len() <= 200
        && asset
            .FileName
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        && asset.FileName.ends_with(".nupkg")
        && asset.SHA256.len() == 64
        && asset.SHA256.bytes().all(|b| b.is_ascii_hexdigit())
        && asset.Size > 0
        && asset.Size <= 512 * 1024 * 1024
}
fn choose(mut entries: Vec<(String, VelopackAsset)>, current: &str) -> Vec<VelopackAsset> {
    let target = entries
        .iter()
        .filter(|(_, a)| a.Type.eq_ignore_ascii_case("full"))
        .filter_map(|(c, a)| Version::parse(&a.Version).ok().map(|v| (v, c.clone())))
        .max_by(|a, b| a.0.cmp(&b.0));
    let Some((_, channel)) = target else {
        return Vec::new();
    };
    entries.retain(|(c, a)| a.Type.eq_ignore_ascii_case("full") || (c == &channel && c == current));
    entries.into_iter().map(|(_, a)| a).collect()
}
impl UpdateSource for Github {
    fn get_release_feed(
        &self,
        _channel: &str,
        app: &Manifest,
        _staged_user_id: &str,
    ) -> Result<VelopackAssetFeed, Error> {
        let base = "https://api.github.com/repos/jiazengp/powerdock/releases";
        let mut releases: Vec<Release> =
            serde_json::from_str(&Self::get(&format!("{base}?per_page=100&page=1"))?)?;
        match Self::get(&format!("{base}/latest")) {
            Ok(json) => releases.push(serde_json::from_str(&json)?),
            Err(error) if http_status(&error) == Some(404) => {}
            Err(error) => return Err(error),
        }
        releases.retain(|r| !r.draft && (self.preview || !r.prerelease));
        releases.sort_by(|a, b| {
            Version::parse(b.tag_name.trim_start_matches('v'))
                .ok()
                .cmp(&Version::parse(a.tag_name.trim_start_matches('v')).ok())
        });
        let mut urls = self
            .urls
            .lock()
            .map_err(|_| Error::Other("source_busy".into()))?;
        urls.clear();
        for release in &releases {
            for asset in &release.assets {
                if asset
                    .browser_download_url
                    .starts_with(&format!("{REPOSITORY}/releases/download/"))
                {
                    urls.entry(asset.name.to_ascii_lowercase())
                        .or_insert_with(|| asset.browser_download_url.clone());
                }
            }
        }
        let mut entries = Vec::new();
        for channel in [STABLE, PREVIEW] {
            if channel == PREVIEW && !self.preview {
                continue;
            }
            let name = format!("releases.{channel}.json");
            let Some(url) = urls.get(&name).cloned() else {
                continue;
            };
            let feed: VelopackAssetFeed = serde_json::from_str(&Self::get(&url)?)?;
            if feed.Assets.len() > 2000 {
                return Err(Error::Other("source_invalid_feed".into()));
            }
            for asset in feed.Assets {
                if !valid_asset(&asset) {
                    return Err(Error::Other("source_invalid_asset".into()));
                }
                if !self.preview && !Version::parse(&asset.Version)?.pre.is_empty() {
                    continue;
                }
                entries.push((channel.to_owned(), asset));
            }
        }
        let mut seen = std::collections::HashSet::new();
        entries.retain(|(_, a)| seen.insert(a.FileName.to_ascii_lowercase()));
        Ok(VelopackAssetFeed {
            Assets: choose(entries, &app.channel),
        })
    }
    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        path: &Path,
        progress: Option<Sender<i16>>,
    ) -> Result<(), Error> {
        if !valid_asset(asset) {
            return Err(Error::Other("source_invalid_asset".into()));
        }
        let url = self
            .urls
            .lock()
            .map_err(|_| Error::Other("source_busy".into()))?
            .get(&asset.FileName.to_ascii_lowercase())
            .cloned()
            .ok_or_else(|| Error::Other("source_asset_missing".into()))?;
        download::download_url_to_file(&url, path, Some(&Self::options(240_000)), |value| {
            if let Some(tx) = &progress {
                let _ = tx.send(value);
            }
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn asset(version: &str, kind: &str) -> VelopackAsset {
        VelopackAsset {
            PackageId: PACKAGE_ID.into(),
            Version: version.into(),
            Type: kind.into(),
            FileName: format!("PowerDock-{version}-{kind}.nupkg"),
            SHA256: "a".repeat(64),
            Size: 100,
            ..Default::default()
        }
    }
    #[test]
    fn crossing_channels_never_combines_delta_chains() {
        let entries = vec![
            (STABLE.into(), asset("1.0.0", "Full")),
            (STABLE.into(), asset("1.0.0", "Delta")),
            (PREVIEW.into(), asset("1.1.0-preview.1", "Full")),
            (PREVIEW.into(), asset("1.1.0-preview.1", "Delta")),
        ];
        assert!(
            choose(entries.clone(), STABLE)
                .iter()
                .all(|a| a.Type == "Full")
        );
        assert_eq!(
            choose(entries, PREVIEW)
                .iter()
                .filter(|a| a.Type == "Delta")
                .count(),
            1
        );
    }
    #[test]
    fn feed_paths_and_identity_are_validated() {
        let mut a = asset("1.0.0", "Full");
        assert!(valid_asset(&a));
        a.FileName = "../outside.nupkg".into();
        assert!(!valid_asset(&a));
        a = asset("1.0.0", "Full");
        a.PackageId = "OtherApplication".into();
        assert!(!valid_asset(&a));
        a = asset("1.0.0", "Full");
        a.SHA256.clear();
        assert!(!valid_asset(&a));
    }
}
