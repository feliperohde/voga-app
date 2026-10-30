//! Extraction: the promises and statements the people followed make in a document. The model
//! must copy the words that support each one; a claim whose quote is not in the text is
//! dropped, so an invented claim never reaches the graph.

use serde::Deserialize;

use crate::collect::{contains_phrase, fold};

pub const SYSTEM: &str = "You read excerpts of Brazilian political documents (speeches, interviews, news, video subtitles, government plans) and extract claims made by specific people. \
Reply with ONLY a JSON object, no prose: {\"claims\": [{\"person\": string, \"kind\": \"promise\" | \"statement\", \"claim\": string, \"quote\": string, \"topic\": string}]}.\n\
- person: one of the people listed, written as listed.\n\
- promise: a commitment about the future (\"vou\", \"vamos\", \"me comprometo\", \"meu governo vai\", the items of a government plan). statement: a position or assertion (\"sou contra\", \"defendo\", \"votei a favor porque\", \"não vou apoiar\").\n\
- Only claims the person makes themselves, or that the text explicitly attributes to them (\"segundo X\", \"X afirmou\", \"X prometeu\"). Never the journalist's own claims, nor other people's.\n\
- claim: one short, specific sentence IN BRAZILIAN PORTUGUESE (never English), in the third person (who commits to or defends what).\n\
- quote: the words of the excerpt that support it, copied exactly as written (at most 300 characters).\n\
- topic: 1 to 4 words in Portuguese naming the subject (e.g. \"reforma tributária\", \"creches\", \"segurança pública\").\n\
- Skip greetings, thanks, procedural remarks (\"pela ordem\", orientação de bancada sem posição) and vague slogans.\n\
- Return {\"claims\": []} when there are none.";

/// A person the claims may be about: a target of the investigation.
#[derive(Debug, Clone)]
pub struct Person {
    pub target_id: String,
    pub name: String,
    pub aliases: Vec<String>,
}

impl Person {
    fn names(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.name).chain(self.aliases.iter())
    }

    /// Whether `text` names this person ("Tabata" does not name "Tabata Amaral"; the full name or an alias does).
    pub fn is(&self, text: &str) -> bool {
        let text = fold(text);
        self.names().any(|n| {
            let n = fold(n);
            text == n || contains_phrase(&text, &n) || contains_phrase(&n, &text) && text.split_whitespace().count() >= 2
        })
    }
}

/// The user prompt: who is followed, what the document is, and the excerpt.
pub fn prompt(people: &[Person], document: &str, excerpt: &str) -> String {
    let people: Vec<String> = people
        .iter()
        .map(|p| if p.aliases.is_empty() { p.name.clone() } else { format!("{} (also: {})", p.name, p.aliases.join(", ")) })
        .collect();
    format!(
        "People followed: {}\n\nDocument: {document}\n\n<excerpt>\n{excerpt}\n</excerpt>\n\nWrite \"claim\" and \"topic\" in Brazilian Portuguese.",
        people.join("; ")
    )
}

#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    pub target_id: String,
    /// promise | statement
    pub kind: String,
    pub text: String,
    pub quote: String,
    pub topic: String,
}

