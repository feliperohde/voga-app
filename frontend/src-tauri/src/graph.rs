//! The investigation's graph: entities (people, parties, bills, votes, claims…) and the
//! relations between them. Official data writes confirmed facts; the AI writes suggestions
//! that a person confirms or rejects.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use sqlx::SqlitePool;

/// Creates the entity, or updates its label and attributes. Returns its id.
pub async fn entity(pool: &SqlitePool, investigation_id: &str, entity_type: &str, key: &str, label: &str, attrs: Value, document_id: Option<&str>) -> Result<String> {
    let id = format!("ent-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO entities (id, investigation_id, type, key, label, attrs, document_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(investigation_id, key) DO UPDATE SET label = excluded.label,
            attrs = json_patch(entities.attrs, excluded.attrs), document_id = COALESCE(excluded.document_id, entities.document_id)",
    )
    .bind(&id)
    .bind(investigation_id)
    .bind(entity_type)
    .bind(key)
    .bind(label)
    .bind(attrs.to_string())
    .bind(document_id)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    let (id,): (String,) = sqlx::query_as("SELECT id FROM entities WHERE investigation_id = ? AND key = ?").bind(investigation_id).bind(key).fetch_one(pool).await?;
    Ok(id)
}

#[allow(clippy::too_many_arguments)]
pub async fn relate(pool: &SqlitePool, investigation_id: &str, from: &str, to: &str, label: &str, date: &str, props: Value, document_id: Option<&str>) -> Result<()> {
    sqlx::query(
        "INSERT INTO relations (id, investigation_id, from_id, to_id, label, props, date, document_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(investigation_id, from_id, to_id, label, date) DO UPDATE SET props = excluded.props,
            document_id = COALESCE(excluded.document_id, relations.document_id)",
    )
    .bind(format!("rel-{}", uuid::Uuid::new_v4()))
    .bind(investigation_id)
    .bind(from)
    .bind(to)
    .bind(label)
    .bind(props.to_string())
    .bind(date)
    .bind(document_id)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// What the AI found about a claim: a suggested relation with its reason and confidence.
/// A relation the user already confirmed or rejected keeps that decision.
#[allow(clippy::too_many_arguments)]
pub async fn suggest(
    pool: &SqlitePool,
    investigation_id: &str,
    from: &str,
    to: &str,
    label: &str,
    date: &str,
    props: Value,
    document_id: Option<&str>,
    rationale: &str,
    confidence: f64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO relations (id, investigation_id, from_id, to_id, label, props, date, document_id, origin, status, confidence, rationale, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'llm', 'suggested', ?, ?, ?)
         ON CONFLICT(investigation_id, from_id, to_id, label, date) DO UPDATE SET
            props = excluded.props, confidence = excluded.confidence, rationale = excluded.rationale
         WHERE relations.status = 'suggested'",
    )
    .bind(format!("rel-{}", uuid::Uuid::new_v4()))
    .bind(investigation_id)
    .bind(from)
    .bind(to)
    .bind(label)
    .bind(props.to_string())
    .bind(date)
    .bind(document_id)
    .bind(confidence)
    .bind(rationale)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

// ===== Reading the graph (the Map, and the export to flowsint) =====

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: String,
    /// person | party | organization | bill | vote_session | candidacy | promise | statement | a document kind
    #[sqlx(rename = "type")]
    pub kind: String,
    pub key: String,
    pub label: String,
    /// JSON text: what official data and enrichment know about it (CNPJ, Receita, sanctions…).
    pub attrs: String,
    pub document_id: Option<String>,
    /// The investigations it is in, on a map of several (the same key in each).
    #[sqlx(skip)]
    pub investigations: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub id: String,
    #[sqlx(rename = "from_id")]
    pub source: String,
    #[sqlx(rename = "to_id")]
    pub target: String,
    pub label: String,
    /// confirmed | suggested
    pub status: String,
    pub date: String,
    /// JSON text: amount, vote, contract details…
    pub props: String,
    /// The record that shows it (the evidence).
    pub document_id: Option<String>,
    /// The vote, for VOTED.
    #[sqlx(skip)]
    pub vote: Option<String>,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
}

/// What the Map shows. By default it stays readable: people, parties, companies, candidacies,
/// claims, the bills people authored, the amendments whose money is traced, and the votes and
/// records claims are tied to.
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphOptions {
    /// Every nominal vote (a deputy has a thousand).
    #[serde(default)]
    pub all_votes: bool,
    /// The AI's findings not reviewed yet.
    #[serde(default)]
    pub suggested: bool,
}

