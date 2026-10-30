//! RSS, Atom and JSON feeds (a site's feed, a YouTube channel, a news search), and the
//! news search used to find what is said about a target.

use anyhow::{anyhow, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub url: String,
    pub title: Option<String>,
    /// RFC 3339.
    pub published: Option<String>,
    /// The item's own text (its content or summary), as plain text.
    pub summary: Option<String>,
    /// Its picture (a media thumbnail or an image enclosure).
    pub image: Option<String>,
}

pub struct Feed {
    pub title: Option<String>,
    pub entries: Vec<Entry>,
}

pub fn parse(bytes: &[u8]) -> Result<Feed> {
    let feed = feed_rs::parser::parse(bytes).map_err(|e| anyhow!("Not a valid RSS/Atom feed: {e}"))?;
    let entries = feed
        .entries
        .into_iter()
        .filter_map(|entry| {
            let url = entry.links.first()?.href.clone();
            let image = entry
                .media
                .iter()
                .flat_map(|m| m.thumbnails.iter().map(|t| t.image.uri.clone()))
                .chain(entry.media.iter().flat_map(|m| m.content.iter()).filter_map(|c| {
                    let url = c.url.as_ref()?.to_string();
                    c.content_type.as_ref().map_or(looks_like_image(&url), |t| t.to_string().starts_with("image/")).then_some(url)
                }))
                .chain(entry.links.iter().filter(|l| l.media_type.as_deref().is_some_and(|t| t.starts_with("image/"))).map(|l| l.href.clone()))
                .find_map(|image| super::extract::absolute_image(Some(&image), &url));
            Some(Entry {
                title: entry.title.map(|t| t.content.trim().to_string()).filter(|t| !t.is_empty()),
                published: entry.published.or(entry.updated).map(|d| d.to_rfc3339()),
                summary: entry.content.and_then(|c| c.body).or(entry.summary.map(|s| s.content)).map(|html| plain_text(&html)).filter(|t| !t.is_empty()),
                image,
                url,
            })
        })
        .collect();
    Ok(Feed { title: feed.title.map(|t| t.content), entries })
}

/// A picture's address by its extension (media:content often says only medium="image").
fn looks_like_image(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or_default().to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".webp", ".gif", ".avif"].iter().any(|ext| path.ends_with(ext))
}

/// The feed of a social profile: Bluesky and Mastodon publish one; a feed address (e.g. from
/// an RSSHub) is kept as is. X, Instagram, Facebook and TikTok publish none.
pub fn social_feed_url(profile: &str) -> Result<String, String> {
    let with_scheme = if profile.contains("://") { profile.trim().to_string() } else { format!("https://{}", profile.trim()) };
    let url = url::Url::parse(&with_scheme).map_err(|_| format!("Not a profile address: {profile}"))?;
    let host = url.host_str().unwrap_or_default().trim_start_matches("www.").to_ascii_lowercase();
    let path = url.path().trim_end_matches('/');
    if ["x.com", "twitter.com", "instagram.com", "facebook.com", "tiktok.com", "threads.net", "threads.com"].contains(&host.as_str()) {
        return Err(format!("{host} does not publish feeds: paste the address of an RSSHub feed for this profile"));
    }
    if host == "bsky.app" && path.starts_with("/profile/") && !path.ends_with("/rss") {
        return Ok(format!("https://bsky.app{path}/rss"));
    }
    // Mastodon (and other Fediverse servers): https://server/@user → https://server/@user.rss
    if path.starts_with("/@") && !path.contains('.') && path.matches('/').count() == 1 {
        return Ok(format!("{}://{}{path}.rss", url.scheme(), url.host_str().unwrap_or_default()));
    }
    Ok(url.to_string())
}

