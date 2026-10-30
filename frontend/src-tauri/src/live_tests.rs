//! End-to-end checks against the real internet and real models, not run by default.
//!
//! With the built-in models (the app's default): VOGA_DATA_DIR is a folder whose models/ has
//! the GGUF files (bge-m3-Q8_0.gguf and a chat model, e.g. Qwen3.5-4B-Q4_K_M.gguf), and
//! VOGA_LLAMA_HELPER the sidecar (`cargo build -p llama-helper --release --features metal`):
//!
//!   VOGA_DATA_DIR=/tmp/voga-models VOGA_LLAMA_HELPER=$PWD/../../target/release/llama-helper \
//!     cargo test --lib live_ -- --ignored --nocapture
//!
//! With Ollama instead: VOGA_OLLAMA=http://192.168.3.16:11434 VOGA_CHAT_MODEL=qwen2.5:14b
//!
//! They collect from public sources (the Câmara's news feed, YouTube), index with bge-m3
//! and answer a question with the chat model.

use crate::collect::{self, QueueItem, SourceRow};
use crate::llm::ConfiguredChatModel;
use crate::rag::{answer, indexer, retriever, store::SearchFilters};
use sqlx::SqlitePool;

const FEED: &str = "https://www.camara.leg.br/noticias/rss/dinamico/POLITICA";

