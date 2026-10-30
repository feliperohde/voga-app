//! Following links: the pages a source brings (its page, the articles of its feed) have their
//! links to the same site queued, up to the investigation's depth and daily page limit.
//! A link whose text or address names someone followed goes in; a headline that names no one
//! is ambiguous, and only then the AI is asked (one request per page, within the daily limit).
//! Pages reached this way are kept only if they mention someone followed (process_item).

use anyhow::Result;
use chrono::Utc;
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::collect::extract::Link;
use crate::collect::{contains_phrase, fold, investigation_names, schedule_time};
use crate::llm::ChatModel;

/// Ambiguous links shown to the AI per page.
const MAX_ASKED: usize = 40;
/// Words a link's text needs to read as a headline (shorter ones are menus: "Política", "Entrar").
const HEADLINE_WORDS: usize = 4;

const SYSTEM: &str = "You choose, among the links of a web page, the ones likely to lead to news, speeches, interviews, documents or records about the people and topics an investigation follows (what they promised, said, voted, spent or did; for a subject, news, research and developments on it). Leave out menus, other subjects, ads and generic sections. Reply with ONLY a JSON object: {\"relevant\": [the numbers of the chosen links]}.";

/// Which model chooses among ambiguous links.
pub enum Triage<'a> {
    /// The one in Settings › AI, loaded only when there is something to ask.
    Configured,
    With(&'a dyn ChatModel),
}

/// The page whose links are followed.
pub struct From<'a> {
    pub investigation_id: &'a str,
    pub source_id: &'a str,
    /// Links away from the source (its own page or feed article: 0).
    pub depth: i64,
    pub page_url: &'a str,
}

fn site(url: &url::Url) -> Option<String> {
    url.host_str().map(|h| h.trim_start_matches("www.").to_ascii_lowercase())
}

/// Addresses that are never articles.
fn skipped_path(path: &str) -> bool {
    const FILES: [&str; 14] = [".jpg", ".jpeg", ".png", ".gif", ".svg", ".webp", ".ico", ".css", ".js", ".zip", ".rar", ".mp3", ".mp4", ".xml"];
    const SECTIONS: [&str; 16] = [
        "login", "entrar", "signin", "cadastro", "assine", "assinatura", "subscribe", "newsletter", "privacidade", "privacy", "termos-de-uso", "terms",
        "fale-conosco", "contato", "/tag/", "/tags/",
    ];
    let path = path.to_ascii_lowercase();
    path.trim_matches('/').is_empty() || FILES.iter().any(|ext| path.ends_with(ext)) || SECTIONS.iter().any(|s| path.contains(s))
}

/// Article addresses have a long slug ("deputada-vota-contra-a-reforma"), a number (an id, a
/// date) or a document extension; section pages ("/noticias", "/politica") have none of these.
fn article_like(path: &str) -> bool {
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
    last.matches('-').count() >= 2
        || path.chars().filter(char::is_ascii_digit).count() >= 4
        || [".html", ".htm", ".ghtml", ".shtml", ".pdf"].iter().any(|ext| last.ends_with(ext))
}

/// The links worth a look: same site as the page, not the page itself, articles (not files or sections).
pub fn candidates<'a>(page_url: &str, links: &'a [Link]) -> Vec<&'a Link> {
    let Ok(page) = url::Url::parse(page_url) else { return Vec::new() };
    let Some(page_site) = site(&page) else { return Vec::new() };
    let page_url = crate::collect::canonical_url(page_url);
    links
        .iter()
        .filter(|link| {
            let Ok(url) = url::Url::parse(&link.url) else { return false };
            site(&url).as_deref() == Some(page_site.as_str()) && link.url != page_url && !skipped_path(url.path()) && article_like(url.path())
        })
        .collect()
}

/// Whether the link's text or address names someone followed ("…/tabata-amaral-vota…").
pub fn names_someone(link: &Link, names: &[String]) -> bool {
    let path = url::Url::parse(&link.url).map(|u| u.path().replace(['-', '_', '/', '.'], " ")).unwrap_or_default();
    let haystack = fold(&format!("{} {}", link.text, path));
    names.iter().any(|name| contains_phrase(&haystack, name))
}

fn is_headline(link: &Link) -> bool {
    link.text.split_whitespace().count() >= HEADLINE_WORDS
}

