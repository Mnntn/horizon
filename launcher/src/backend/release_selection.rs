//! Resolve a published release's channel or exact installed version.
use super::Release;

pub(super) fn by_tag<'a>(releases: &'a [Release], tag: &str) -> Option<&'a Release> {
    releases
        .iter()
        .find(|release| release.tag == tag || release.nightly.as_deref() == Some(tag))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::game_releases_with;

    fn published(channel: &str) -> Vec<Release> {
        game_releases_with(
            &serde_json::json!([{
                "tag_name": channel,
                "name": "Nightly 2026-11-10 · 9f13812",
                "published_at": "2026-11-10T17:07:46Z",
                "prerelease": true,
                "assets": [{"name": "horizon-linux-x86_64.tar.gz", "size": 123}]
            }]),
            Some("horizon-linux-x86_64.tar.gz"),
            None,
        )
    }

    #[test]
    fn channel_and_exact_version_select_the_same_published_build() {
        for channel in ["nightly", "nightly-2026-11-10"] {
            let releases = published(channel);
            for requested in [channel, "nightly-2026-11-10-9f13812"] {
                let release = by_tag(&releases, requested).unwrap();
                assert_eq!(release.tag, "nightly-2026-11-10-9f13812");
                assert_eq!(release.nightly.as_deref(), Some(channel));
                assert_eq!(
                    release.asset,
                    Some((format!("https://github.com/arismoko/horizon/releases/download/{channel}/horizon-linux-x86_64.tar.gz"), 123))
                );
            }
        }
    }

    #[test]
    fn an_unknown_tag_never_falls_back_to_an_available_build() {
        let releases = published("nightly");
        assert!(by_tag(&releases, "nightly-2026-11-09").is_none());
        assert!(by_tag(&releases, "nightly-2026-11-10-deadbee").is_none());
        assert!(by_tag(&[], "nightly").is_none());
    }
}