async fn pool() -> SqlitePool {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,app_lib=info")).is_test(true).try_init();
    let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    configure_ai(&pool).await;
    sqlx::query("INSERT INTO investigations (id, name, description, created_at) VALUES ('inv', 'Câmara', 'Notícias da Câmara dos Deputados', '2026-10-15')")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

/// The built-in models, or Ollama when VOGA_OLLAMA names a server (VOGA_CHAT_MODEL: its model,
/// or a built-in one; empty picks the best built-in model downloaded).
async fn configure_ai(pool: &SqlitePool) {
    let model = std::env::var("VOGA_CHAT_MODEL").unwrap_or_default();
    if let Ok(ollama) = std::env::var("VOGA_OLLAMA") {
        sqlx::query("UPDATE ai_settings SET provider = 'ollama', model = ?, ollama_endpoint = ?, embedding_provider = 'ollama', embedding_model = 'bge-m3' WHERE id = 1")
            .bind(if model.is_empty() { "qwen2.5:14b".to_string() } else { model })
            .bind(ollama)
            .execute(pool)
            .await
            .unwrap();
    } else {
        let dir = std::env::var("VOGA_DATA_DIR").expect("VOGA_DATA_DIR (a folder with models/) or VOGA_OLLAMA");
        crate::builtin::set_data_dir(dir.into());
        sqlx::query("UPDATE ai_settings SET provider = 'builtin-ai', model = ?, embedding_provider = 'builtin', embedding_model = 'bge-m3-q8' WHERE id = 1")
            .bind(model)
            .execute(pool)
            .await
            .unwrap();
    }
}

fn source(kind: &str, url: &str) -> SourceRow {
    SourceRow {
        id: format!("src-{kind}"),
        investigation_id: "inv".into(),
        target_id: None,
        kind: kind.into(),
        url: url.into(),
        title: None,
        etag: None,
        last_modified: None,
        follow_links: false,
    }
}

async fn insert_source(pool: &SqlitePool, source: &SourceRow) {
    sqlx::query("INSERT INTO sources (id, investigation_id, kind, url, next_run_at, created_at) VALUES (?, 'inv', ?, ?, '2026-01-01T00:00:00Z', '2026-10-15')")
        .bind(&source.id)
        .bind(&source.kind)
        .bind(&source.url)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore]
async fn live_feed_to_answer() {
    let pool = pool().await;
    let dir = std::env::temp_dir().join("voga-live");
    let embedder = indexer::embedder(&pool).await.unwrap();

    let feed = source("rss", FEED);
    insert_source(&pool, &feed).await;
    let queued = collect::run_source(&pool, &dir, &embedder, &feed).await.unwrap();
    println!("feed queued {queued} articles");
    assert!(queued > 0);

    let items: Vec<QueueItem> = sqlx::query_as(
        "SELECT id, investigation_id, source_id, url, kind, title, published_at, attempts FROM crawl_queue ORDER BY id LIMIT 3",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for item in &items {
        let saved = collect::process_item(&pool, &dir, &embedder, item).await;
        println!("{} -> {saved:?}", item.url);
        assert!(saved.unwrap());
    }
    let docs: Vec<(String, String, Option<String>, i64, Option<String>)> =
        sqlx::query_as("SELECT title, site, published_at, length(text), index_error FROM documents").fetch_all(&pool).await.unwrap();
    for doc in &docs {
        println!("doc: {doc:?}");
        assert!(doc.3 > 200, "too little text extracted: {doc:?}");
        assert_eq!(doc.4, None, "not embedded: {doc:?}");
    }

    let found = retriever::hybrid_search(&pool, &embedder, &["inv".to_string()], "projeto aprovado pela Câmara", &SearchFilters::default(), 5).await.unwrap();
    assert!(found.vector_error.is_none(), "{:?}", found.vector_error);
    assert!(found.results.iter().any(|r| r.sources.contains(&"vector")));

    let llm = ConfiguredChatModel::from_settings(&pool).await.unwrap();
    let today = chrono::Utc::now().date_naive();
    let reply = answer::ask(&pool, &embedder, &llm, &["inv".to_string()], "Quais projetos ou temas aparecem nessas notícias?", &[], today).await.unwrap();
    println!("\nANSWER:\n{}\n\ncitations: {:?}", reply.answer, reply.citations.iter().map(|c| (&c.index, &c.title)).collect::<Vec<_>>());
    assert!(reply.found);
    assert!(!reply.citations.is_empty());
}

#[tokio::test]
#[ignore]
async fn live_youtube_video() {
    let pool = pool().await;
    let dir = std::env::temp_dir().join("voga-live");
    let embedder = indexer::embedder(&pool).await.unwrap();
    let video = source("youtube_video", "https://www.youtube.com/watch?v=fFbQi8zCXyY");
    insert_source(&pool, &video).await;
    let result = collect::run_source(&pool, &dir, &embedder, &video).await;
    println!("video -> {result:?}");
    let (title, segments, note): (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT title, segments, note FROM documents").fetch_one(&pool).await.unwrap();
    let lines = segments.as_deref().map(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).unwrap().len()).unwrap_or(0);
    println!("title: {title}\nsubtitle lines: {lines}\nnote: {note:?}");
    assert!(lines > 0 || note.is_some());
}

/// Indexes by keywords only: the official connectors are checked, not Ollama.
struct NoVectors;

#[async_trait::async_trait]
impl crate::rag::embeddings::EmbeddingProvider for NoVectors {
    fn model_id(&self) -> &str {
        "none"
    }
    async fn embed(&self, _: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        anyhow::bail!("no vectors in this test")
    }
}

async fn official_source(pool: &SqlitePool, kind: &str, id: &str, target: &str) -> SourceRow {
    sqlx::query("INSERT OR IGNORE INTO targets (id, investigation_id, kind, name, created_at) VALUES ('target', 'inv', 'person', ?, '2026-10-15')")
        .bind(target)
        .execute(pool)
        .await
        .unwrap();
    let source = SourceRow { target_id: Some("target".into()), ..source(kind, id) };
    insert_source(pool, &source).await;
    sqlx::query("UPDATE sources SET target_id = 'target' WHERE id = ?").bind(&source.id).execute(pool).await.unwrap();
    source
}

async fn report(pool: &SqlitePool) {
    let kinds: Vec<(String, i64, i64)> = sqlx::query_as("SELECT kind, COUNT(*), CAST(AVG(length(text)) AS INTEGER) FROM documents GROUP BY kind").fetch_all(pool).await.unwrap();
    let relations: Vec<(String, i64)> = sqlx::query_as("SELECT label, COUNT(*) FROM relations GROUP BY label").fetch_all(pool).await.unwrap();
    let entities: Vec<(String, i64)> = sqlx::query_as("SELECT type, COUNT(*) FROM entities GROUP BY type").fetch_all(pool).await.unwrap();
    println!("documents (kind, count, average length): {kinds:?}\nrelations: {relations:?}\nentities: {entities:?}");
    let samples: Vec<(String, String)> =
        sqlx::query_as("SELECT kind, substr(title, 1, 140) FROM documents GROUP BY kind").fetch_all(pool).await.unwrap();
    for sample in samples {
        println!("  e.g. {sample:?}");
    }
}

#[tokio::test]
#[ignore]
async fn live_camara() {
    let pool = pool().await;
    let dir = std::env::temp_dir().join("voga-live");
    let matches = crate::collect::official::camara::find("Tabata Amaral").await.unwrap();
    println!("matches: {matches:?}");
    let source = official_source(&pool, "camara_deputado", &matches[0].id, "Tabata Amaral").await;
    let saved = crate::collect::official::run(&pool, &dir, &NoVectors, &source).await.unwrap();
    println!("saved {saved}");
    report(&pool).await;
    let (cursor,): (String,) = sqlx::query_as("SELECT cursor FROM sources").fetch_one(&pool).await.unwrap();
    println!("cursor: {cursor}");
    assert!(saved > 10);

    // A second run only looks at what is new
    let again = crate::collect::official::run(&pool, &dir, &NoVectors, &source).await.unwrap();
    println!("second run saved {again}");

    let llm = ConfiguredChatModel::from_settings(&pool).await.unwrap();
    let today = chrono::Utc::now().date_naive();
    let reply = answer::ask(&pool, &NoVectors, &llm, &["inv".to_string()], "Como a deputada votou nas votações sobre o PL 4015/2023 e o que esse projeto propõe?", &[], today)
        .await
        .unwrap();
    println!("\nPLAN: {:?}\nANSWER:\n{}\ncitations: {:?}", reply.plan, reply.answer, reply.citations.iter().map(|c| (&c.index, &c.kind, &c.title)).collect::<Vec<_>>());
    assert!(reply.found);
}

#[tokio::test]
#[ignore]
async fn live_senado() {
    let pool = pool().await;
    let dir = std::env::temp_dir().join("voga-live");
    let matches = crate::collect::official::senado::find("Alan Rick").await.unwrap();
    println!("matches: {matches:?}");
    let source = official_source(&pool, "senado_senador", &matches[0].id, "Alan Rick").await;
    let saved = crate::collect::official::run(&pool, &dir, &NoVectors, &source).await.unwrap();
    println!("saved {saved}");
    report(&pool).await;
    assert!(saved > 10);
}

#[tokio::test]
#[ignore]
async fn live_tse() {
    let pool = pool().await;
    let dir = std::env::temp_dir().join("voga-live");
    let matches = crate::collect::official::tse::find(&pool, &dir, "Tabata Amaral", 2022).await.unwrap();
    println!("matches: {matches:?}");
    let deputy = official_source(&pool, "tse_candidato", &matches[0].id, "Tabata Amaral").await;
    crate::collect::official::run(&pool, &dir, &NoVectors, &deputy).await.unwrap();
    // An executive candidate files a government proposal
    let governor = crate::collect::official::tse::find(&pool, &dir, "Tarcisio Gomes de Freitas", 2022).await.unwrap();
    println!("governor matches: {governor:?}");
    let governor_source = SourceRow { id: "src-gov".into(), target_id: Some("target".into()), ..source("tse_candidato", &governor[0].id) };
    sqlx::query("INSERT INTO sources (id, investigation_id, target_id, kind, url, next_run_at, created_at) VALUES ('src-gov', 'inv', 'target', 'tse_candidato', ?, '2026-01-01T00:00:00Z', '2026-10-15')")
        .bind(&governor_source.url)
        .execute(&pool)
        .await
        .unwrap();
    crate::collect::official::run(&pool, &dir, &NoVectors, &governor_source).await.unwrap();
    report(&pool).await;
    let (proposal,): (i64,) = sqlx::query_as("SELECT length(text) FROM documents WHERE kind = 'proposal'").fetch_one(&pool).await.unwrap();
    println!("proposal text: {proposal} chars");
    let candidacy: (String,) = sqlx::query_as("SELECT text FROM documents WHERE kind = 'candidacy' LIMIT 1").fetch_one(&pool).await.unwrap();
    println!("{}", candidacy.0.chars().take(900).collect::<String>());
    assert!(!candidacy.0.contains("38848319840"), "CPF must not be stored");
}

#[tokio::test]
#[ignore]
async fn live_planner() {
    use crate::llm::ChatModel;
    let pool = pool().await;
    let llm = ConfiguredChatModel::from_settings(&pool).await.unwrap();
    let reply = llm
        .complete(answer::PLANNER_SYSTEM, "Today is 2026-10-14.\n\nQuestion: Como a deputada votou nas votações sobre o PL 4015/2023?")
        .await;
    println!("RAW: {reply:?}");
    println!("PLAN: {:?}", answer::parse_plan(&reply.unwrap_or_default(), "q"));
}

/// Known cases with a fictional deputy: the AI must find the contradiction and the promise kept,
/// and nothing for the statement that no record touches.
///
///   VOGA_DATA_DIR=… VOGA_LLAMA_HELPER=… cargo test --lib live_eval_analysis -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn live_eval_analysis() {
    use crate::collect::{save_document, NewDocument};
    let pool = pool().await;
    // Two deputies followed, each with their official source: the records say whose act they are
    for (id, name) in [("t1", "Marina Exemplo"), ("t2", "Otávio Fictício")] {
        sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES (?, 'inv', 'person', ?, '2026')").bind(id).bind(name).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO sources (id, investigation_id, target_id, kind, url, next_run_at, created_at) VALUES (?, 'inv', ?, 'camara_deputado', ?, '2099-01-01T00:00:00Z', '2026')")
            .bind(format!("src-{id}"))
            .bind(id)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    let docs = [
        ("speech", "https://e.example/discurso", "Discurso de Marina Exemplo – 01/09/2022", "2022-09-01",
         "A SRA. MARINA EXEMPLO - Senhor presidente, quero assumir aqui um compromisso com as famílias trabalhadoras. Eu vou votar pela isenção do imposto sobre a cesta básica, porque comida não pode pagar imposto. Também vou lutar para construir creches em todos os bairros da zona norte, onde hoje faltam vagas. E digo mais: a segurança pública precisa de mais inteligência e menos improviso. Muito obrigada."),
        ("vote", "https://e.example/votacao-1", "Votação 06/07/2023 – PLP 99/2023: Marina Exemplo votou Não", "2023-07-06",
         "Em 06/07/2023, na Câmara dos Deputados (PLEN), Marina Exemplo votou \"Não\" na votação: Aprovada a emenda que isenta de impostos os produtos da cesta básica.\nProposição: PLP 99/2023 – Isenta de tributos os alimentos da cesta básica nacional.\nResultado: aprovada (310 votos sim, 120 não)."),
        ("bill", "https://e.example/pl-1234", "PL 1234/2023 – Cria o programa Creche no Bairro", "2023-03-10",
         "PL 1234/2023, apresentado em 10/03/2023 com autoria de Marina Exemplo.\n\nEmenta: Cria o programa Creche no Bairro, com recursos federais para construir creches em bairros sem vagas suficientes."),
        ("vote", "https://e.example/votacao-2", "Votação 12/04/2023 – PL 777/2023: Marina Exemplo votou Sim", "2023-04-12",
         "Em 12/04/2023, na Câmara dos Deputados (PLEN), Marina Exemplo votou \"Sim\" na votação: Aprovado o projeto que regulamenta o transporte por aplicativo.\nProposição: PL 777/2023 – Regulamenta o transporte remunerado por aplicativos.\nResultado: aprovada (400 votos sim, 30 não)."),
        // Another deputy's vote against the very thing Marina promised: never her contradiction
        ("vote", "https://e.example/votacao-3", "Votação 06/07/2023 – PLP 99/2023: Otávio Fictício votou Não", "2023-07-06",
         "Em 06/07/2023, na Câmara dos Deputados (PLEN), Otávio Fictício votou \"Não\" na votação: Aprovada a emenda que isenta de impostos os produtos da cesta básica.\nProposição: PLP 99/2023 – Isenta de tributos os alimentos da cesta básica nacional.\nResultado: aprovada (310 votos sim, 120 não)."),
        ("article", "https://e.example/noticia", "Prefeitura anuncia obras", "2023-05-02",
         "A prefeitura anunciou nesta terça obras de recapeamento em 40 ruas. O secretário de obras afirmou que o cronograma será cumprido até dezembro."),
    ];
    let embedder = crate::rag::indexer::embedder(&pool).await.unwrap();
    for (kind, url, title, date, text) in docs {
        let source = match (kind, url.ends_with("votacao-3")) {
            ("vote", true) => Some("src-t2"),
            ("vote" | "bill" | "speech", false) => Some("src-t1"),
            _ => None,
        };
        let id = save_document(&pool, NewDocument {
            investigation_id: "inv", source_id: source, url, kind, title, author: Some("Marina Exemplo"), site: Some("Exemplo"),
            published_at: Some(date), lang: Some("pt"), text, segments: None, note: None,
        })
        .await
        .unwrap()
        .unwrap();
        crate::rag::indexer::index_document(&pool, &embedder, &id).await.unwrap();
    }
    let llm = ConfiguredChatModel::from_settings(&pool).await.unwrap().deterministic();
    let started = std::time::Instant::now();
    while crate::analyze::pending(&pool).await.unwrap() > 0 {
        crate::analyze::run_batch(&pool, &embedder, &llm, 5, |t| println!("analyzing {t}")).await.unwrap();
    }
    println!("analysis took {:?}", started.elapsed());

    let claims: Vec<(String, String, String)> = sqlx::query_as("SELECT id, type, label FROM entities WHERE type IN ('promise', 'statement')").fetch_all(&pool).await.unwrap();
    let findings: Vec<(String, String, f64, String, String, String)> = sqlx::query_as(
        "SELECT r.from_id, r.label, r.confidence, r.rationale, d.url, COALESCE(json_extract(r.props, '$.scope'), '') FROM relations r JOIN documents d ON d.id = r.document_id WHERE r.label IN ('CONTRADICTS', 'FULFILLS')",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (id, kind, text) in &claims {
        println!("\n[{kind}] {text}");
        for (_, label, confidence, rationale, url, scope) in findings.iter().filter(|f| &f.0 == id) {
            println!("   {label} {confidence:.2} {url} (scope {scope})\n      {rationale}");
        }
    }
    let has = |label: &str, url: &str| findings.iter().any(|f| f.1 == label && f.4 == url);
    let wrong: Vec<_> = findings
        .iter()
        .filter(|f| !(f.1 == "CONTRADICTS" && f.4.ends_with("votacao-1")) && !(f.1 == "FULFILLS" && f.4.ends_with("pl-1234")) && !(f.4.ends_with("votacao-3") && f.5 == "other"))
        .collect();
    println!("\ncontradiction found: {}\nfulfillment found: {}\nunexpected findings: {}", has("CONTRADICTS", "https://e.example/votacao-1"), has("FULFILLS", "https://e.example/pl-1234"), wrong.len());
    assert!(claims.len() >= 3, "expected the two promises and the statement");
    assert!(has("CONTRADICTS", "https://e.example/votacao-1"));
    assert!(has("FULFILLS", "https://e.example/pl-1234"));
    assert!(wrong.is_empty(), "unexpected findings: {wrong:?}");
    // Marina's own records are hers; Otávio's vote, if it shows up at all, is someone else's act
    assert!(findings.iter().filter(|f| !f.4.ends_with("votacao-3")).all(|f| f.5 == "self"));
}

/// Quality check for scandals.rs's prompt: news reporting an investigation of someone followed
/// becomes a suggestion; news where they accuse someone else, or praise prosecutors, does not.
///
///   VOGA_DATA_DIR=… VOGA_LLAMA_HELPER=… cargo test --lib live_eval_scandals -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn live_eval_scandals() {
    let pool = pool().await;
    sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t1', 'inv', 'person', 'Helena Prado', '2026')").execute(&pool).await.unwrap();
    let people = crate::analyze::people(&pool, "inv").await.unwrap();
    let llm = ConfiguredChatModel::from_settings(&pool).await.unwrap().deterministic();
    let news = [
        ("A Polícia Federal deflagrou nesta quarta a Operação Ponte Quebrada, que apura fraude na compra de merenda escolar em Cidade Exemplo. Segundo o Ministério Público, a prefeita Helena Prado é investigada por ter direcionado a licitação a uma empresa de um aliado. Agentes cumpriram mandados de busca e apreensão na prefeitura.", 1),
        ("A prefeita Helena Prado denunciou ao Ministério Público o ex-secretário Carlos Fictício, que teria desviado recursos da saúde. Carlos Fictício nega as acusações e diz que vai provar sua inocência.", 0),
        ("Em visita à sede do Ministério Público, a prefeita Helena Prado elogiou o trabalho dos promotores no combate à corrupção e anunciou um convênio para treinar servidores em transparência.", 0),
    ];
    for (i, (text, expected)) in news.iter().enumerate() {
        let id = format!("d{i}");
        sqlx::query("INSERT INTO documents (id, investigation_id, url, kind, title, text, fetched_at, content_hash) VALUES (?, 'inv', ?, 'article', 'Notícia', ?, '2026', ?)")
            .bind(&id)
            .bind(format!("https://e.example/{i}"))
            .bind(text)
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(crate::scandals::worth_asking(&people, text), "news {i} must be asked about");
        let found = crate::scandals::suggest(&pool, &llm, "inv", &id, &people, "\"Notícia\" (article, Jornal Exemplo, 2026-03-01)", text).await.unwrap();
        println!("news {i}: {found} report(s), expected {expected}");
        assert_eq!(found, *expected, "news {i}");
    }
    let suggested: Vec<(String, String)> = sqlx::query_as("SELECT s.label, r.status FROM relations r JOIN entities s ON s.id = r.to_id WHERE r.label = 'INVOLVED_IN'").fetch_all(&pool).await.unwrap();
    println!("{suggested:?}");
    assert!(suggested.iter().all(|(_, status)| status == "suggested"), "the AI never confirms");
}

/// compare.rs's write-up of two subjects: three sections, citing the passages.
///
///   VOGA_DATA_DIR=… VOGA_LLAMA_HELPER=… cargo test --lib live_compare_summary -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn live_compare_summary() {
    use crate::collect::{save_document, NewDocument};
    let pool = pool().await;
    sqlx::query("UPDATE investigations SET name = 'Inteligência artificial', kind = 'interest' WHERE id = 'inv'").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO investigations (id, name, kind, created_at) VALUES ('ind', 'Indústria', 'interest', '2026')").execute(&pool).await.unwrap();
    let docs = [
        ("inv", "https://e.example/modelo", "Modelo de IA em português roda em notebook", "Pesquisadores mostraram um modelo de linguagem treinado em português que responde perguntas sem internet, num notebook de 16 GB."),
        ("inv", "https://e.example/regras", "Senado debate regras para IA no serviço público", "O projeto exige que órgãos públicos digam quando uma decisão foi tomada com ajuda de inteligência artificial."),
        ("inv", "https://e.example/fabricas-ia", "Fábricas usam IA para prever falhas", "Indústrias da região passaram a usar inteligência artificial para ler sensores das máquinas e avisar antes de uma peça quebrar, reduzindo paradas."),
        ("ind", "https://e.example/aco", "Siderúrgica amplia produção de aço", "A siderúrgica anunciou um novo forno e 300 vagas; a produção de aço deve crescer 15% no ano."),
        ("ind", "https://e.example/energia", "Conta de energia pesa na indústria", "Empresários dizem que o custo da energia elétrica é o maior obstáculo para a indústria crescer."),
        ("ind", "https://e.example/manutencao", "Manutenção preditiva chega às fábricas", "Fábricas adotam sistemas de inteligência artificial que preveem falhas nas máquinas a partir de sensores."),
    ];
    let embedder = crate::rag::indexer::embedder(&pool).await.unwrap();
    for (investigation_id, url, title, text) in docs {
        let id = save_document(&pool, NewDocument {
            investigation_id, source_id: None, url, kind: "article", title, author: None, site: Some("Jornal Exemplo"),
            published_at: Some("2026-10-20"), lang: Some("pt"), text, segments: None, note: None,
        })
        .await
        .unwrap()
        .unwrap();
        crate::rag::indexer::index_document(&pool, &embedder, &id).await.unwrap();
    }
    let llm = ConfiguredChatModel::from_settings(&pool).await.unwrap().deterministic();
    let ids = ["inv".to_string(), "ind".to_string()];
    let comparison = crate::compare::compare(&pool, &embedder, &ids).await.unwrap();
    println!("bridges: {:?}", comparison.bridges.iter().map(|p| &p.title).collect::<Vec<_>>());
    println!("divergent: {:?}", comparison.divergent.iter().map(|p| &p.title).collect::<Vec<_>>());
    let summary = crate::compare::summarize(&pool, &embedder, &llm, &ids).await.unwrap();
    println!("{}\n\ncited: {:?}", summary.text, summary.citations.iter().map(|c| &c.title).collect::<Vec<_>>());
    for heading in ["Onde se conectam", "Onde divergem", "O que ainda falta saber"] {
        assert!(summary.text.contains(heading), "missing {heading}");
    }
    assert!(!summary.citations.is_empty(), "it cites the passages");
    assert!(comparison.bridges.iter().any(|p| p.title.contains("falhas") || p.title.contains("preditiva")), "AI in the factories connects them");
}

