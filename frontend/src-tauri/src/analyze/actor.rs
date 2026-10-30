//! Whose act a record is, next to who made the claim: the politician themselves, someone of the
//! same party, someone of a party in their coalition, or someone else. A vote by another deputy
//! is never the claimant's contradiction; at most their party's or their political group's.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// self | party | ally | other
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    pub target_id: String,
    pub name: String,
    pub party: Option<String>,
    pub scope: String,
}

/// A party's acronym as written anywhere ("PC do B", "PCdoB" → "PCDOB").
pub fn party_code(party: &str) -> String {
    crate::collect::fold(party).chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_uppercase()
}

/// The parties of a TSE coalition or federation: "REPUBLICANOS / PL / PSD", or
/// "Federação BRASIL DA ESPERANÇA - FE BRASIL(PT/PC do B/PV) / PSB".
pub fn coalition_parties(composition: &str) -> Vec<String> {
    // A federation's name before its parties ("Federação … (PT/PC do B/PV)") is not a party
    let mut parties = String::new();
    for c in composition.chars() {
        match c {
            '(' => parties.truncate(parties.rfind('/').map_or(0, |i| i + 1)),
            ')' => {}
            c => parties.push(c),
        }
    }
    parties.split('/').map(party_code).filter(|code| !code.is_empty()).collect()
}

/// A person's current party: the Câmara or Senado profile (undated) first, else the latest.
async fn party_of(pool: &SqlitePool, investigation_id: &str, target_id: &str) -> Result<Option<String>> {
    Ok(sqlx::query_as::<_, (String,)>(
        "SELECT p.label FROM relations r JOIN entities pe ON pe.id = r.from_id JOIN entities p ON p.id = r.to_id
         WHERE pe.investigation_id = ? AND pe.key = ? AND r.label = 'MEMBER_OF' AND p.key LIKE 'party:%'
         ORDER BY r.date = '' DESC, r.date DESC LIMIT 1",
    )
    .bind(investigation_id)
    .bind(format!("target:{target_id}"))
    .fetch_optional(pool)
    .await?
    .map(|(label,)| label))
}

/// The parties of the coalition (or federation) of a person's latest candidacy.
async fn coalition_of(pool: &SqlitePool, investigation_id: &str, target_id: &str) -> Result<Vec<String>> {
    let attrs: Option<(String,)> = sqlx::query_as(
        "SELECT c.attrs FROM relations r JOIN entities pe ON pe.id = r.from_id JOIN entities c ON c.id = r.to_id
         WHERE pe.investigation_id = ? AND pe.key = ? AND r.label = 'CANDIDATE_IN' ORDER BY r.date DESC LIMIT 1",
    )
    .bind(investigation_id)
    .bind(format!("target:{target_id}"))
    .fetch_optional(pool)
    .await?;
    let attrs: serde_json::Value = attrs.and_then(|(a,)| serde_json::from_str(&a).ok()).unwrap_or_default();
    Ok(["coalition", "federation"]
        .iter()
        .flat_map(|field| attrs[field].as_array().cloned().unwrap_or_default())
        .filter_map(|p| p.as_str().map(party_code))
        .collect())
}

