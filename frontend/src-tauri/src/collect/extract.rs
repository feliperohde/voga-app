//! The readable part of a web page as Markdown (Mozilla Readability's algorithm, via
//! dom_smoothie), with its title, author, site and publication date.

use dom_smoothie::{Config, Readability, TextMode};

#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    pub title: String,
    pub author: Option<String>,
    pub site: Option<String>,
    /// RFC 3339 or YYYY-MM-DD.
    pub published: Option<String>,
    pub lang: Option<String>,
    pub text: String,
    /// The picture the page shares itself with (og:image), absolute.
    pub image: Option<String>,
}

/// Turns the dates pages put in their metadata into RFC 3339 (or a bare date).
pub fn normalize_date(text: &str) -> Option<String> {
    let text = text.trim();
    if let Ok(date) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(date.with_timezone(&chrono::Utc).to_rfc3339());
    }
    if let Ok(date) = chrono::DateTime::parse_from_rfc2822(text) {
        return Some(date.with_timezone(&chrono::Utc).to_rfc3339());
    }
    let day = text.get(..10)?;
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok().map(|d| d.to_string())
}

fn non_empty(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// Plain text of the whole body, for pages Readability cannot make sense of.
fn fallback(html: &str) -> Extracted {
    let document = scraper::Html::parse_document(html);
    let pick = |selector: &str| scraper::Selector::parse(selector).ok().and_then(|s| document.select(&s).next());
    let title = pick("title").map(|t| t.text().collect::<String>()).unwrap_or_default();
    let text = pick("body")
        .map(|body| body.text().map(str::trim).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    Extracted { title: title.trim().to_string(), author: None, site: None, published: None, lang: None, text, image: None }
}

/// `image` as an absolute http(s) address, relative to the page at `base`.
pub fn absolute_image(image: Option<&str>, base: &str) -> Option<String> {
    let image = image?.trim();
    let url = url::Url::parse(base).ok()?.join(image).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

pub fn article(html: &str, url: &str) -> Extracted {
    let config = Config { text_mode: TextMode::Markdown, ..Default::default() };
    let parsed = Readability::new(html, Some(url), Some(config)).and_then(|mut readability| readability.parse());
    match parsed {
        Ok(article) if !article.text_content.trim().is_empty() => Extracted {
            title: article.title.trim().to_string(),
            author: non_empty(article.byline),
            site: non_empty(article.site_name),
            published: article.published_time.as_deref().and_then(normalize_date),
            lang: non_empty(article.lang),
            text: article.text_content.trim().to_string(),
            image: absolute_image(article.image.as_deref(), url),
        },
        Ok(_) => fallback(html),
        Err(e) => {
            log::info!("Extract: Readability gave up on {url} ({e}); using the page text");
            fallback(html)
        }
    }
}

/// A link on a page: where it goes and what it says.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub url: String,
    pub text: String,
}

/// The page's web links, absolute, once each (the first text found for each address).
pub fn links(html: &str, base: &str) -> Vec<Link> {
    let Ok(base) = url::Url::parse(base) else { return Vec::new() };
    let document = scraper::Html::parse_document(html);
    let Ok(anchors) = scraper::Selector::parse("a[href]") else { return Vec::new() };
    let mut seen = std::collections::HashSet::new();
    let mut found = Vec::new();
    for anchor in document.select(&anchors) {
        let Some(href) = anchor.value().attr("href") else { continue };
        let Ok(mut url) = base.join(href.trim()) else { continue };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        url.set_fragment(None);
        let url = super::canonical_url(url.as_str());
        let text = anchor
            .text()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
            // Many links have only an image: its alt or the title attribute say what it is
            .chars()
            .take(300)
            .collect::<String>();
        let text = if text.is_empty() { anchor.value().attr("title").unwrap_or_default().trim().to_string() } else { text };
        if seen.insert(url.clone()) {
            found.push(Link { url, text });
        }
    }
    found
}

/// The text of a PDF. Parsing runs on a blocking thread and a malformed file that makes the
/// parser panic becomes an error.
pub async fn pdf_text(bytes: Vec<u8>) -> anyhow::Result<String> {
    let text = tokio::task::spawn_blocking(move || std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(&bytes)))
        .await?
        .map_err(|_| anyhow::anyhow!("The PDF could not be read"))?
        .map_err(|e| anyhow::anyhow!("The PDF could not be read: {e}"))?;
    // Keep paragraphs, drop the runs of blank lines and spaces PDFs are full of
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")) {
        if line.is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        out.push_str(&line);
        blank = 0;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<html lang="pt-BR"><head>
        <title>Deputada vota contra reforma | Jornal Exemplo</title>
        <meta property="og:site_name" content="Jornal Exemplo">
        <meta property="article:published_time" content="2023-07-06T21:30:00-03:00">
        <meta name="author" content="Maria Repórter">
        <meta property="og:image" content="/fotos/plenario.jpg">
        </head><body>
        <nav><a href="/">Início</a> <a href="/politica">Política</a></nav>
        <article><h1>Deputada vota contra reforma</h1>
        <p>A deputada votou contra o texto-base da reforma tributária nesta quinta-feira, contrariando o que havia defendido durante a campanha de 2022, quando prometeu apoiar a simplificação dos impostos.</p>
        <p>Em nota, a assessoria afirmou que a mudança de posição se deve a alterações feitas no texto pelo relator, que segundo ela prejudicam estados do Nordeste e aumentam a carga sobre serviços.</p>
        <p>O texto foi aprovado por 382 votos a 118 e segue para o Senado, onde deve ser votado ainda neste semestre, segundo líderes da base do governo ouvidos pela reportagem.</p>
        </article><footer>© Jornal Exemplo</footer></body></html>"#;

    #[test]
    fn extracts_the_article_and_its_metadata() {
        let extracted = article(PAGE, "https://jornal.example/politica/deputada-vota");
        assert!(extracted.title.contains("Deputada vota contra reforma"), "{}", extracted.title);
        assert_eq!(extracted.site.as_deref(), Some("Jornal Exemplo"));
        assert_eq!(extracted.published.as_deref(), Some("2023-07-07T00:30:00+00:00"));
        assert!(extracted.text.contains("382 votos a 118"));
        assert_eq!(extracted.image.as_deref(), Some("https://jornal.example/fotos/plenario.jpg"));
        assert!(!extracted.text.contains("© Jornal Exemplo"), "footer leaked: {}", extracted.text);
    }

    #[test]
    fn links_are_absolute_and_unique() {
        let html = r##"<body><a href="/politica/a-1">Deputada <b>vota</b></a> <a href="https://outro.example/b#x">Outro</a>
            <a href="/politica/a-1?utm_source=home">Repetido</a> <a href="mailto:x@y.z">E-mail</a> <a href="#topo">Topo</a>
            <a href="/foto" title="Galeria"><img src="f.jpg"></a></body>"##;
        let found = links(html, "https://jornal.example/home");
        assert_eq!(found[0], Link { url: "https://jornal.example/politica/a-1".into(), text: "Deputada vota".into() });
        assert_eq!(found[1].url, "https://outro.example/b");
        assert!(found.iter().all(|l| l.url.starts_with("http")));
        assert_eq!(found.iter().filter(|l| l.url.ends_with("/politica/a-1")).count(), 1);
        assert_eq!(found.iter().find(|l| l.url.ends_with("/foto")).unwrap().text, "Galeria");
        // The page's own anchor (#topo) is the page itself
        assert!(found.iter().any(|l| l.url == "https://jornal.example/home"));
    }

    #[test]
    fn dates_are_normalized() {
        assert_eq!(normalize_date("Fri, 09 Oct 2026 19:55:55 GMT").as_deref(), Some("2026-10-09T19:55:55+00:00"));
        assert_eq!(normalize_date("2022-08-16 10:00").as_deref(), Some("2022-08-16"));
        assert_eq!(normalize_date("ontem"), None);
    }

    #[test]
    fn unreadable_pages_fall_back_to_their_text() {
        let extracted = article("<html><head><title>Agenda</title></head><body><p>Sessão às 14h</p></body></html>", "https://x.example/");
        assert_eq!(extracted.title, "Agenda");
        assert!(extracted.text.contains("Sessão às 14h"));
    }
}