/// Real data: a deputy's speeches checked against her real votes and bills (with bge-m3).
/// Prints what the AI suggests, for a person to judge.
#[tokio::test]
#[ignore]
async fn live_analysis_real() {
    // Kept on disk: later runs skip the collection and only analyze again
    let dir = std::env::temp_dir().join("voga-live");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("analysis_real.sqlite");
    let reuse = path.exists();
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path).create_if_missing(true).foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,app_lib=info")).is_test(true).try_init();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    configure_ai(&pool).await;
    let embedder = crate::rag::indexer::embedder(&pool).await.unwrap();
    if reuse {
        sqlx::query("DELETE FROM entities WHERE type IN ('promise', 'statement') OR key LIKE 'doc:%'").execute(&pool).await.unwrap();
        sqlx::query("UPDATE documents SET analyzed_at = NULL, analysis_error = NULL").execute(&pool).await.unwrap();
        println!("reusing {}", path.display());
    } else {
        sqlx::query("INSERT INTO investigations (id, name, description, created_at) VALUES ('inv', 'Câmara', '', '2026-10-15')").execute(&pool).await.unwrap();
        let source = official_source(&pool, "camara_deputado", "204534", "Tabata Amaral").await;
        let saved = crate::collect::official::run(&pool, &dir, &embedder, &source).await.unwrap();
        println!("collected {saved}");
    }
    let llm = ConfiguredChatModel::from_settings(&pool).await.unwrap().deterministic();
    let limit: i64 = std::env::var("VOGA_ANALYZE").ok().and_then(|n| n.parse().ok()).unwrap_or(8);
    let started = std::time::Instant::now();
    crate::analyze::run_batch(&pool, &embedder, &llm, limit, |t| println!("analyzing {t}")).await.unwrap();
    println!("analysis of {limit} documents took {:?}", started.elapsed());
    let claims: Vec<(String, String, String, String)> =
        sqlx::query_as("SELECT id, type, label, json_extract(attrs, '$.quote') FROM entities WHERE type IN ('promise', 'statement')").fetch_all(&pool).await.unwrap();
    let findings: Vec<(String, String, f64, String, String)> = sqlx::query_as(
        "SELECT r.from_id, r.label, r.confidence, r.rationale, d.title FROM relations r JOIN documents d ON d.id = r.document_id WHERE r.label IN ('CONTRADICTS', 'FULFILLS')",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for (id, kind, text, quote) in &claims {
        println!("\n[{kind}] {text}\n   “{quote}”");
        for (_, label, confidence, rationale, title) in findings.iter().filter(|f| &f.0 == id) {
            println!("   → {label} {confidence:.2} · {title}\n     {rationale}");
        }
    }
    println!("\n{} claims, {} findings", claims.len(), findings.len());
}