/// Whose act the record `document_id` is, for a claim by `claimant`. None: nobody's act (news,
/// a web page, a video, a gazette passage), or a record no one followed is tied to.
pub async fn of(pool: &SqlitePool, investigation_id: &str, claimant: &str, document_id: &str) -> Result<Option<Actor>> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT t.id, t.name, d.kind FROM documents d JOIN sources s ON s.id = d.source_id JOIN targets t ON t.id = s.target_id WHERE d.id = ?",
    )
    .bind(document_id)
    .fetch_optional(pool)
    .await?;
    let Some((target_id, name, kind)) = row else { return Ok(None) };
    // Official records are acts; a gazette passage that names someone is not theirs
    if !crate::collect::official::is_record(&kind) || kind == "gazette" {
        return Ok(None);
    }
    let party = party_of(pool, investigation_id, &target_id).await?;
    let code = party.as_deref().map(party_code).filter(|c| !c.is_empty());
    let scope = if target_id == claimant {
        "self"
    } else {
        let claimant_party = party_of(pool, investigation_id, claimant).await?.map(|p| party_code(&p));
        match code {
            Some(code) if Some(&code) == claimant_party.as_ref() => "party",
            Some(code) if coalition_of(pool, investigation_id, claimant).await?.contains(&code) => "ally",
            _ => "other",
        }
    };
    Ok(Some(Actor { target_id, name, party, scope: scope.into() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{entity, relate};
    use serde_json::json;

    #[test]
    fn coalitions_and_federations_are_read_as_party_codes() {
        assert_eq!(coalition_parties("REPUBLICANOS / PL / PSD / PTB / PSC / PMN"), ["REPUBLICANOS", "PL", "PSD", "PTB", "PSC", "PMN"]);
        assert_eq!(coalition_parties("Federação BRASIL DA ESPERANÇA - FE BRASIL(PT/PC do B/PV) / PSB"), ["PT", "PCDOB", "PV", "PSB"]);
        assert!(coalition_parties("").is_empty());
        assert_eq!(party_code("PCdoB"), party_code("PC do B"));
    }

    #[tokio::test]
    async fn an_act_is_the_claimants_their_partys_their_groups_or_someone_elses() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'T', '2026')").execute(&pool).await.unwrap();
        // The governor (party A, coalition A/B), a deputy of A, one of B, one of C, and the news
        let people = [("gov", "Governador", "A"), ("same", "Deputada A", "A"), ("ally", "Deputado B", "B"), ("other", "Deputado C", "C")];
        for (id, name, party) in people {
            sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES (?, 'inv', 'person', ?, '2026')").bind(id).bind(name).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO sources (id, investigation_id, target_id, kind, url, next_run_at, created_at) VALUES (?, 'inv', ?, 'camara_deputado', ?, '2026', '2026')")
                .bind(format!("src-{id}"))
                .bind(id)
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
            let person = entity(&pool, "inv", "person", &format!("target:{id}"), name, json!({}), None).await.unwrap();
            let party_id = entity(&pool, "inv", "party", &format!("party:{party}"), party, json!({}), None).await.unwrap();
            relate(&pool, "inv", &person, &party_id, "MEMBER_OF", "", json!({}), None).await.unwrap();
        }
        let gov = entity(&pool, "inv", "person", "target:gov", "Governador", json!({}), None).await.unwrap();
        let candidacy = entity(&pool, "inv", "candidacy", "tse:cand:1", "Governador 2022", json!({"coalition": ["A", "B"], "federation": []}), None).await.unwrap();
        relate(&pool, "inv", &gov, &candidacy, "CANDIDATE_IN", "2022-10-02", json!({}), None).await.unwrap();
        let doc = |source: Option<&'static str>, kind: &'static str, url: &'static str| {
            let pool = pool.clone();
            async move {
                crate::collect::save_document(
                    &pool,
                    crate::collect::NewDocument { investigation_id: "inv", source_id: source, url, kind, title: "t", author: None, site: None, published_at: None, lang: None, text: url, segments: None, note: None },
                )
                .await
                .unwrap()
                .unwrap()
            }
        };
        let scope = |document: String| {
            let pool = pool.clone();
            async move { of(&pool, "inv", "gov", &document).await.unwrap().map(|a| a.scope) }
        };
        assert_eq!(scope(doc(Some("src-gov"), "proposal", "p").await).await.as_deref(), Some("self"));
        assert_eq!(scope(doc(Some("src-same"), "vote", "v1").await).await.as_deref(), Some("party"));
        assert_eq!(scope(doc(Some("src-ally"), "vote", "v2").await).await.as_deref(), Some("ally"));
        assert_eq!(scope(doc(Some("src-other"), "vote", "v3").await).await.as_deref(), Some("other"));
        assert_eq!(scope(doc(Some("src-other"), "article", "news").await).await, None);
        assert_eq!(scope(doc(None, "vote", "v4").await).await, None);
        let actor = of(&pool, "inv", "gov", &doc(Some("src-ally"), "bill", "b").await).await.unwrap().unwrap();
        assert_eq!((actor.name.as_str(), actor.party.as_deref()), ("Deputado B", Some("B")));
    }
}