fn parse_relevant(reply: &str, asked: usize) -> Vec<usize> {
    #[derive(Deserialize)]
    struct Reply {
        #[serde(default)]
        relevant: Vec<usize>,
    }
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else { return Vec::new() };
    let Ok(parsed) = serde_json::from_str::<Reply>(reply.get(start..=end).unwrap_or_default()) else { return Vec::new() };
    let mut picked: Vec<usize> = parsed.relevant.into_iter().filter(|n| (1..=asked).contains(n)).map(|n| n - 1).collect();
    picked.dedup();
    picked
}

/// Asks the AI which ambiguous links are about the investigation.
async fn triage<'a>(pool: &SqlitePool, llm: &dyn ChatModel, investigation_id: &str, ambiguous: &[&'a Link]) -> Result<Vec<&'a Link>> {
    let asked = &ambiguous[..ambiguous.len().min(MAX_ASKED)];
    let list = asked.iter().enumerate().map(|(i, l)| format!("[{}] {}", i + 1, l.text)).collect::<Vec<_>>().join("\n");
    let brief = super::brief(pool, investigation_id).await?;
    let reply = llm.complete(SYSTEM, &format!("{brief}\n\n<links>\n{list}\n</links>")).await?;
    Ok(parse_relevant(&reply, asked.len()).into_iter().map(|i| asked[i]).collect())
}