/// Lowercase, no accents, no punctuation, single spaces: how quotes are compared to the text
/// (subtitles have no punctuation; the model often adds some).
pub fn normalize(text: &str) -> String {
    fold(text)
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A word as compared: lowercase, no accents, letters and digits only.
fn word(token: &str) -> String {
    fold(token).chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Where the model's `quote` is in `text`, returned as the text's own words. Small slips
/// (a word added or dropped, punctuation) are tolerated: at least 80% of the quote's words
/// must be in a stretch of the text about as long. None when the words are not there.
pub fn locate_quote(text: &str, quote: &str) -> Option<String> {
    let wanted: Vec<String> = quote.split_whitespace().map(word).filter(|w| !w.is_empty()).collect();
    if wanted.len() < 3 {
        return None;
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let words: Vec<String> = tokens.iter().map(|t| word(t)).collect();
    let width = (wanted.len() + 2).min(words.len());
    let mut best: Option<(usize, usize)> = None; // (matches, start)
    for start in 0..=words.len().saturating_sub(width) {
        let mut pool: Vec<&String> = words[start..start + width].iter().collect();
        let matches = wanted
            .iter()
            .filter(|w| match pool.iter().position(|p| p == w) {
                Some(at) => {
                    pool.remove(at);
                    true
                }
                None => false,
            })
            .count();
        if best.map_or(true, |(m, _)| matches > m) {
            best = Some((matches, start));
        }
    }
    let (matches, start) = best?;
    if (matches as f64) < 0.8 * wanted.len() as f64 {
        return None;
    }
    // Trim the window to the words that belong to the quote
    let window = start..start + width;
    let first = window.clone().find(|&i| wanted.contains(&words[i]))?;
    let last = window.rev().find(|&i| wanted.contains(&words[i]))?;
    Some(tokens[first..=last].join(" "))
}

/// Reads the model's reply and keeps the claims that name a followed person and whose quote
/// is really in `excerpt`. `default_person`: the author, for a speech or a government plan.
pub fn parse(reply: &str, people: &[Person], excerpt: &str, default_person: Option<&Person>) -> Vec<Claim> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        claims: Vec<RawClaim>,
    }
    #[derive(Deserialize)]
    struct RawClaim {
        #[serde(default)]
        person: String,
        #[serde(default)]
        kind: String,
        #[serde(default)]
        claim: String,
        #[serde(default)]
        quote: String,
        #[serde(default)]
        topic: String,
    }
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else { return Vec::new() };
    let Ok(raw) = serde_json::from_str::<Raw>(reply.get(start..=end).unwrap_or_default()) else {
        return Vec::new();
    };
    raw.claims
        .into_iter()
        .filter_map(|c| {
            let person = people.iter().find(|p| p.is(&c.person)).or(default_person)?;
            let kind = c.kind.trim().to_lowercase();
            if !matches!(kind.as_str(), "promise" | "statement") || c.claim.trim().is_empty() {
                return None;
            }
            let Some(quote) = locate_quote(excerpt, &c.quote) else {
                log::info!("Analyze: dropped a claim whose quote is not in the text: {:?}", c.quote);
                return None;
            };
            Some(Claim {
                target_id: person.target_id.clone(),
                kind,
                text: c.claim.trim().to_string(),
                quote,
                topic: c.topic.trim().to_lowercase(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people() -> Vec<Person> {
        vec![
            Person { target_id: "t1".into(), name: "Marina Exemplo".into(), aliases: vec!["Marina do Bairro".into()] },
            Person { target_id: "t2".into(), name: "João Teste".into(), aliases: vec![] },
        ]
    }

    const EXCERPT: &str = "eu vou votar pela simplificação dos impostos o pequeno comerciante não aguenta mais. \
        Segundo João Teste, \"o governo não vai aumentar a carga tributária\". O jornal avalia que a reforma é necessária.";

    #[test]
    fn keeps_claims_whose_quote_is_in_the_text() {
        let reply = r#"Here: {"claims": [
            {"person": "Marina do Bairro", "kind": "promise", "claim": "Marina Exemplo promete votar pela simplificação dos impostos.", "quote": "Eu vou votar pela simplificação dos impostos!", "topic": "Reforma tributária"},
            {"person": "João Teste", "kind": "statement", "claim": "João Teste afirma que o governo não aumentará impostos.", "quote": "o governo não vai aumentar a carga tributária", "topic": "impostos"},
            {"person": "João Teste", "kind": "statement", "claim": "Inventada.", "quote": "nunca dissemos isso aqui", "topic": "x"},
            {"person": "Fulano", "kind": "statement", "claim": "Outra pessoa.", "quote": "a reforma é necessária", "topic": "x"},
            {"person": "Marina Exemplo", "kind": "opinion", "claim": "Tipo errado.", "quote": "o pequeno comerciante não aguenta mais", "topic": "x"}
        ]}"#;
        let claims = parse(reply, &people(), EXCERPT, None);
        assert_eq!(claims.len(), 2, "{claims:?}");
        assert_eq!(claims[0].target_id, "t1");
        assert_eq!(claims[0].topic, "reforma tributária");
        assert_eq!(claims[1].target_id, "t2");
    }

    #[test]
    fn quotes_are_found_despite_small_slips_and_never_invented() {
        let text = "Senhor presidente, também vou lutar para construir creches em todos os bairros da zona norte, onde hoje faltam vagas. Obrigada.";
        assert_eq!(
            locate_quote(text, "Eu vou lutar para construir creches em todos os bairros da zona norte, onde hoje faltam vagas."),
            Some("vou lutar para construir creches em todos os bairros da zona norte, onde hoje faltam vagas.".into())
        );
        assert_eq!(locate_quote(text, "construir creches em todos os bairros"), Some("construir creches em todos os bairros".into()));
        assert_eq!(locate_quote(text, "vou acabar com todas as creches da cidade"), None);
        assert_eq!(locate_quote(text, "creches"), None);
        // Subtitles have no punctuation; the model adds some
        assert!(locate_quote("eu vou votar pela simplificação dos impostos o comerciante", "Eu vou votar pela simplificação dos impostos.").is_some());
    }

    #[test]
    fn the_author_is_the_default_person() {
        let reply = r#"{"claims": [{"person": "", "kind": "promise", "claim": "Promete creches.", "quote": "vou votar pela simplificação dos impostos", "topic": "impostos"}]}"#;
        let people = people();
        assert!(parse(reply, &people, EXCERPT, None).is_empty());
        assert_eq!(parse(reply, &people, EXCERPT, Some(&people[0]))[0].target_id, "t1");
    }

    #[test]
    fn garbage_gives_nothing() {
        assert!(parse("no json", &people(), EXCERPT, None).is_empty());
        assert!(parse("{\"claims\": \"x\"}", &people(), EXCERPT, None).is_empty());
    }

    #[test]
    fn people_are_recognized_by_full_names_and_aliases() {
        let p = &people()[0];
        assert!(p.is("MARINA EXEMPLO"));
        assert!(p.is("a deputada Marina do Bairro"));
        assert!(!p.is("Marina"));
    }
}