/// Sends the real graph kept by `live_analysis_real` to flowsint, twice, and reads it back.
/// By default flowsint is installed on this computer through Docker (as the app does), in
/// <tmp>/voga-live; `docker compose -p voga-flowsint down -v --rmi all` removes it.
/// With VOGA_FLOWSINT (+ VOGA_FLOWSINT_EMAIL, VOGA_FLOWSINT_PASSWORD) it uses that server instead.
///
///   cargo test --lib live_flowsint -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn live_flowsint() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,app_lib=info")).is_test(true).try_init();
    let dir = std::env::temp_dir().join("voga-live");
    let path = dir.join("analysis_real.sqlite");
    assert!(path.exists(), "run live_analysis_real first");
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path).foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let (endpoint, email, password) = match std::env::var("VOGA_FLOWSINT") {
        Ok(endpoint) => {
            let email = std::env::var("VOGA_FLOWSINT_EMAIL").expect("VOGA_FLOWSINT_EMAIL");
            let password = std::env::var("VOGA_FLOWSINT_PASSWORD").expect("VOGA_FLOWSINT_PASSWORD");
            // A new server has no account yet
            let registered = reqwest::Client::new()
                .post(format!("{endpoint}/api/auth/register"))
                .json(&serde_json::json!({"email": email, "password": password}))
                .send()
                .await
                .unwrap();
            println!("register: {}", registered.status());
            (endpoint, email, password)
        }
        Err(_) => {
            let started = std::time::Instant::now();
            let report = |p: &crate::flowsint::local::Progress| println!("[{}] {}", p.stage, p.message);
            let target = crate::flowsint::local::ensure_running(&report, &pool, &dir).await;
            let target = match target {
                Ok(target) => target,
                // Not installed yet: install (pulls the images)
                Err(_) => {
                    crate::flowsint::local::install(&report, &pool, &dir).await.unwrap();
                    crate::flowsint::local::ensure_running(&report, &pool, &dir).await.unwrap()
                }
            };
            println!("local flowsint ready in {:?}: {}", started.elapsed(), target.0);
            target
        }
    };

    let first = crate::flowsint::push_to(&pool, "inv", &endpoint, &email, &password).await.unwrap();
    println!("first push: {} nodes, {} edges, {} created, errors: {:?}\n{}", first.nodes, first.edges, first.nodes_created, first.errors.iter().take(5).collect::<Vec<_>>(), first.url);
    let second = crate::flowsint::push_to(&pool, "inv", &endpoint, &email, &password).await.unwrap();
    println!("second push: {} created, errors: {}", second.nodes_created, second.errors.len());
    assert_eq!(first.url, second.url, "the same sketch is reused");

    // What flowsint holds now
    let token: serde_json::Value = reqwest::Client::new()
        .post(format!("{endpoint}/api/auth/token"))
        .form(&[("username", email.as_str()), ("password", password.as_str())])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sketch = first.url.rsplit('/').next().unwrap();
    let graph: serde_json::Value = reqwest::Client::new()
        .get(format!("{endpoint}/api/sketches/{sketch}/graph"))
        .bearer_auth(token["access_token"].as_str().unwrap())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let nodes = graph["nds"].as_array().or(graph["nodes"].as_array()).map_or(0, Vec::len);
    let edges = graph["rls"].as_array().or(graph["edges"].as_array()).or(graph["relationships"].as_array()).map_or(0, Vec::len);
    println!("flowsint sketch: {nodes} nodes, {edges} edges (keys: {:?})", graph.as_object().map(|o| o.keys().collect::<Vec<_>>()));
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    assert_eq!(nodes, first.nodes, "re-sending must not duplicate nodes");
}

