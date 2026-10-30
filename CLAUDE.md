# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Required for Every Change: Version and Release Notes

Every new feature or fix that changes the app (not docs-only, tests-only or refactors with no user-visible effect) must, in the same change set:

1. **Bump the app version** (semver, pre-1.0): a new feature or a visible behavior change bumps the minor version (`0.1.0` → `0.2.0`), and a fix bumps the patch version (`0.1.0` → `0.1.1`). Bump once per change set, not once per commit. Keep all four places identical:
   - `frontend/package.json` (`"version"`)
   - `frontend/src-tauri/tauri.conf.json` (`"version"`)
   - `frontend/src-tauri/Cargo.toml` (`[package] version`)
   - `Cargo.lock` (the `voga` package entry)
2. **Add an entry at the top of [RELEASE_NOTES.md](RELEASE_NOTES.md)** as `## [x.y.z] - YYYY-MM-DD`, with `### Novidades` / `### Mudanças` / `### Correções` / `### Limitações conhecidas` as needed.
   - Written in Brazilian Portuguese, for the people who use the app: what they can now do or what was fixed, not file or function names.
   - Never edit the entry of a version that was already released; describe corrections in the new version.
   - `scripts/generate-update-manifest-github.js` uses this entry as the in-app update notes.
3. If several changes go into the same unreleased version, update that version's existing entry instead of bumping again.

Never commit signing keys. They live in `~/.voga/` (`updater.key`, `updater.key.password`), outside the repo.

## Project Overview

**Voga** follows public figures, starting with Brazilian politicians: it collects what they promise, say and do (web pages, RSS feeds, YouTube subtitles, Câmara/Senado/TSE open data), indexes it locally, extracts promises and statements, flags contradictions and fulfilled promises for human review, and shows the connections in a graph, enriched with free public data (Receita record and partners, CEIS/CNEP/CEPIM and São Paulo state sanctions for each company). Live sessions (Ao vivo) hear a candidate as they speak, transcribe on this computer, tell the voices apart and fact-check each claim against the records. The graph can also be sent to flowsint. Every feature is always available: there is no trial, license key or per-build feature list.

Its core was transplanted from Assunta (`../../assunta`): the settings, the UI shell, mock mode, the build scripts, and the live recording (sound capture, Silero VAD, Whisper, live speakers). Voga is independent from it: copy code over, never import across repos. Voga is GPL-3.0 (LICENSE.md). Its built-in models engine (builtin/, llama-helper), LLM client (llm/llm_client.rs), tray, updater UI, update manifest script, Whisper cleanup and ONNX Runtime build script were written for Voga: never copy Assunta's versions of them back. List every library whose code ships in THIRD_PARTY_NOTICES.md.