/// The text of an HTML fragment.
pub fn plain_text(html: &str) -> String {
    scraper::Html::parse_fragment(html).root_element().text().map(str::trim).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Bing News' RSS for `query`, in Brazilian Portuguese.
///
/// ponytail: Bing's terms allow these results in a personal, non-commercial aggregator only;
/// a licensed news/search API (phase 5, discover/websearch) replaces it for commercial use.
pub fn news_search_url(query: &str) -> String {
    let mut url = url::Url::parse("https://www.bing.com/news/search").expect("valid URL");
    url.query_pairs_mut().append_pair("q", query).append_pair("format", "rss").append_pair("setlang", "pt-BR").append_pair("cc", "BR");
    url.to_string()
}

/// Bing News links go through a click tracker; the article's address is its `url` parameter.
pub fn unwrap_news_link(link: &str) -> String {
    url::Url::parse(link)
        .ok()
        .filter(|u| u.host_str().is_some_and(|h| h.ends_with("bing.com")))
        .and_then(|u| u.query_pairs().find(|(k, _)| k == "url").map(|(_, v)| v.into_owned()))
        .unwrap_or_else(|| link.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn social_profiles_become_feeds() {
        assert_eq!(social_feed_url("https://bsky.app/profile/marina.bsky.social").unwrap(), "https://bsky.app/profile/marina.bsky.social/rss");
        assert_eq!(social_feed_url("mastodon.social/@marina").unwrap(), "https://mastodon.social/@marina.rss");
        assert_eq!(social_feed_url("https://rsshub.example/twitter/user/marina").unwrap(), "https://rsshub.example/twitter/user/marina");
        assert!(social_feed_url("https://x.com/marina").unwrap_err().contains("RSSHub"));
    }

    #[test]
    fn reads_rss_and_atom_entries() {
        let rss = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Jornal</title>
            <item><title>Votação adiada</title><link>https://jornal.example/a</link><pubDate>Fri, 09 Oct 2026 19:55:55 GMT</pubDate></item>
            <item><title> </title><link>https://jornal.example/b</link></item></channel></rss>"#;
        let feed = parse(rss.as_bytes()).unwrap();
        assert_eq!(feed.title.as_deref(), Some("Jornal"));
        assert_eq!(
            feed.entries[0],
            Entry { url: "https://jornal.example/a".into(), title: Some("Votação adiada".into()), published: Some("2026-10-09T19:55:55+00:00".into()), summary: None, image: None }
        );
        assert_eq!(feed.entries[1].title, None);

        let atom = br#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Canal</title>
            <entry><id>yt:video:abc</id><title>Discurso</title><link rel="alternate" href="https://www.youtube.com/watch?v=abc"/>
            <published>2026-10-13T21:45:33+00:00</published><updated>2026-10-14T00:30:15+00:00</updated></entry></feed>"#;
        let feed = parse(atom).unwrap();
        assert_eq!(feed.entries[0].url, "https://www.youtube.com/watch?v=abc");
        assert_eq!(feed.entries[0].published.as_deref(), Some("2026-10-13T21:45:33+00:00"));
        assert!(parse(b"<html>not a feed</html>").is_err());
    }

    #[test]
    fn entries_bring_their_picture() {
        let rss = r#"<?xml version="1.0"?><rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/"><channel><title>Jornal</title>
            <item><title>Com mídia</title><link>https://jornal.example/a</link><media:content url="https://img.example/a.jpg?w=800" medium="image"/></item>
            <item><title>Com anexo</title><link>https://jornal.example/b</link><enclosure url="https://img.example/b.png" type="image/png" length="1"/></item>
            <item><title>Com áudio</title><link>https://jornal.example/c</link><enclosure url="https://img.example/c.mp3" type="audio/mpeg" length="1"/></item>
            </channel></rss>"#;
        let feed = parse(rss.as_bytes()).unwrap();
        let images: Vec<Option<&str>> = feed.entries.iter().map(|e| e.image.as_deref()).collect();
        assert_eq!(images, vec![Some("https://img.example/a.jpg?w=800"), Some("https://img.example/b.png"), None]);
    }

    #[test]
    fn news_links_are_unwrapped() {
        let link = "http://www.bing.com/news/apiclick.aspx?ref=FexRss&aid=&tid=x&url=https%3a%2f%2foglobo.globo.com%2fpolitica%2fnoticia.ghtml&c=1";
        assert_eq!(unwrap_news_link(link), "https://oglobo.globo.com/politica/noticia.ghtml");
        assert_eq!(unwrap_news_link("https://jornal.example/a"), "https://jornal.example/a");
        assert!(news_search_url("\"Tabata Amaral\"").contains("q=%22Tabata+Amaral%22"));
    }
}