/// A mayoral candidate's government proposal read out of the TSE's 1.4 GB São Paulo file without
/// downloading it (Tabata Amaral, 2024).
///
///   cargo test --lib live_proposal_remote -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn live_proposal_remote() {
    let url = "https://cdn.tse.jus.br/estatistica/sead/odsele/proposta_governo/proposta_governo_2024_SP.zip";
    let pdfs = crate::collect::official::remote_zip_entries(url, |n| crate::collect::official::is_part(n, "SP/2024SP250002163891")).await.unwrap();
    let pdf = pdfs.into_iter().next().expect("the PDF is in the file");
    let text = crate::collect::extract::pdf_text(pdf.clone()).await.unwrap();
    println!("{} bytes of PDF, {} characters of text:\n{}", pdf.len(), text.len(), &text.chars().take(400).collect::<String>());
    assert!(pdf.starts_with(b"%PDF") && text.len() > 1000);
}

/// A database without any AI set up, for checks that only collect.
async fn bare_pool() -> SqlitePool {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,app_lib=info")).is_test(true).try_init();
    let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'T', '2026')").execute(&pool).await.unwrap();
    pool
}

/// Following the money for real: a federal deputy's amendments (author 4132, who has emendas
/// Pix), who received them, and one day of the São Paulo city hall's purchases and contracts.
///
///   cargo test --lib live_money -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn live_money() {
    let pool = bare_pool().await;
    let dir = std::env::temp_dir().join("voga-live");
    let amendments = official_source(&pool, "transparencia_emendas", "4132", "Tabata Amaral").await;
    let saved = crate::collect::official::run(&pool, &dir, &NoVectors, &amendments).await.unwrap();
    println!("amendments: saved {saved}");
    let (pix,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM documents WHERE site = 'Transferegov'").fetch_one(&pool).await.unwrap();
    let (paid,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM relations WHERE label = 'PAID_TO'").fetch_one(&pool).await.unwrap();
    assert!(pix > 0 && paid > 0, "emendas Pix {pix}, PAID_TO {paid}");

    let hall = crate::collect::official::pncp::city_hall("SP", "SÃO PAULO").await.unwrap().unwrap();
    println!("{hall:?}");
    assert_eq!(hall.id, "3550308");
    let mut source = official_source(&pool, "pncp_prefeitura", &hall.id, "Ricardo Nunes").await;
    source.id = "src-pncp_prefeitura".into();
    source.title = Some(format!("{} · {}", hall.label, hall.detail));
    // Only yesterday
    let before = (chrono::Utc::now().date_naive() - chrono::Duration::days(2)).format("%Y-%m-%d").to_string();
    sqlx::query("UPDATE sources SET cursor = ? WHERE id = ?").bind(serde_json::json!({"day": before}).to_string()).bind(&source.id).execute(&pool).await.unwrap();
    let saved = crate::collect::official::run(&pool, &dir, &NoVectors, &source).await.unwrap();
    println!("city hall: saved {saved}");
    report(&pool).await;
    let (cursor,): (String,) = sqlx::query_as("SELECT cursor FROM sources WHERE id = ?").bind(&source.id).fetch_one(&pool).await.unwrap();
    println!("cursor: {}", &cursor[..cursor.len().min(300)]);
    let (procurement,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM documents WHERE kind IN ('procurement', 'contract')").fetch_one(&pool).await.unwrap();
    assert!(procurement > 0);
}

/// State deputies' amendments from four states' transparency portals: each deputy is found by
/// name, linked, and their amendments collected, with who received them.
///
///   cargo test --lib live_state_amendments -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn live_state_amendments() {
    let pool = bare_pool().await;
    let dir = std::env::temp_dir().join("voga-live");
    for (i, (uf, name)) in [("SP", "Andréa Werner"), ("MG", "Beatriz Cerqueira"), ("ES", "Raquel Lessa"), ("BA", "Hilton Coelho")].into_iter().enumerate() {
        let matches = crate::collect::official::estaduais::find(&pool, &dir, name).await;
        println!("{uf} {name}: {matches:?}");
        let found = matches.iter().find(|m| m.id.starts_with(&format!("{uf}:"))).unwrap_or_else(|| panic!("{name} in {uf}"));
        let target = format!("t{i}");
        sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES (?, 'inv', 'person', ?, '2026')").bind(&target).bind(name).execute(&pool).await.unwrap();
        let row = SourceRow { id: format!("src-{uf}"), target_id: Some(target), ..source("emendas_estaduais", &found.id) };
        insert_source(&pool, &row).await;
        let saved = crate::collect::official::run(&pool, &dir, &NoVectors, &row).await.unwrap();
        let sample: (String, String) = sqlx::query_as("SELECT title, text FROM documents WHERE source_id = ? ORDER BY published_at DESC LIMIT 1").bind(&row.id).fetch_one(&pool).await.unwrap();
        println!("  saved {saved}\n  e.g. {}\n{}\n", sample.0, sample.1.chars().take(900).collect::<String>());
        assert!(saved > 0);
    }
    report(&pool).await;
}

/// Free public data about a real company: its Receita record (minhareceita.org) and the newest
/// CEIS/CNEP/CEPIM and São Paulo sanction lists (downloaded to <tmp>/voga-live/bulk).
///
///   cargo test --lib live_enrich -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn live_enrich() {
    use serde_json::json;
    let pool = bare_pool().await;
    let person = crate::graph::entity(&pool, "inv", "person", "target:t", "Pessoa", json!({}), None).await.unwrap();
    let bank = crate::graph::entity(&pool, "inv", "organization", "cnpj:00000000000191", "Banco do Brasil", json!({"cnpj": "00000000000191"}), None).await.unwrap();
    crate::graph::relate(&pool, "inv", &person, &bank, "SPENT_WITH", "2026-01", json!({}), None).await.unwrap();
    crate::enrich::step(&pool, &std::env::temp_dir().join("voga-live")).await;
    let (attrs,): (String,) = sqlx::query_as("SELECT attrs FROM entities WHERE id = ?").bind(&bank).fetch_one(&pool).await.unwrap();
    println!("{attrs}");
    let attrs: serde_json::Value = serde_json::from_str(&attrs).unwrap();
    assert_eq!(attrs["razao_social"], "BANCO DO BRASIL SA");
    assert!(attrs["sanctions"].is_array() && attrs["sanctions_checked"].is_string());
}