/// Queues the links of a page that are worth following. Returns how many were queued.
pub async fn follow(pool: &SqlitePool, triage_with: Triage<'_>, from: &From<'_>, links: &[Link]) -> Result<usize> {
    let limits = super::limits(pool, from.investigation_id).await?;
    if from.depth >= limits.depth {
        return Ok(0);
    }
    let (pages_left, ai_left) = super::left(pool, from.investigation_id).await?;
    if pages_left == 0 {
        return Ok(0);
    }
    let names = investigation_names(pool, from.investigation_id).await?;
    let candidates = candidates(from.page_url, links);
    let (mut chosen, others): (Vec<&Link>, Vec<&Link>) = candidates.into_iter().partition(|link| names_someone(link, &names));
    let ambiguous: Vec<&Link> = others.into_iter().filter(|l| is_headline(l)).collect();
    if (chosen.len() as i64) < pages_left && !ambiguous.is_empty() && ai_left > 0 {
        let configured;
        let llm: Option<&dyn ChatModel> = match triage_with {
            Triage::With(llm) => Some(llm),
            Triage::Configured => {
                configured = super::model(pool).await;
                configured.as_ref().map(|m| m as &dyn ChatModel)
            }
        };
        if let Some(llm) = llm {
            match triage(pool, llm, from.investigation_id, &ambiguous).await {
                Ok(picked) => chosen.extend(picked),
                Err(e) => log::warn!("Discover: could not choose links of {}: {e}", from.page_url),
            }
            super::spend(pool, from.investigation_id, 0, 1).await?;
        }
    }
    chosen.truncate(pages_left as usize);

    let now = schedule_time(Utc::now());
    let mut queued = 0;
    for link in chosen {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO crawl_queue (investigation_id, source_id, url, kind, title, via, depth, next_attempt_at, created_at)
             SELECT ?1, ?2, ?3, 'article', ?4, 'link', ?5, ?6, ?6
             WHERE NOT EXISTS (SELECT 1 FROM documents WHERE investigation_id = ?1 AND url = ?3)",
        )
        .bind(from.investigation_id)
        .bind(from.source_id)
        .bind(&link.url)
        .bind(Some(link.text.as_str()).filter(|t| !t.is_empty()))
        .bind(from.depth + 1)
        .bind(&now)
        .execute(pool)
        .await?;
        queued += result.rows_affected() as usize;
    }
    if queued > 0 {
        super::spend(pool, from.investigation_id, queued as i64, 0).await?;
        log::info!("Discover: {queued} links queued from {}", from.page_url);
    }
    Ok(queued)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(url: &str, text: &str) -> Link {
        Link { url: url.into(), text: text.into() }
    }

    #[test]
    fn only_same_site_article_links_are_candidates() {
        let links = [
            link("https://jornal.example/politica/deputada-vota-contra", "Deputada vota contra"),
            link("https://www.jornal.example/economia/juros", "Juros sobem de novo neste mês"),
            link("https://outro.example/politica/x", "Outro site"),
            link("https://jornal.example/", "Início"),
            link("https://jornal.example/login", "Entrar"),
            link("https://jornal.example/tag/camara", "Câmara"),
            link("https://jornal.example/foto.jpg", "Foto"),
            link("https://jornal.example/home", "A própria página"),
            link("https://jornal.example/noticias", "Agência de Notícias"),
            link("https://jornal.example/2023/07/06/votacao", "Votação de ontem"),
            link("https://jornal.example/docs/plano.pdf", "Plano de governo"),
        ];
        let found: Vec<&str> = candidates("https://jornal.example/home", &links).iter().map(|l| l.url.as_str()).collect();
        assert_eq!(
            found,
            [
                "https://jornal.example/politica/deputada-vota-contra",
                "https://jornal.example/2023/07/06/votacao",
                "https://jornal.example/docs/plano.pdf"
            ]
        );
    }

    #[test]
    fn names_in_the_text_or_the_address_count() {
        let names = vec![fold("Tabata Amaral")];
        assert!(names_someone(&link("https://j.example/a", "Tábata Amaral vota contra"), &names));
        assert!(names_someone(&link("https://j.example/politica/tabata-amaral-defende-creches", "Leia mais"), &names));
        assert!(!names_someone(&link("https://j.example/politica/juros", "Juros sobem"), &names));
        assert!(is_headline(&link("https://j.example/x", "Câmara aprova reforma em segundo turno")));
        assert!(!is_headline(&link("https://j.example/x", "Política")));
    }

    struct PicksSecond;

    #[async_trait::async_trait]
    impl ChatModel for PicksSecond {
        async fn complete(&self, _system: &str, user: &str) -> Result<String> {
            assert!(user.contains("Marina Exemplo"), "the AI is told who is followed: {user}");
            Ok(r#"{"relevant": [2]}"#.into())
        }
    }

    async fn pool() -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'Mandato', '2026')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t1', 'inv', 'person', 'Marina Exemplo', '2026')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO sources (id, investigation_id, kind, url, next_run_at, follow_links, created_at) VALUES ('s1', 'inv', 'page', 'https://j.example/', '2026', 1, '2026')")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn links_are_queued_within_the_limits() {
        let pool = pool().await;
        let links = [
            link("https://j.example/politica/marina-exemplo-vota", "Leia"),
            link("https://j.example/economia/juros-sobem-de-novo", "Juros sobem de novo neste mês"),
            link("https://j.example/cidades/obras-na-zona-norte-atrasam", "Obras na zona norte atrasam outra vez"),
            link("https://j.example/politica", "Política"),
        ];
        let from = From { investigation_id: "inv", source_id: "s1", depth: 0, page_url: "https://j.example/" };
        let queued = follow(&pool, Triage::With(&PicksSecond), &from, &links).await.unwrap();
        // The one naming her, and the headline the AI picked
        assert_eq!(queued, 2);
        let rows: Vec<(String, String, i64)> = sqlx::query_as("SELECT url, via, depth FROM crawl_queue ORDER BY id").fetch_all(&pool).await.unwrap();
        assert_eq!(rows[0], ("https://j.example/politica/marina-exemplo-vota".into(), "link".into(), 1));
        assert_eq!(rows[1].0, "https://j.example/cidades/obras-na-zona-norte-atrasam");
        let used = super::super::usage(&pool, "inv").await.unwrap();
        assert_eq!((used.pages, used.ai_calls), (2, 1));

        // Pages at the investigation's depth (1 by default) have their links left alone
        let deeper = From { depth: 1, ..from };
        assert_eq!(follow(&pool, Triage::With(&PicksSecond), &deeper, &links).await.unwrap(), 0);

        // The daily page limit holds
        sqlx::query("UPDATE investigations SET discovery = '{\"pagesPerDay\": 3}'").execute(&pool).await.unwrap();
        let more = [link("https://j.example/politica/marina-exemplo-discursa", "x"), link("https://j.example/politica/marina-exemplo-viaja", "y")];
        assert_eq!(follow(&pool, Triage::With(&PicksSecond), &from, &more).await.unwrap(), 1);
        assert_eq!(follow(&pool, Triage::With(&PicksSecond), &from, &more).await.unwrap(), 0);
    }

    #[test]
    fn the_ai_reply_is_read_safely() {
        assert_eq!(parse_relevant("```json\n{\"relevant\": [2, 5, 9, 0]}\n```", 5), vec![1, 4]);
        assert!(parse_relevant("nenhum", 5).is_empty());
    }
}