pub async fn load(pool: &SqlitePool, investigation_id: &str, options: GraphOptions) -> Result<Graph> {
    let nodes: Vec<Node> = sqlx::query_as("SELECT id, type, key, label, attrs, document_id FROM entities WHERE investigation_id = ?")
        .bind(investigation_id)
        .fetch_all(pool)
        .await?;
    let mut links: Vec<Link> = sqlx::query_as(
        "SELECT id, from_id, to_id, label, status, date, props, document_id FROM relations
         WHERE investigation_id = ? AND status != 'rejected' AND (status = 'confirmed' OR ?)",
    )
    .bind(investigation_id)
    .bind(options.suggested)
    .fetch_all(pool)
    .await?;
    for link in &mut links {
        link.vote = serde_json::from_str::<Value>(&link.props).ok().and_then(|p| p["vote"].as_str().map(String::from));
    }

    // Nodes touched by a claim (its findings) stay even when their kind is hidden
    let claims: std::collections::HashSet<&str> =
        nodes.iter().filter(|n| matches!(n.kind.as_str(), "promise" | "statement")).map(|n| n.id.as_str()).collect();
    let near_claims: std::collections::HashSet<&str> = links
        .iter()
        .filter(|l| claims.contains(l.source.as_str()) || claims.contains(l.target.as_str()))
        .flat_map(|l| [l.source.as_str(), l.target.as_str()])
        .collect();
    let authored: std::collections::HashSet<&str> = links.iter().filter(|l| l.label == "AUTHORED").map(|l| l.target.as_str()).collect();
    // Amendments whose money is traced (who received it, which contract it paid)
    let paying: std::collections::HashSet<&str> = links.iter().filter(|l| l.label == "PAID_TO" || l.label == "FUNDS").map(|l| l.source.as_str()).collect();
    let shown: std::collections::HashSet<String> = nodes
        .iter()
        .filter(|n| match n.kind.as_str() {
            "vote_session" => options.all_votes || near_claims.contains(n.id.as_str()),
            "amendment" => options.all_votes || paying.contains(n.id.as_str()) || near_claims.contains(n.id.as_str()),
            "bill" => options.all_votes || authored.contains(n.id.as_str()) || near_claims.contains(n.id.as_str()),
            "person" | "party" | "organization" | "candidacy" | "promise" | "statement" => true,
            _ => near_claims.contains(n.id.as_str()),
        })
        .map(|n| n.id.clone())
        .collect();
    let links: Vec<Link> = links.into_iter().filter(|l| shown.contains(&l.source) && shown.contains(&l.target)).collect();
    let nodes: Vec<Node> = nodes.into_iter().filter(|n| shown.contains(&n.id)).collect();
    Ok(Graph { nodes, links })
}

/// Several investigations on one map. What has the same key in more than one (a party, a vote,
/// a bill, a company, a city hall) is one node, kept even where each map alone would hide it,
/// and every node says which investigations it is in.
pub async fn load_many(pool: &SqlitePool, ids: &[String], options: GraphOptions) -> Result<Graph> {
    let mut graphs = Vec::new();
    for id in ids {
        let everything = load(pool, id, GraphOptions { all_votes: true, ..options }).await?;
        let shown: HashSet<String> = load(pool, id, options).await?.nodes.into_iter().map(|n| n.id).collect();
        graphs.push((id, everything, shown));
    }
    let mut owners: HashMap<String, Vec<String>> = HashMap::new();
    for (id, graph, _) in &graphs {
        for node in &graph.nodes {
            owners.entry(node.key.clone()).or_default().push(id.to_string());
        }
    }
    let mut merged = Graph::default();
    // The node kept for each key, and what each investigation's own node id became
    let mut kept: HashMap<String, String> = HashMap::new();
    let mut alias: HashMap<String, String> = HashMap::new();
    for (_, graph, shown) in graphs {
        for mut node in graph.nodes {
            let investigations = &owners[&node.key];
            if !shown.contains(&node.id) && investigations.len() < 2 {
                continue;
            }
            match kept.get(&node.key) {
                Some(id) => {
                    alias.insert(node.id, id.clone());
                }
                None => {
                    kept.insert(node.key.clone(), node.id.clone());
                    alias.insert(node.id.clone(), node.id.clone());
                    node.investigations = investigations.clone();
                    merged.nodes.push(node);
                }
            }
        }
        for mut link in graph.links {
            if let (Some(source), Some(target)) = (alias.get(&link.source), alias.get(&link.target)) {
                link.source = source.clone();
                link.target = target.clone();
                merged.links.push(link);
            }
        }
    }
    Ok(merged)
}