/// Discovery on real pages: the Câmara's latest news is a seed page; its links about the person
/// followed are queued (the AI picks among ambiguous headlines), a few are fetched, and the AI
/// suggests searches for a promise nothing proves yet.
///
///   VOGA_DATA_DIR=… VOGA_LLAMA_HELPER=… cargo test --lib live_discovery -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn live_discovery() {
    let pool = pool().await;
    let name = std::env::var("VOGA_TARGET").unwrap_or_else(|_| "Hugo Motta".into());
    sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t1', 'inv', 'person', ?, '2026')").bind(&name).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO sources (id, investigation_id, kind, url, next_run_at, follow_links, created_at) VALUES ('src-page', 'inv', 'page', 'https://www.camara.leg.br/noticias/ultimas', '2026', 1, '2026')")
        .execute(&pool)
        .await
        .unwrap();
    let embedder = indexer::embedder(&pool).await.unwrap();
    let seed = SourceRow { id: "src-page".into(), kind: "page".into(), url: "https://www.camara.leg.br/noticias/ultimas".into(), follow_links: true, ..source("page", "") };
    let started = std::time::Instant::now();
    let found = collect::run_source(&pool, std::path::Path::new("/tmp"), &embedder, &seed).await.unwrap();
    println!("seed page: {found} new (page + links queued) in {:?}", started.elapsed());
    let queued: Vec<QueueItem> = sqlx::query_as(
        "SELECT id, investigation_id, source_id, url, kind, title, published_at, attempts, via, depth FROM crawl_queue WHERE via = 'link' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    for item in &queued {
        println!("  queued (depth {}): {} · {}", item.depth, item.title.as_deref().unwrap_or(""), item.url);
    }
    let usage = crate::discover::usage(&pool, "inv").await.unwrap();
    println!("spent today: {} pages, {} AI requests", usage.pages, usage.ai_calls);
    for item in queued.iter().take(4) {
        let result = collect::process_item(&pool, std::path::Path::new("/tmp"), &embedder, item).await;
        println!("  fetched {}: {:?}", item.url, result.map_err(|e| e.to_string()));
    }

    sqlx::query("INSERT INTO entities (id, investigation_id, type, key, label, created_at) VALUES ('c1', 'inv', 'promise', 'claim:1', ?, '2026')")
        .bind(format!("{name} prometeu votar a isenção do imposto de renda para quem ganha até 5 mil reais"))
        .execute(&pool)
        .await
        .unwrap();
    let llm = ConfiguredChatModel::from_settings(&pool).await.unwrap().deterministic();
    let added = crate::discover::suggest::suggest(&pool, &llm, "inv", false).await.unwrap();
    let suggested: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT kind, url, note FROM sources WHERE status = 'suggested'").fetch_all(&pool).await.unwrap();
    println!("\nthe AI suggested {added}:");
    for (kind, query, note) in &suggested {
        println!("  {kind}: {query}\n     {}", note.as_deref().unwrap_or(""));
    }
    assert!(added > 0);
}