- **Desktop app**: Tauri 2 (Rust) + Next.js 14 (static export) + React 18 + Tailwind + shadcn/Radix
- **Storage**: SQLite via sqlx (`voga.sqlite` in the app data folder), migrations in `frontend/src-tauri/migrations`
- **LLM**: built-in by default (llama.cpp in the `llama-helper` sidecar, GGUF models downloaded to `<app data>/models`, `src/builtin`); optional Ollama (this computer or the LAN), Claude, OpenAI, OpenRouter, Groq, OpenAI-compatible servers, Claude Code CLI (`src/llm`)
- **Live sessions**: whisper.cpp in the app itself (whisper-rs, Metal on macOS; `--llama=cuda|vulkan` also picks Whisper's GPU), Silero VAD and WeSpeaker voice embeddings on ONNX Runtime (`ort`: linked statically on macOS/Linux, `onnxruntime.dll` fetched by `build/onnxruntime.rs` and bundled on Windows), sound through cpal and, on macOS, a Core Audio process tap (cidre; CoreAudio is linked weakly in build.rs so the app still opens before macOS 14.2)
- **External AI for modest computers**: Settings › AI › Connect your AI (`components/ProviderConnect.tsx`) takes one provider + key and sets chat, search and live transcription where the provider offers them (`lib/providers.ts`: capabilities and model picks from the provider's own `/models`, via `providers::list_provider_models`; model names are never hard-coded). OpenAI-compatible bases live in `providers::openai_base` (chat in llm_client, `rag::embeddings::OpenAiEmbedder`, `live::remote::RemoteTranscriber`); prefs `transcription_provider`/`transcription_remote_model`. `builtin_ai_computer_profile` (`is_basic`: < 8 GB, or no GPU and < 16 GB) makes the first run recommend it. `node scripts/check-providers.mjs` checks the picks.
- **No server required, not even on the LAN.** Every feature must work with the built-in models alone; Ollama and cloud providers are options the user configures. Enrichment uses only free, keyless sources that Voga's robots.txt rules allow (BrasilAPI and Wikidata are robots-disallowed). flowsint is an option the user sets up: it runs on this computer as the Docker Compose project `voga-flowsint` (Docker Desktop/OrbStack required), or on a server.

## Commands

All from `frontend/`:

```bash
pnpm install
pnpm tauri:dev              # desktop app (dev server on :3128)
pnpm tauri:build            # release build; signs updater artifacts with ~/.voga/updater.key
pnpm dev:mock               # frontend in a plain browser with demo data (http://localhost:3128)
pnpm tauri:dev:mock         # desktop app showing demo data
pnpm build:mock             # static demo site in out-mock/
pnpm typecheck              # tsc --noEmit
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

Build options (flags to `scripts/tauri-auto.js`, see its header): `--mock`, `--fresh` (first run erases the app data), `--llama=cuda|vulkan|cpu` (GPU of the built-in models; default Metal on macOS, CPU elsewhere). `node scripts/tauri-auto.js name …` prints the build's app name.

`dev` and `build` first build the `llama-helper` sidecar (always `--release`: a debug llama.cpp is unusably slow) and copy it to `src-tauri/binaries/llama-helper-<target triple>` (git-ignored), which `bundle.externalBin` packs. `llama-cpp-2` and `llama-cpp-sys-2` are pinned to the same version (=0.1.146, as in Assunta): a newer sys crate does not compile against it.

### Mock mode

`NEXT_PUBLIC_MOCK=1` (set by `scripts/mock.js`) aliases `@tauri-apps/api/core` to `src/mocks/tauri-core.ts`: app commands are answered by `src/mocks/handlers.ts`; `plugin:*` commands go to the real IPC in the desktop app, or to `src/mocks/browser.ts` in a browser. **A new Tauri command used by the frontend needs a handler there** (unhandled commands log `[mock] no handler` and return `null`/`[]`).

## Architecture

```
llama-helper/         the built-in models' sidecar (llama.cpp via llama-cpp-2): JSON lines on stdin/stdout
                      (generate, embed; each names its model file, the last one per op stays loaded; all
                      layers on the GPU in metal/cuda/vulkan builds, the processor if that fails); built and
                      bundled by scripts/tauri-auto.js
frontend/src-tauri/src/
├── lib.rs            plugins, setup (window state, tray, database), command registration
├── database.rs       DatabaseManager: opens voga.sqlite, runs migrations, WAL recovery
├── settings.rs       Settings › AI (provider, model, Ollama endpoint, embedding provider + model, API keys)
├── llm/              ChatModel trait + ConfiguredChatModel; llm_client::generate per provider (builtin-ai: the
│                     best built-in model downloaded when none is chosen; an error until one is downloaded;
│                     Claude's Messages API; the others OpenAI-compatible, Ollama asked not to think)
├── builtin/          the built-in models (kind "summary" = chat): catalog.rs (names saved in settings, never
│                     renamed; chat templates and sampling; recommended first), files.rs (state from the file
│                     size, resumable downloads with builtin-ai-download-progress events, cancel = "CANCELLED"),
│                     helper.rs (llama-helper processes, one for chat, one for embeddings, one request at a
│                     time each, killed after 5 idle minutes or a timeout), data_dir() set at startup
├── net.rs            client_for(url): no proxy for local/LAN hosts (Ollama on the LAN)
├── prefs.rs          key → value preferences (analysis on/off, flowsint, sanctions check day…); yt-dlp never uses a login
├── investigations.rs commands for investigations, targets, sources, documents
├── collect/          collection
│   ├── mod.rs        run_source (by kind), process_item (queue), ingest_page / ingest_video, save_document
│   ├── fetch.rs      polite HTTP: robots.txt, ≥2 s per host, ETag/Last-Modified, size limit; a rustls client for
│   │                 hosts that refuse the native TLS (TLS 1.3 only, e.g. dados.mg.gov.br; macOS's stops at 1.2)
│   ├── extract.rs    Readability (dom_smoothie) → Markdown + title/site/author/date
│   ├── feeds.rs      RSS/Atom (feed-rs) and the news search feed
│   ├── youtube.rs    yt-dlp (downloaded to app data, updated weekly): subtitles (VTT) and search
│   ├── official/     Câmara, Senado, TSE, Portal da Transparência amendments (bulk ZIP, no key) and ALESP
│   │                 (XML files) connectors (sources camara_deputado / senado_senador / tse_candidato /
│   │                 transparencia_emendas / alesp_deputado / pncp_prefeitura / emendas_estaduais, made by
│   │                 linking a target): documents + entities/relations; bulk dumps cached in <app data>/bulk
│   │                 (bulk_file; bulk_file_named when two dumps share a file name),
│   │                 resumable downloads, per-stream cursor; scan_zip_csv / scan_xml stream files of
│   │                 hundreds of MB; remote_zip_entries reads single files out of a ZIP on a server by
│   │                 HTTP ranges (the TSE's proposal dumps: 1.4 GB for SP; prefs tse_full_proposals downloads
│   │                 them whole instead). Matches are `similar` unless same_name (ballot name, or the full name
│   │                 typed whole). investigations::forget_official: deleting an official source (or its target)
│   │                 removes its documents, the facts/entities they back and orphaned entities. transparencia.rs also reads who received each amendment (PAID_TO a
│   │                 cnpj:, PJ only; people only counted) and runs transferegov.rs (emendas Pix, same
│   │                 author code, PostgREST API). pncp.rs: a mayor's city hall (id = IBGE code, found by
│   │                 tse::find for PREFEITO rows via the IBGE API): purchases + contracts of the city's
│   │                 municipal executive bodies (CNPJs learned from purchases), one document per day,
│   │                 two months per hourly run (429 → backoff); a contract becomes CONTRACTED/FUNDS only
│   │                 when its supplier or amendment is already in the investigation. estaduais.rs: state
│   │                 deputies' amendments from the states' transparency portals (SP emendas.zip, MG Segov CSV,
│   │                 ES yearly CSVs found on the CKAN dataset page (its /api/ is robots-disallowed), BA ZIP), id
│   │                 "UF:author" as the state writes it; each portal repeats rows its own way (see its header);
│   │                 PAID_TO only for recipients with a CNPJ (MG, ES). Goiás left out (broken CNPJs, stale).
│   ├── gazettes.rs   municipal gazettes via Querido Diário (gazette_search, one per target; its passages
│   │                 are the document). Not the DOU: in.gov.br's robots.txt is Disallow: /
│   ├── (mod.rs)      social sources: a Bluesky/Mastodon profile (or an RSSHub feed) whose items are saved
│   │                 as `post` documents straight from the feed (feeds::social_feed_url)
│   ├── thumbs.rs     news pictures (documents.image_url from og:image / feed media / yt-dlp thumbnail, kept by
│   │                 collect::remember_image): prefs news_images url | image | both; the worker (idle, within each
│   │                 investigation's schedule) downloads up to 10 per tick via fetch::get_image (image/*, ≤ 2 MB,
│   │                 2 tries) to <app data>/thumbs/<doc id>.<ext> (image_path), served by the asset protocol
│   │                 (scope $APPDATA/thumbs/**); forget() deletes them with their documents
│   ├── schedule.rs   each investigation's collection schedule (`investigations.schedule` JSON: paused, a pace
│   │                 per group of sources overriding each source's every_minutes (not PNCP's catch-up),
│   │                 local hours + weekdays): gate() lists the closed investigations, whose due sources and
│   │                 queued pages wait (OPEN_SOURCE / OPEN_ITEM); run_now() lets a source and its queue through
│   └── scheduler.rs  the single worker: due sources, then the crawl queue, then missing embeddings,
│                     then analysis, then (idle) the AI's daily source suggestions
├── discover/         discovery, each investigation within its daily limits (`investigations.discovery`
│                     JSON {pagesPerDay, aiCallsPerDay, depth ≤ 2, autoApprove}, spent in `discovery_usage`):
│                     links.rs follows same-site article links of sources with `follow_links` (user page/rss
│                     by default): names in the link's text/address go in, ambiguous headlines go to the AI
│                     (one request per page); websearch.rs is the optional web search (Brave key or SearXNG
│                     address; Bing's /search is robots-disallowed), a `web_search` source per target while
│                     set up; suggest.rs asks the AI for searches (only searches: it invents URLs) for
│                     promises without evidence, as sources with status `suggested` + `note` (origin llm),
│                     approved or `rejected` (kept so they are never suggested again)
├── analyze/          the AI reads speeches, videos, news and government plans (run_batch, from the
│                     scheduler when the queue is empty, temperature 0): claims.rs extracts promises/
│                     statements of the targets and keeps only those whose quote is in the text
│                     (locate_quote); judge.rs compares each claim with records found by hybrid search
│                     (votes and bills first; procedural votes such as requerimentos and destaques are
│                     never evidence) and keeps CONTRADICTS / FULFILLS the model marks same_subject as
│                     `suggested` relations
├── dashboard.rs      Home: dashboard() = each investigation's InvestigationStatus (sources/failing, documents, unread,
│                     indexing, queued, schedule + next opening, contradictions/fulfilled per claim state
│                     (analyze::claim_state, the Rust twin of claimsService.claimState), insights = suggestions to
│                     review, latest confirmed finding/check/scandal else latest suggestion, disk estimate cached 10
│                     min); scoreboard() = per person followed, only what a person confirmed (+ to_review apart).
│                     scheduler Progress.runs: per-investigation done/total of the current run
├── scandals.rs       scandal entities (`scandal:<uuid>` marked by the user from a document, `scandal:<hash>` from the
│                     AI) + INVOLVED_IN (target person → scandal; user = confirmed, llm = suggested, reviewed with
│                     set_finding_status); suggest() runs in analyze_document on news that name a target and a case
│                     sign (one request; quote must be located); sanctioned(): `cnpj:` with sanctions ≤ 2 hops from a
│                     target (SPENT_WITH; AUTHORED/HEADS → PAID_TO/FUNDS/CONTRACTED), facts side by side, never an
│                     accusation. `live_eval_scandals` is the prompt's quality check
├── compare.rs        several investigations side by side (`compare_investigations`): same URLs, same keys in the
│                     graph (+ incoming ties), votes all sides cast (agreement), claim topics aligned by embedding,
│                     bridges (passages ≥ 0.9 × as close to the others' interest as to their own) and divergent
│                     passages (feed::interest profiles), weekly counts; `compare_summary` (Ask feature) writes it up
│                     citing passages (`live_compare_summary`)
├── feed.rs           the Feed (/feed, /journal): news/pages/videos/posts by investigation ids or kind (the Journal = interests), by date or relevance, read/
│                     starred/feedback marks; score_step (worker, after embeddings) sets documents.relevance =
│                     best passage's cosine to the investigation's interest (name + description + targets,
│                     Rocchio-moved by liked/disliked documents); reset() rescores an investigation
├── graph.rs          entity / relate (facts) / suggest (AI findings; never overwrites a reviewed one);
│                     load(GraphOptions) for the Map: votes and bills only when tied to a claim unless all_votes;
│                     load_many: several investigations merged by key (shared nodes kept, Node.investigations)
├── enrich.rs         free public data for `cnpj:` companies, by the worker when idle (step) and the Map's panel
│                     (enrich_entity): minhareceita.org record + partners (PARTNER_OF; a person partner's key
│                     hashes name + masked CPF, no digits kept; >10 partners only counted; companies that are
│                     only partners wait for the button), the CEIS/CNEP/CEPIM daily ZIPs (CGU, bulk_file) and São Paulo's
│                     sanctions (transparencia.sp.gov.br Sancoes.zip)
├── flowsint/         mod.rs: push to flowsint (local by default, prefs flowsint_mode = server for a server):
│                     login, custom types (Party, Bill, VoteSession, Candidacy, Promise, Statement, Record),
│                     one flowsint investigation + sketch per Voga investigation (prefs flowsint_sync:<id>),
│                     import/execute with only confirmed relations; flowsint MERGEs by label, so
│                     import_payload makes labels unique per type.
│                     local.rs: the flowsint on this computer: writes compose.yml (pinned 1.2.13, only the web
│                     app published on 127.0.0.1, no Docker socket) + .env (generated secrets) to
│                     <app data>/flowsint, finds the docker CLI outside the shell PATH, install (pull with
│                     progress events) / start / stop / remove (down -v --rmi all), creates Voga's account;
│                     a push starts it when needed
├── rag/              index and Ask (from Assunta): chunker, embeddings (built-in `builtin:<model>` or Ollama;
│                     changing the model re-embeds everything through embed_missing), store (SQLite
│                     vectors + FTS5), retriever (hybrid RRF), indexer, answer (plan → search → cited answer); search
│                     and Ask take a scope (one or more investigation ids, json_each); ask_history.scope keeps questions
│                     about several together
├── live/             live sessions (Ao vivo; capture, VAD, Whisper and voices from Assunta): audio.rs (mic via cpal,
│                     the computer's sound via a Core Audio tap or WASAPI loopback, each to 16 kHz with rubato,
│                     mixed), vad.rs (Silero; a stretch over 15 s is cut at its quietest moment), whisper.rs (pt,
│                     the names followed as initial prompt, PT hallucinations dropped), pauses.rs (lines cut at
│                     pauses), voices.rs (WeSpeaker embeddings clustered live: "Voz N: quem é?", targets'
│                     voiceprints name a voice; only with prefs voice_recognition = consent), check.rs (window of
│                     a person's last minute every 20 s of speech → checkable claims, quote located → evidence
│                     via analyze::evidence → verdict true/false/partly/unverified citing records, or unverified;
│                     web search + collected pages when unsettled), session.rs (threads audio → VAD → Whisper +
│                     voices, lines/voices/checks saved as they come, `<app data>/live/<id>.wav`, a `live`
│                     document when it ends, `recover` closes sessions the app quit during; `busy()` pauses the
│                     scheduler's analysis and discovery's AI), commands.rs
├── live_tests.rs     #[ignore] end-to-end checks against real sites with the built-in models or Ollama (see its header)
├── tray.rs, window_state.rs, fresh_install.rs, state.rs
frontend/src/
├── app/              pages (static export): / (Home: cards, Journal, scoreboard; no sidebar), /investigation (overview),
│                     /journal (?id=, ?starred=1), /feed (News of what is open), /compare, /sources, /documents,
│                     /document?id=, /claims, /live (?id=), /debate (?id=), /map, /ask, /settings
├── contexts/         InvestigationContext: the open context (one investigation, several compared, or the Journal; null
│                     on Home), kept per device; `scope` = the investigations open, `current` = the one when single
├── lib/nav.ts        the sidebar's pages per context (politics, interest, comparison, Journal)
├── components/       Sidebar (shown only with something open), MainContent, home/* (cards, news cards, scoreboard),
│                     ScandalDialog, ScandalsSection, CitationCard, DocumentList, AiSettings, CollectSettings, ThinkingOrb, ui/*
├── services/         invoke() wrappers + TS types mirroring the Rust camelCase structs
├── i18n/             I18nProvider, useI18n; messages/en.ts (keys) and pt-BR.ts
└── mocks/            mock mode
```

Investigations have a `kind`: `politics` (default) or `interest` (any subject: no gazette search, no official data, no claim analysis, no Promises/Map pages; a `topic` target is searched as loose words and its items need not mention it, relevance sorts them instead).

Data (SQLite): `investigations` → `targets` (who is followed; each gets `news_search` + `youtube_search` sources, and `web_search` while a web search service is set up) → `sources` (kind, schedule in `next_run_at`, `every_minutes` 0 = once) → `crawl_queue` (what feeds, searches and followed links listed; `via` feed|search|link, `depth`) → `documents` (Markdown text, video `segments`) → `rag_chunks` + `rag_chunks_fts`. Scheduling columns are compared as text: write them with `collect::schedule_time`.

Documents carry `image_url` / `image_path` (thumbs.rs); documents collected before 0.3.0 have none (never re-fetched).

Graph (from official data now, from the AI in phase 3): `entities` (unique `key` per investigation: `target:<id>` is the target's person, `party:PSB`, `camara:prop:<id>`, `cnpj:<digits>`…) and `relations` (MEMBER_OF, AUTHORED, VOTED {vote}, ABOUT, SPENT_WITH {amount}, CANDIDATE_IN, PARTNER_OF {role}, PAID_TO {amount} (amendment → recipient), HEADS (mayor → `prefeitura:<ibge>`), CONTRACTED {valor, objeto, orgao, pncp} (city hall → supplier), FUNDS (amendment → supplier), INVOLVED_IN (person → `scandal`, see scandals.rs); `status` confirmed/suggested/rejected). Never store CPF, voter id or e-mail from TSE data. Claims are entities of type promise/statement (attrs: quote, topic, stated_at, target_id, chunk_id, start_time) made by the target (MADE); the AI's CONTRADICTS/FULFILLS relations carry `rationale`, `confidence` and start `suggested`: only a person sets `confirmed` or `rejected`. Their props carry `actor`/`scope` (analyze/actor.rs: whose act the evidence is, from documents.source_id → sources.target_id: self / party (same MEMBER_OF) / ally (party in the claimant's candidacy `coalition`/`federation`) / other; `none` for news): only `self` (or none) findings decide a claim's state, and the judge is told whose act each record is. `live_eval_analysis` (live_tests.rs) is the quality check for prompt changes: a contradiction, a promise kept and an unrelated statement must come out right.

Live sessions: `live_sessions` (candidate, notes, status recording/done/interrupted, audio_path, document_id) → `live_segments` (start_time/end_time seconds, voice_id) + `live_voices` (number, target_id or label, resolved, recognized, centroid) + `live_checks` (quote, claim, verdict, rationale, evidence JSON, status suggested/confirmed/rejected); `targets.voiceprint` is the mean of the voices the user gave that person. A verdict needs a record cited (else `unverified`) and is a suggestion until a person confirms it; voices are biometric: nothing is embedded without consent, a voiceprint learns only from the user's answers, and withdrawing consent erases voiceprints and centroids. Claims are an `affirmation` or a `denial` (live_checks.kind); with `live_sessions.auto_check` off they stay `verdict = 'detected'` until `live_check_now` (check::check_now) checks them; a voice with `role = 'interviewer'` is never checked. Debates (/debate) are sessions outside every investigation (`investigation_id` NULL) whose people are `live_participants` (position, name, target in any investigation; `investigations::follow_person` reuses someone followed under that name or makes an investigation for them): `session::followed` is who a session follows either way, each claim is checked in the investigation of whoever said it, a debate needs voices, and its transcript becomes a `live` document in each participant's investigation (`voga://live/<id>`). The Debate view is derived in the frontend (`lib/debate.ts` `debateOf`: interviewer turns with "?" are the numbered questions, each person's turns after question N are their Answer N). `live_eval_factcheck` is the quality check for check.rs's prompts (a refuted claim, a confirmed one, an unrecorded statistic, a refuted denial); `live_transcribe_pt` and `live_voices` use macOS `say` voices.

flowsint (see above): six containers (Postgres, Redis, Neo4j, API, Celery worker, nginx web app) from ghcr.io/reconurge, never bundled in the app. Updating flowsint = bump `local::VERSION` and check `compose.yml` against upstream's docker-compose.prod.yml. `live_flowsint` installs it locally through Docker (or uses VOGA_FLOWSINT) and checks a real push (twice, no duplicates).

Collection rules: identify as Voga, obey robots.txt, one request at a time with ≥2 s per host, never re-fetch what is stored. Items found by a target's searches are kept only if they mention one of its names (`Hints::must_mention`); pages reached by links or by the AI's searches, if they mention anyone the investigation follows. Renaming a target updates only its search sources (its official sources keep their ids). Demo data in `src/mocks` uses fictional people and outlets only.

Patterns:
- **Commands**: `#[tauri::command]` functions take `tauri::State<'_, AppState>` and use `state.db_manager.pool()`; register them in `lib.rs`; wrap them in a `services/*.ts` object; add a mock handler.
- **UI text**: every string goes through `t('key')`; add the key to `en.ts` and `pt-BR.ts` (tsc checks pt-BR has every key).
- **Activity**: use `ThinkingOrb` (`thinking` for AI answers, `background` for long jobs, `working` for quick loading or saving) rather than a spinning icon.
- Errors: Rust uses `anyhow::Result` internally and `Result<T, String>` at the command boundary; the frontend shows `toast.error(t(...), { description: String(error) })`.

## Plan

The roadmap (phases 0.1.0 → 0.8.0: collection and Ask, official sources, promise/contradiction analysis, graph + flowsint, autonomous discovery, more sources, product polish) is in the approved plan kept outside the repo; ask the user for it if needed. Contradictions and fulfillments are always created as `suggested` and need a person to confirm them: Voga must show its evidence and never present an unreviewed suggestion as fact.
