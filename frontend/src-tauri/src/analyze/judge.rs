//! Checking a claim against the records: for each record found about its subject, the model
//! says whether it contradicts the claim, fulfills it (promises), agrees with it, or is
//! unrelated. Only contradictions and fulfillments are kept, as suggestions.

use serde::Deserialize;

pub const SYSTEM: &str = "You compare a claim made by a Brazilian politician with numbered records (nominal votes, bills, speeches, news, government plans), each with its date. For each record decide how it relates to the claim:\n\
- CONTRADICTS: the record shows the same person doing or saying the opposite (promised to support X and voted against X; said they oppose Y and authored a bill for Y; promised one thing and later said the opposite).\n\
- FULFILLS: only for promises: the record shows the same person acting to keep it (voted for it, authored a bill for it, directed funds to it).\n\
- CONSISTENT: the same position again, without an act that keeps the promise.\n\
- UNRELATED: another subject, another person's actions, or not enough to tell.\n\
Each record says whose act it is (\"Act of\"). When it is another person's act (another deputy's vote, another politician's bill), judge that act against what was said (a vote against the very thing promised CONTRADICTS it, a vote for it FULFILLS it): it will be shown as the act of that person, their party or their group, never as the claimant's. Never write that the claimant did what another person did: the rationale names who acted. Records with no \"Act of\" (news, pages, videos) are reports: they count only when they report the claimant's own act or words.\n\
Be strict: the record must be about the same bill or the same specific policy as the claim; a different bill on a related theme is UNRELATED. Read what a vote was about before using its Sim/Não: voting \"Não\" on a motion to reject a bill is supporting it. Dates matter: a record can contradict an earlier or a later claim.\n\
Reply with ONLY a JSON object: {\"judgements\": [{\"record\": number, \"same_subject\": true | false, \"relation\": \"CONTRADICTS\" | \"FULFILLS\" | \"CONSISTENT\" | \"UNRELATED\", \"confidence\": number from 0 to 1, \"rationale\": one sentence in Portuguese explaining it}]}. same_subject: true only when the record is about the same bill or specific policy the claim is about.";

/// A record shown to the model.
pub struct Record<'a> {
    /// Whose act it is ("the politician themselves", "Adilson Barroso (PL), another person"), if anyone's.
    pub actor: Option<String>,
    pub kind: &'a str,
    pub title: &'a str,
    pub date: &'a str,
    pub text: &'a str,
}

pub fn prompt(person: &str, kind: &str, claim: &str, quote: &str, date: &str, records: &[Record]) -> String {
    let kind = if kind == "promise" { "Promise" } else { "Statement" };
    format!("Politician: {person}\n{kind} ({}): {claim}\nTheir words: \"{quote}\"\n\n<records>\n{}\n</records>", date.get(..10).unwrap_or(date), numbered(records))
}

/// The records as the model reads them: numbered from 1, with kind, date, title and whose act.
pub fn numbered(records: &[Record]) -> String {
    records
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let actor = r.actor.as_deref().map(|a| format!("\nAct of: {a}")).unwrap_or_default();
            format!("[{}] {} · {} · \"{}\"{actor}\n{}", i + 1, r.kind, r.date.get(..10).unwrap_or(r.date), r.title, r.text)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Judgement {
    /// Index into the records given.
    pub record: usize,
    /// CONTRADICTS | FULFILLS
    pub relation: String,
    pub confidence: f64,
    pub rationale: String,
}

/// The contradictions (and, for promises, fulfillments) at least `min_confidence` sure.
pub fn parse(reply: &str, records: usize, is_promise: bool, min_confidence: f64) -> Vec<Judgement> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        judgements: Vec<RawJudgement>,
    }
    #[derive(Deserialize)]
    struct RawJudgement {
        record: Option<usize>,
        /// Missing counts as no: the model must say the subject is the same
        #[serde(default)]
        same_subject: bool,
        #[serde(default)]
        relation: String,
        confidence: Option<f64>,
        #[serde(default)]
        rationale: String,
    }
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else { return Vec::new() };
    let Ok(raw) = serde_json::from_str::<Raw>(reply.get(start..=end).unwrap_or_default()) else {
        return Vec::new();
    };
    let mut kept: Vec<Judgement> = Vec::new();
    for j in raw.judgements {
        let relation = j.relation.trim().to_uppercase();
        let (Some(record), Some(confidence)) = (j.record, j.confidence) else { continue };
        let wanted = relation == "CONTRADICTS" || (relation == "FULFILLS" && is_promise);
        if !wanted || !j.same_subject || record == 0 || record > records || confidence < min_confidence || j.rationale.trim().is_empty() {
            continue;
        }
        // One judgement per record
        if kept.iter().any(|k| k.record == record - 1) {
            continue;
        }
        kept.push(Judgement { record: record - 1, relation, confidence: confidence.min(1.0), rationale: j.rationale.trim().to_string() });
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_confident_contradictions_and_fulfillments_of_promises() {
        let reply = r#"```json
        {"judgements": [
          {"record": 1, "same_subject": true, "relation": "CONTRADICTS", "confidence": 0.9, "rationale": "Prometeu apoiar e votou contra."},
          {"record": 2, "same_subject": true, "relation": "FULFILLS", "confidence": 0.8, "rationale": "Apresentou o projeto prometido."},
          {"record": 3, "same_subject": true, "relation": "CONTRADICTS", "confidence": 0.4, "rationale": "Talvez."},
          {"record": 4, "same_subject": true, "relation": "UNRELATED", "confidence": 0.9, "rationale": "Outro assunto."},
          {"record": 9, "same_subject": true, "relation": "CONTRADICTS", "confidence": 0.9, "rationale": "Não existe."},
          {"record": 1, "same_subject": true, "relation": "FULFILLS", "confidence": 0.9, "rationale": "Repetido."},
          {"record": 3, "same_subject": false, "relation": "CONTRADICTS", "confidence": 0.95, "rationale": "Outro projeto do mesmo tema."},
          {"record": 4, "relation": "CONTRADICTS", "confidence": 0.95, "rationale": "Sem dizer se é o mesmo assunto."}
        ]}
        ```"#;
        let kept = parse(reply, 4, true, 0.6);
        assert_eq!(kept.len(), 2, "{kept:?}");
        assert_eq!(kept[0], Judgement { record: 0, relation: "CONTRADICTS".into(), confidence: 0.9, rationale: "Prometeu apoiar e votou contra.".into() });
        assert_eq!(kept[1].relation, "FULFILLS");
        // A statement is not "fulfilled"
        assert_eq!(parse(reply, 4, false, 0.6).len(), 1);
        assert!(parse("nope", 4, true, 0.6).is_empty());
    }

    #[test]
    fn the_prompt_numbers_records_with_dates_and_whose_act_they_are() {
        let records = [Record { actor: Some("Adilson Barroso (PL), another person".into()), kind: "vote", title: "Votação", date: "2023-07-06T21:00", text: "Votou Não." }];
        let text = prompt("Marina", "promise", "Promete X.", "vou fazer X", "2022-09-01", &records);
        assert!(text.contains("Promise (2022-09-01): Promete X."));
        assert!(text.contains("[1] vote · 2023-07-06 · \"Votação\"\nAct of: Adilson Barroso (PL), another person\nVotou Não."));
    }
}