/// Phase 6 sources against the real sites: municipal gazettes (Querido Diário),
/// parliamentary amendments (Portal da Transparência), the ALESP and a Bluesky profile.
///
///   VOGA_DATA_DIR=… VOGA_LLAMA_HELPER=… cargo test --lib live_more_sources -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn live_more_sources() {
    let pool = pool().await;
    let dir = std::env::temp_dir().join("voga-live");
    std::fs::create_dir_all(&dir).unwrap();
    sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('target', 'inv', 'person', 'Tabata Amaral', '2026')").execute(&pool).await.unwrap();
    let embedder = indexer::embedder(&pool).await.unwrap();
    let docs = |kind: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (String, String)>("SELECT title, substr(text, 1, 220) FROM documents WHERE kind = ? ORDER BY published_at DESC LIMIT 3")
                .bind(kind)
                .fetch_all(&pool)
                .await
                .unwrap()
        }
    };

    let gazettes = SourceRow { id: "src-qd".into(), target_id: Some("target".into()), ..source("gazette_search", "\"Tabata Amaral\"") };
    insert_source(&pool, &gazettes).await;
    println!("Querido Diário: {:?}", collect::run_source(&pool, &dir, &embedder, &gazettes).await.map_err(|e| e.to_string()));
    for (title, text) in docs("gazette").await {
        println!("  {title}\n    {text}");
    }

    let matches = collect::official::find_matches(&pool, &dir, "Tabata Amaral", &[]).await.unwrap();
    println!("official matches: {matches:?}");
    let amendments = matches.iter().find(|m| m.kind == "transparencia_emendas").expect("her amendments");
    let source_row = official_source(&pool, "transparencia_emendas", &amendments.id, "Tabata Amaral").await;
    println!("amendments: {} new", collect::official::run(&pool, &dir, &embedder, &source_row).await.unwrap());
    for (title, text) in docs("amendment").await {
        println!("  {title}\n    {text}");
    }

    sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t-alesp', 'inv', 'person', 'Danilo Balas', '2026')").execute(&pool).await.unwrap();
    let alesp = collect::official::alesp::find(&pool, &dir, "Danilo Balas").await.unwrap();
    println!("ALESP matches: {alesp:?}");
    let alesp_source = SourceRow { id: "src-alesp".into(), target_id: Some("t-alesp".into()), ..source("alesp_deputado", &alesp[0].id) };
    insert_source(&pool, &alesp_source).await;
    println!("ALESP: {} new", collect::official::run(&pool, &dir, &embedder, &alesp_source).await.unwrap());
    let (bills, expenses): (i64, i64) = sqlx::query_as("SELECT SUM(kind = 'bill'), SUM(kind = 'expense') FROM documents WHERE source_id = 'src-alesp'").fetch_one(&pool).await.unwrap();
    println!("  {bills} bills, {expenses} months of expenses");

    let social = source("social", &collect::feeds::social_feed_url("https://bsky.app/profile/bsky.app").unwrap());
    let social = SourceRow { id: "src-social".into(), ..social };
    insert_source(&pool, &social).await;
    println!("Bluesky: {} posts", collect::run_source(&pool, &dir, &embedder, &social).await.unwrap());
    for (title, _) in docs("post").await {
        println!("  {title}");
    }
}

// ===== Live sessions (Ao vivo) =====
//
// macOS: speech is made with `say` (Brazilian voices) and `afconvert`. The transcription model:
// VOGA_WHISPER_MODEL (a ggml file), else <VOGA_DATA_DIR>/models/ggml-large-v3-turbo-q5_0.bin.
//
//   VOGA_WHISPER_MODEL=~/Library/Application\ Support/com.voga.app/models/ggml-large-v3-turbo-q5_0.bin \
//     cargo test --lib live_transcribe -- --ignored --nocapture

/// 16 kHz mono speech said by a macOS voice.
fn say(voice: &str, text: &str) -> Vec<f32> {
    let dir = std::env::temp_dir().join(format!("voga-say-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let (aiff, wav) = (dir.join("speech.aiff"), dir.join("speech.wav"));
    let said = std::process::Command::new("say").args(["-v", voice, "-o"]).arg(&aiff).arg(text).status().expect("say (macOS)");
    assert!(said.success(), "say -v {voice}");
    let converted = std::process::Command::new("afconvert").args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"]).arg(&aiff).arg(&wav).status().unwrap();
    assert!(converted.success());
    let bytes = std::fs::read(&wav).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    // The samples are in the "data" chunk (afconvert may put others before it)
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if &bytes[at..at + 4] == b"data" {
            let data = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
            return data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32).collect();
        }
        at += 8 + size + size % 2;
    }
    panic!("no data chunk in the WAV");
}

fn whisper_model() -> std::path::PathBuf {
    match std::env::var("VOGA_WHISPER_MODEL") {
        Ok(path) => path.into(),
        Err(_) => std::path::PathBuf::from(std::env::var("VOGA_DATA_DIR").expect("VOGA_WHISPER_MODEL or VOGA_DATA_DIR"))
            .join("models")
            .join("ggml-large-v3-turbo-q5_0.bin"),
    }
}

/// Speech detection and Whisper on Portuguese speech: the key words come out.
#[test]
#[ignore]
fn live_transcribe_pt() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,app_lib=info")).is_test(true).try_init();
    let mut audio = vec![0.0; 16_000];
    audio.extend(say(
        "Luciana",
        "Boa noite. Eu votei contra a reforma da previdência em dois mil e dezenove. No meu governo, construímos cento e vinte creches na zona norte.",
    ));
    audio.extend(vec![0.0; 16_000 * 2]);
    let mut vad = crate::live::vad::Vad::new().unwrap();
    let mut stretches = Vec::new();
    for block in audio.chunks(800) {
        stretches.extend(vad.push(block).unwrap());
    }
    stretches.extend(vad.flush());
    assert!(!stretches.is_empty(), "no speech detected");
    let transcriber = crate::live::whisper::Transcriber::load(&whisper_model()).unwrap();
    let mut text = String::new();
    for stretch in &stretches {
        let started = std::time::Instant::now();
        let (said, words) = transcriber.transcribe(&stretch.samples, "Debate com Marina Exemplo.").unwrap();
        println!("{:>5.1}-{:>5.1}s ({} words, {:.1}s): {said}", stretch.start, stretch.end, words.len(), started.elapsed().as_secs_f64());
        text.push_str(&said);
        text.push(' ');
    }
    let text = crate::collect::fold(&text);
    for expected in ["reforma da previdencia", "creches", "zona norte"] {
        assert!(text.contains(expected), "{expected:?} not in {text:?}");
    }
}

