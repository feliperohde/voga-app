//! Emendas Pix (transferências especiais) from the Transferegov's open API (PostgREST, no key):
//! each action plan says which town or state got an amendment's money, for what, and how much of
//! it the town reports as spent. The author code is the Portal da Transparência's, so it runs as
//! part of the transparencia_emendas source; plans join the same `emenda:<code>` entity.

use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

use super::{brl, Ctx, Doc};
use crate::collect::fetch;

const API: &str = "https://api.transferegov.gestao.gov.br/transferenciasespeciais";
const PAGE: usize = 200;

#[derive(Debug, Deserialize)]
pub struct Plan {
    id_plano_acao: i64,
    codigo_plano_acao: Option<String>,
    ano_plano_acao: Option<i64>,
    situacao_plano_acao: Option<String>,
    cnpj_beneficiario_plano_acao: Option<String>,
    nome_beneficiario_plano_acao: Option<String>,
    uf_beneficiario_plano_acao: Option<String>,
    codigo_emenda_parlamentar_formatado_plano_acao: Option<String>,
    codigo_descricao_areas_politicas_publicas_plano_acao: Option<String>,
    descricao_programacao_orcamentaria_plano_acao: Option<String>,
    motivo_impedimento_plano_acao: Option<String>,
    valor_custeio_plano_acao: Option<f64>,
    valor_investimento_plano_acao: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct Report {
    id_plano_acao: i64,
    data_e_hora_relatorio_gestao_novo: Option<String>,
    tipo_relatorio_gestao_novo: Option<String>,
    valor_executado_relatorio_gestao_novo: Option<f64>,
    valor_pendente_relatorio_gestao_novo: Option<f64>,
}

/// "202141320002-Tabata Amaral" → the Portal's amendment code, 202141320002.
fn amendment_code(formatted: &str) -> Option<String> {
    let code: String = formatted.split('-').next().unwrap_or("").trim().chars().filter(char::is_ascii_digit).collect();
    (code.len() == 12).then_some(code)
}

fn text(author: &str, plan: &Plan, report: Option<&Report>) -> String {
    let opt = |v: &Option<String>| v.as_deref().unwrap_or("").trim().to_string();
    let custeio = plan.valor_custeio_plano_acao.unwrap_or(0.0);
    let investimento = plan.valor_investimento_plano_acao.unwrap_or(0.0);
    let mut text = format!(
        "Emenda Pix (transferência especial) de {author}: plano de ação {} de {}.\nEmenda: {}.\nBeneficiário: {} ({}, CNPJ {}).\nÁrea: {}.\nProgramação: {}.\nValor: {} ({} de custeio e {} de investimento).\nSituação: {}.",
        opt(&plan.codigo_plano_acao),
        plan.ano_plano_acao.map(|y| y.to_string()).unwrap_or_default(),
        opt(&plan.codigo_emenda_parlamentar_formatado_plano_acao),
        opt(&plan.nome_beneficiario_plano_acao),
        opt(&plan.uf_beneficiario_plano_acao),
        opt(&plan.cnpj_beneficiario_plano_acao),
        opt(&plan.codigo_descricao_areas_politicas_publicas_plano_acao),
        opt(&plan.descricao_programacao_orcamentaria_plano_acao),
        brl(custeio + investimento),
        brl(custeio),
        brl(investimento),
        opt(&plan.situacao_plano_acao),
    );
    let impediment = opt(&plan.motivo_impedimento_plano_acao);
    if !impediment.is_empty() {
        text.push_str(&format!("\nImpedimento: {impediment}."));
    }
    match report {
        Some(r) => text.push_str(&format!(
            "\n\nRelatório de gestão ({}, {}): executado {}, pendente {}.",
            opt(&r.tipo_relatorio_gestao_novo),
            opt(&r.data_e_hora_relatorio_gestao_novo).get(..10).unwrap_or(""),
            brl(r.valor_executado_relatorio_gestao_novo.unwrap_or(0.0)),
            brl(r.valor_pendente_relatorio_gestao_novo.unwrap_or(0.0)),
        )),
        None => text.push_str("\n\nO beneficiário ainda não publicou relatório de gestão."),
    }
    text
}

/// The author's action plans and, for each, its latest management report.
async fn plans_of(author_code: &str) -> Result<(Vec<Plan>, HashMap<i64, Report>)> {
    let mut plans: Vec<Plan> = Vec::new();
    loop {
        let page: Vec<Plan> = fetch::get_json(&format!(
            "{API}/plano_acao_especial?codigo_parlamentar_emenda_plano_acao=eq.{author_code}&order=id_plano_acao&limit={PAGE}&offset={}",
            plans.len()
        ))
        .await?;
        let done = page.len() < PAGE;
        plans.extend(page);
        if done {
            break;
        }
    }
    let mut reports: HashMap<i64, Report> = HashMap::new();
    for chunk in plans.chunks(PAGE) {
        let ids: Vec<String> = chunk.iter().map(|p| p.id_plano_acao.to_string()).collect();
        let found: Vec<Report> = fetch::get_json(&format!("{API}/relatorio_gestao_novo_especial?id_plano_acao=in.({})&order=data_e_hora_relatorio_gestao_novo", ids.join(","))).await?;
        // Ordered by time: the last one wins
        for r in found {
            reports.insert(r.id_plano_acao, r);
        }
    }
    Ok((plans, reports))
}

pub async fn run(ctx: &mut Ctx<'_>) -> Result<()> {
    let author_code = ctx.source.url.clone();
    let (plans, reports) = plans_of(&author_code).await?;
    let author = ctx.name.clone();
    let person = ctx.person.clone();
    for plan in &plans {
        let report = reports.get(&plan.id_plano_acao);
        let year = plan.ano_plano_acao.map(|y| y.to_string()).unwrap_or_default();
        let place = plan.nome_beneficiario_plano_acao.clone().unwrap_or_default();
        let total = plan.valor_custeio_plano_acao.unwrap_or(0.0) + plan.valor_investimento_plano_acao.unwrap_or(0.0);
        let url = format!("{API}/plano_acao_especial?id_plano_acao=eq.{}", plan.id_plano_acao);
        let title = format!("Emenda Pix {year} – {place}: {}", brl(total));
        let date = format!("{year}-01-01");
        let doc = ctx
            .document(Doc { url: &url, kind: "amendment", title: &title, author: Some(&author), site: Some("Transferegov"), published_at: Some(&date), text: &text(&author, plan, report), note: None })
            .await?;
        let Some(code) = plan.codigo_emenda_parlamentar_formatado_plano_acao.as_deref().and_then(amendment_code) else { continue };
        let executed = report.and_then(|r| r.valor_executado_relatorio_gestao_novo);
        let amendment = ctx
            .entity("amendment", &format!("emenda:{code}"), &format!("Emenda Pix {year} ({place})"), json!({"pix": true, "situacao_pix": plan.situacao_plano_acao, "executado": executed}), Some(&doc))
            .await?;
        ctx.relate(&person, &amendment, "AUTHORED", &date, json!({}), Some(&doc)).await?;
        let cnpj: String = plan.cnpj_beneficiario_plano_acao.as_deref().unwrap_or("").chars().filter(char::is_ascii_digit).collect();
        if cnpj.len() == 14 {
            let town = ctx.entity("organization", &format!("cnpj:{cnpj}"), &place, json!({"cnpj": cnpj}), None).await?;
            ctx.relate(&amendment, &town, "PAID_TO", &date, json!({"amount": total, "via": "pix"}), Some(&doc)).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_join_the_portals_amendment_and_say_what_was_spent() {
        assert_eq!(amendment_code("202141320002-Tabata Amaral").as_deref(), Some("202141320002"));
        assert_eq!(amendment_code("sem código"), None);
        let plan: Plan = serde_json::from_value(json!({
            "id_plano_acao": 7, "codigo_plano_acao": "0903-000007", "ano_plano_acao": 2024, "situacao_plano_acao": "CIENTE",
            "cnpj_beneficiario_plano_acao": "11222333000181", "nome_beneficiario_plano_acao": "MUNICIPIO DE CIDADE EXEMPLO",
            "uf_beneficiario_plano_acao": "SP", "codigo_emenda_parlamentar_formatado_plano_acao": "202441320003-Marina Exemplo",
            "valor_custeio_plano_acao": 100000.0, "valor_investimento_plano_acao": 50000.0
        }))
        .unwrap();
        let report: Report = serde_json::from_value(json!({
            "id_plano_acao": 7, "data_e_hora_relatorio_gestao_novo": "2025-03-01T10:00:00", "tipo_relatorio_gestao_novo": "Final",
            "valor_executado_relatorio_gestao_novo": 140000.0, "valor_pendente_relatorio_gestao_novo": 10000.0
        }))
        .unwrap();
        let with = text("Marina Exemplo", &plan, Some(&report));
        assert!(with.contains("Valor: R$ 150.000,00") && with.contains("executado R$ 140.000,00") && with.contains("2025-03-01"));
        assert!(text("Marina Exemplo", &plan, None).contains("ainda não publicou"));
    }
}