#[tauri::command]
pub async fn investigation_graph(
    state: tauri::State<'_, crate::state::AppState>,
    investigation_ids: Vec<String>,
    options: Option<GraphOptions>,
) -> std::result::Result<Graph, String> {
    let pool = state.db_manager.pool();
    let options = options.unwrap_or_default();
    match investigation_ids.as_slice() {
        [one] => load(pool, one, options).await,
        many => load_many(pool, many, options).await,
    }
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn the_map_stays_readable_and_hides_rejected_findings() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'T', '2026')").execute(&pool).await.unwrap();
        let e = |t: &'static str, k: &'static str| {
            let pool = pool.clone();
            async move { entity(&pool, "inv", t, k, k, json!({}), None).await.unwrap() }
        };
        let person = e("person", "target:t1").await;
        let party = e("party", "party:PDE").await;
        let vote_plain = e("vote_session", "camara:vot:1").await;
        let vote_claimed = e("vote_session", "camara:vot:2").await;
        let bill = e("bill", "camara:prop:9").await;
        let claim = e("promise", "claim:x").await;
        let amendment = e("amendment", "emenda:1").await;
        let quiet_amendment = e("amendment", "emenda:2").await;
        let town = e("organization", "cnpj:1").await;
        relate(&pool, "inv", &person, &party, "MEMBER_OF", "", json!({}), None).await.unwrap();
        relate(&pool, "inv", &person, &vote_plain, "VOTED", "2023-01-01", json!({"vote": "Sim"}), None).await.unwrap();
        relate(&pool, "inv", &person, &vote_claimed, "VOTED", "2023-02-01", json!({"vote": "Não"}), None).await.unwrap();
        relate(&pool, "inv", &person, &bill, "AUTHORED", "2023-03-01", json!({}), None).await.unwrap();
        relate(&pool, "inv", &person, &claim, "MADE", "2022-09-01", json!({}), None).await.unwrap();
        relate(&pool, "inv", &amendment, &town, "PAID_TO", "2023-06", json!({}), None).await.unwrap();
        suggest(&pool, "inv", &claim, &vote_claimed, "CONTRADICTS", "2023-02-01", json!({}), None, "r", 0.9).await.unwrap();

        // Suggested findings hidden: the claimed vote has nothing tying it to a claim
        let graph = load(&pool, "inv", GraphOptions::default()).await.unwrap();
        let ids: std::collections::HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(person.as_str()) && ids.contains(party.as_str()) && ids.contains(bill.as_str()) && ids.contains(claim.as_str()));
        assert!(!ids.contains(vote_plain.as_str()) && !ids.contains(vote_claimed.as_str()));
        assert!(ids.contains(amendment.as_str()) && !ids.contains(quiet_amendment.as_str()));

        let with_suggestions = load(&pool, "inv", GraphOptions { suggested: true, all_votes: false }).await.unwrap();
        assert!(with_suggestions.nodes.iter().any(|n| n.id == vote_claimed));
        assert!(!with_suggestions.nodes.iter().any(|n| n.id == vote_plain));
        let voted = with_suggestions.links.iter().find(|l| l.label == "VOTED").unwrap();
        assert_eq!(voted.vote.as_deref(), Some("Não"));

        sqlx::query("UPDATE relations SET status = 'rejected' WHERE label = 'CONTRADICTS'").execute(&pool).await.unwrap();
        let everything = load(&pool, "inv", GraphOptions { suggested: true, all_votes: true }).await.unwrap();
        assert_eq!(everything.nodes.len(), 9);
        assert!(!everything.links.iter().any(|l| l.label == "CONTRADICTS"));
    }

    #[tokio::test]
    async fn two_investigations_share_what_has_the_same_key() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for id in ["a", "b"] {
            sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES (?, ?, '2026')").bind(id).bind(id).execute(&pool).await.unwrap();
            let person = entity(&pool, id, "person", &format!("target:{id}"), id, json!({}), None).await.unwrap();
            let party = entity(&pool, id, "party", "party:PDE", "PDE", json!({}), None).await.unwrap();
            // The same vote: each map alone hides it (no claim), together it is what they share
            let vote = entity(&pool, id, "vote_session", "camara:vot:1", "PL 1", json!({}), None).await.unwrap();
            relate(&pool, id, &person, &party, "MEMBER_OF", "", json!({}), None).await.unwrap();
            relate(&pool, id, &person, &vote, "VOTED", "2026-01-01", json!({"vote": if id == "a" { "Sim" } else { "Não" }}), None).await.unwrap();
        }
        let only = entity(&pool, "b", "vote_session", "camara:vot:2", "PL 2", json!({}), None).await.unwrap();
        let graph = load_many(&pool, &["a".to_string(), "b".to_string()], GraphOptions::default()).await.unwrap();
        let party: Vec<&Node> = graph.nodes.iter().filter(|n| n.key == "party:PDE").collect();
        assert_eq!(party.len(), 1, "one party node for both");
        assert_eq!(party[0].investigations, vec!["a", "b"]);
        let vote = graph.nodes.iter().find(|n| n.key == "camara:vot:1").expect("the shared vote is shown");
        let votes: Vec<_> = graph.links.iter().filter(|l| l.target == vote.id).filter_map(|l| l.vote.as_deref()).collect();
        assert_eq!(votes, vec!["Sim", "Não"]);
        assert!(!graph.nodes.iter().any(|n| n.id == only), "a vote of one of them stays hidden");
        assert!(graph.links.iter().all(|l| graph.nodes.iter().any(|n| n.id == l.source) && graph.nodes.iter().any(|n| n.id == l.target)));
    }
}