/// The live fact check with a real model: a claim the votes refute, one a bill confirms and a
/// statistic no record covers. Models as in live_eval_analysis (VOGA_DATA_DIR or VOGA_OLLAMA).
#[tokio::test]
#[ignore]
async fn live_eval_factcheck() {
    use crate::collect::{save_document, NewDocument};
    use crate::live::check::{CheckView, Checker, Line};
    use std::sync::{Arc, Mutex};
    let pool = pool().await;
    for sql in [
        "INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t1', 'inv', 'person', 'Marina Exemplo', '2026')",
        "INSERT INTO sources (id, investigation_id, target_id, kind, url, next_run_at, created_at) VALUES ('src-t1', 'inv', 't1', 'camara_deputado', 't1', '2099-01-01T00:00:00Z', '2026')",
        "INSERT INTO live_sessions (id, investigation_id, target_id, title, started_at) VALUES ('s1', 'inv', 't1', 'Debate', '2026-10-22T20:00:00Z')",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    let docs = [
        ("vote", Some("src-t1"), "https://e.example/votacao-1", "Votação 06/07/2023 – PLP 99/2023: Marina Exemplo votou Não", "2023-07-06",
         "Em 06/07/2023, na Câmara dos Deputados (PLEN), Marina Exemplo votou \"Não\" na votação: Aprovada a emenda que isenta de impostos os produtos da cesta básica.\nProposição: PLP 99/2023 – Isenta de tributos os alimentos da cesta básica nacional.\nResultado: aprovada (310 votos sim, 120 não)."),
        ("bill", Some("src-t1"), "https://e.example/pl-1234", "PL 1234/2023 – Cria o programa Creche no Bairro", "2023-03-10",
         "PL 1234/2023, apresentado em 10/03/2023 com autoria de Marina Exemplo.\n\nEmenta: Cria o programa Creche no Bairro, com recursos federais para construir creches em bairros sem vagas suficientes."),
        ("vote", Some("src-t1"), "https://e.example/votacao-2", "Votação 12/04/2023 – PL 777/2023: Marina Exemplo votou Sim", "2023-04-12",
         "Em 12/04/2023, na Câmara dos Deputados (PLEN), Marina Exemplo votou \"Sim\" na votação: Aprovado o projeto que regulamenta o transporte por aplicativo.\nProposição: PL 777/2023 – Regulamenta o transporte remunerado por aplicativos.\nResultado: aprovada (400 votos sim, 30 não)."),
        ("article", None, "https://e.example/noticia", "Prefeitura anuncia obras", "2023-05-02",
         "A prefeitura anunciou nesta terça obras de recapeamento em 40 ruas. O secretário de obras afirmou que o cronograma será cumprido até dezembro."),
    ];
    let embedder = crate::rag::indexer::embedder(&pool).await.unwrap();
    for (kind, source_id, url, title, date, text) in docs {
        let id = save_document(&pool, NewDocument {
            investigation_id: "inv", source_id, url, kind, title, author: None, site: Some("Exemplo"),
            published_at: Some(date), lang: Some("pt"), text, segments: None, note: None,
        })
        .await
        .unwrap()
        .unwrap();
        crate::rag::indexer::index_document(&pool, embedder.as_ref(), &id).await.unwrap();
    }
    let llm = ConfiguredChatModel::from_settings(&pool).await.unwrap().deterministic();
    let seen = Arc::new(Mutex::new(Vec::<CheckView>::new()));
    let heard = seen.clone();
    let people = crate::analyze::people(&pool, "inv").await.unwrap();
    let mut checker = Checker::new(pool.clone(), Arc::new(llm), Arc::from(embedder), "s1".into(), people, "2026-10-22".into(),
        Box::new(move |check| heard.lock().unwrap().push(check.clone())));
    let said = [
        "Boa noite a todos, obrigada pelo convite.",
        "Eu votei a favor da isenção de impostos da cesta básica em 2023, porque comida não pode pagar imposto.",
        "E fui eu que apresentei o projeto Creche no Bairro, o PL 1234 de 2023, para construir creches onde faltam vagas.",
        "Na minha cidade o desemprego caiu trinta por cento no ano passado.",
        "E é mentira o que dizem: eu nunca votei a favor de regulamentar o transporte por aplicativo.",
    ];
    let started = std::time::Instant::now();
    for (i, text) in said.iter().enumerate() {
        let start = i as f64 * 8.0;
        checker.heard(Line { start, end: start + 7.5, text: text.to_string(), target_id: "t1".into() }).await;
    }
    checker.flush().await;
    println!("checks took {:?}", started.elapsed());

    let checks: Vec<CheckView> = crate::live::check::list(&pool, "s1").await.unwrap();
    for c in &checks {
        println!("\n[{} {}] {} ({:?})\n   “{}”\n   {}", c.verdict, c.kind, c.claim, c.confidence, c.quote, c.rationale.as_deref().unwrap_or(""));
        for e in &c.evidence {
            println!("   ← {} {}", e.kind, e.url);
        }
    }
    let find = |words: &[&str]| checks.iter().find(|c| words.iter().all(|w| crate::collect::fold(&c.claim).contains(w)));
    let about = |words: &[&str]| find(words).map(|c| c.verdict.as_str());
    assert_eq!(about(&["cesta basica"]), Some("false"), "voted Não on the exemption");
    assert_eq!(about(&["creche"]), Some("true"), "authored PL 1234/2023");
    assert_eq!(about(&["desemprego"]), Some("unverified"), "no record covers it");
    // A denial the vote refutes: she voted Sim
    assert_eq!(find(&["aplicativo"]).map(|c| (c.verdict.as_str(), c.kind.as_str())), Some(("false", "denial")));
    assert!(seen.lock().unwrap().iter().any(|c| c.verdict == "checking"), "the UI saw them being checked");
}

/// Two macOS voices taking turns are told apart, and each line gets its voice. The voice
/// model: VOGA_VOICE_MODEL, else <VOGA_DATA_DIR>/models/wespeaker_en_voxceleb_resnet34_LM.onnx.
#[test]
#[ignore]
fn live_voices() {
    use crate::live::voices;
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,app_lib=info")).is_test(true).try_init();
    let model = match std::env::var("VOGA_VOICE_MODEL") {
        Ok(path) => std::path::PathBuf::from(path),
        Err(_) => std::path::PathBuf::from(std::env::var("VOGA_DATA_DIR").expect("VOGA_VOICE_MODEL or VOGA_DATA_DIR"))
            .join("models")
            .join("wespeaker_en_voxceleb_resnet34_LM.onnx"),
    };
    let turns = [
        ("Luciana", "Boa noite. Eu sou candidata ao governo e quero falar sobre a saúde pública da nossa cidade."),
        ("Eddy", "Obrigado, candidata. Agora a palavra é do segundo candidato, que tem um minuto para responder."),
        ("Luciana", "Nos últimos quatro anos nós abrimos três hospitais e contratamos mais de mil médicos."),
        ("Eddy", "O tempo terminou. Vamos para o próximo bloco do debate, sobre educação e segurança."),
    ];
    voices::start(&model, Vec::new(), |change| println!("voice {} announced", change.view.number)).unwrap();
    let mut at = 0.0;
    let mut stretches = Vec::new();
    for (voice, text) in turns {
        let samples = say(voice, text);
        let end = at + samples.len() as f64 / 16_000.0;
        voices::offer(&samples, at, end);
        stretches.push((voice, at, end));
        at = end + 0.5;
    }
    // The worker embeds the stretches in the background
    let last = stretches.last().unwrap();
    for _ in 0..200 {
        if voices::voice_during(last.1, last.2).is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let heard: Vec<(&str, Option<String>)> = stretches.iter().map(|(v, s, e)| (*v, voices::voice_during(*s, *e))).collect();
    println!("{heard:?}");
    let announced = voices::stop();
    assert_eq!(heard[0].1, heard[2].1, "Luciana is one voice");
    assert_eq!(heard[1].1, heard[3].1, "Eddy is one voice");
    assert_ne!(heard[0].1, heard[1].1, "two people");
    assert!(heard.iter().all(|(_, v)| v.is_some()));
    assert_eq!(announced.len(), 2);
}
