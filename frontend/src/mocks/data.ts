/**
 * Demo dataset for mock mode. Everyone and every outlet here is fictional: a demo must never
 * put invented statements in a real person's mouth.
 */

import type { Claim } from '@/services/claimsService';
import type { DocumentSummary, Investigation, OfficialMatch, Source, Target, TimedText } from '@/services/investigationService';

const day = 86400000;
const ago = (days: number) => new Date(Date.now() - days * day).toISOString();

export const INVESTIGATIONS: Investigation[] = [
  {
    id: 'inv-demo',
    name: 'Mandato de Marina Exemplo',
    description: 'Promessas da campanha de 2022 da deputada federal (fictícia) Marina Exemplo e o que ela fez no mandato.',
    kind: 'politics',
    createdAt: ago(40),
    documentCount: 0,
    sourceCount: 0,
  },
  {
    id: 'inv-prefeita',
    name: 'Prefeitura de Cidade Exemplo',
    description: 'O que a prefeita (fictícia) Helena Prado prometeu e o que a prefeitura contratou.',
    kind: 'politics',
    createdAt: ago(30),
    documentCount: 0,
    sourceCount: 0,
  },
  {
    id: 'inv-ia',
    name: 'Inteligência artificial',
    description: 'Modelos, regras e uso de IA nas empresas e no governo.',
    kind: 'interest',
    createdAt: ago(10),
    documentCount: 0,
    sourceCount: 0,
  },
  {
    id: 'inv-interesse',
    name: 'Novos tratamentos para diabetes',
    description: 'Pesquisas, aprovações e testes de tratamentos para diabetes tipo 1.',
    kind: 'interest',
    createdAt: ago(20),
    documentCount: 0,
    sourceCount: 0,
  },
];

/** What a search of the official records finds for the demo deputy (fictional ids). */
export const OFFICIAL_MATCHES: OfficialMatch[] = [
  { kind: 'camara_deputado', id: '900001', label: 'Marina Exemplo', detail: 'Câmara · PDE-SP · legislatura 57' },
  { kind: 'tse_candidato', id: '2022:SP:990000000001', label: 'MARINA DO BAIRRO', detail: 'TSE 2022 · DEPUTADO FEDERAL · PDE 9090 · SÃO PAULO · ELEITO POR QP' },
  { kind: 'tse_candidato', id: '2022:MG:990000000777', label: 'MARINA EXEMPLO SOUZA', detail: 'TSE 2022 · DEPUTADO ESTADUAL · PXX 12345 · MINAS GERAIS · NÃO ELEITO', similar: true },
  { kind: 'transparencia_emendas', id: '9001', label: 'MARINA EXEMPLO', detail: 'Portal da Transparência · 24 emendas parlamentares (2023–2026)' },
  { kind: 'emendas_estaduais', id: 'MG:MARINA EXEMPLO SOUZA', label: 'MARINA EXEMPLO SOUZA', detail: 'Portal de Emendas de Minas Gerais · 38 emendas estaduais (2023–2026)', similar: true },
  { kind: 'pncp_prefeitura', id: '9999999', label: 'Prefeitura de Cidade Exemplo', detail: 'PNCP · licitações e contratos da prefeitura (SP) desde 01/01/2025' },
];

export const TARGETS: Target[] = [
  {
    id: 'target-marina',
    investigationId: 'inv-demo',
    kind: 'person',
    name: 'Marina Exemplo',
    aliases: ['Marina do Bairro'],
    official: OFFICIAL_MATCHES.slice(0, 2),
    createdAt: ago(40),
  },
  { id: 'target-reforma', investigationId: 'inv-demo', kind: 'topic', name: 'reforma tributária', aliases: [], official: [], createdAt: ago(39) },
  { id: 'target-helena', investigationId: 'inv-prefeita', kind: 'person', name: 'Helena Prado', aliases: [], official: [], createdAt: ago(30) },
  { id: 'target-ia', investigationId: 'inv-ia', kind: 'topic', name: 'inteligência artificial', aliases: ['IA generativa'], official: [], createdAt: ago(10) },
  { id: 'target-diabetes', investigationId: 'inv-interesse', kind: 'topic', name: 'tratamento diabetes tipo 1', aliases: ['insulina', 'transplante de ilhotas'], official: [], createdAt: ago(20) },
];

const source = (s: Partial<Source> & Pick<Source, 'id' | 'kind' | 'url'>): Source => ({
  investigationId: 'inv-demo',
  targetId: null,
  title: null,
  everyMinutes: 360,
  nextRunAt: new Date(Date.now() + 3 * 3600000).toISOString(),
  lastRunAt: ago(0.1),
  lastError: null,
  status: 'active',
  origin: 'user',
  followLinks: s.origin === undefined && (s.kind === 'rss' || s.kind === 'page'),
  note: null,
  createdAt: ago(40),
  documentCount: 0,
  queued: 0,
  ...s,
});

export const SOURCES: Source[] = [
  // Searches the AI suggested for promises nothing proves yet (fictional)
  source({
    id: 'src-sug-creches',
    kind: 'news_search',
    url: 'Marina Exemplo creches zona norte emenda',
    origin: 'llm',
    status: 'suggested',
    lastRunAt: null,
    note: 'Procura registros da promessa de construir creches em todos os bairros da zona norte.',
  }),
  source({
    id: 'src-sug-transporte',
    kind: 'youtube_search',
    url: 'Marina Exemplo tarifa zero ônibus discurso',
    origin: 'llm',
    status: 'suggested',
    everyMinutes: 1440,
    lastRunAt: null,
    note: 'Procura falas sobre a promessa de tarifa zero nos ônibus.',
  }),
  source({ id: 'src-news', kind: 'news_search', url: '"Marina Exemplo"', title: 'Marina Exemplo', targetId: 'target-marina', origin: 'target', documentCount: 2, queued: 3 }),
  source({ id: 'src-yt-search', kind: 'youtube_search', url: '"Marina Exemplo"', title: 'Marina Exemplo', targetId: 'target-marina', origin: 'target', everyMinutes: 1440, documentCount: 1 }),
  source({ id: 'src-reforma', kind: 'news_search', url: '"reforma tributária"', title: 'reforma tributária', targetId: 'target-reforma', origin: 'target' }),
  source({ id: 'src-rss', kind: 'rss', url: 'https://jornal-ficticio.example/politica/feed', title: 'Jornal Fictício · Política', everyMinutes: 60, documentCount: 1 }),
  source({
    id: 'src-camara',
    kind: 'camara_deputado',
    url: '900001',
    title: 'Marina Exemplo · Câmara · PDE-SP · legislatura 57',
    targetId: 'target-marina',
    origin: 'target',
    everyMinutes: 1440,
    documentCount: 3,
  }),
  source({
    id: 'src-tse',
    kind: 'tse_candidato',
    url: '2022:SP:990000000001',
    title: 'MARINA DO BAIRRO · TSE 2022 · DEPUTADO FEDERAL',
    targetId: 'target-marina',
    origin: 'target',
    everyMinutes: 0,
    status: 'done',
    documentCount: 1,
  }),
  source({
    id: 'src-channel',
    kind: 'youtube_channel',
    url: 'https://www.youtube.com/feeds/videos.xml?channel_id=UCdemo',
    title: 'Marina Exemplo (canal)',
    everyMinutes: 180,
    lastError: 'yt-dlp: ERROR: Sign in to confirm you’re not a bot',
  }),
  source({ id: 'src-helena-news', investigationId: 'inv-prefeita', targetId: 'target-helena', kind: 'news_search', url: '"Helena Prado"', title: 'Helena Prado', origin: 'target', documentCount: 1 }),
  source({ id: 'src-ia-news', investigationId: 'inv-ia', targetId: 'target-ia', kind: 'news_search', url: 'inteligência artificial', title: 'inteligência artificial', origin: 'target', documentCount: 3 }),
  source({ id: 'src-diabetes-news', investigationId: 'inv-interesse', targetId: 'target-diabetes', kind: 'news_search', url: 'tratamento diabetes tipo 1', title: 'tratamento diabetes tipo 1', origin: 'target', documentCount: 4 }),
  source({ id: 'src-diabetes-rss', investigationId: 'inv-interesse', kind: 'rss', url: 'https://saude-ficticia.example/feed', title: 'Revista Saúde Fictícia', everyMinutes: 60, documentCount: 1 }),
];

/** What the AI read in the demo documents: a contradiction to review, a promise kept, and one with nothing yet. */
export const CLAIMS: Claim[] = [
  {
    id: 'claim-impostos',
    kind: 'promise',
    text: 'Marina Exemplo promete votar pela simplificação dos impostos.',
    quote: 'eu vou votar pela simplificação dos impostos',
    topic: 'reforma tributária',
    statedAt: '2022-09-28',
    targetId: 'target-marina',
    startTime: 66,
    source: { documentId: 'doc-comicio', title: 'Comício de encerramento – Marina Exemplo 2022', url: 'https://www.youtube.com/watch?v=demo0000001', kind: 'video', site: 'YouTube' },
    findings: [
      {
        id: 'finding-voto',
        label: 'CONTRADICTS',
        status: 'suggested',
        confidence: 0.92,
        rationale: 'Prometeu votar pela simplificação dos impostos e votou "Não" no texto-base que institui o imposto sobre bens e serviços.',
        date: '2026-10-02',
        excerpt: 'Marina Exemplo votou "Não" na votação: Aprovado o texto-base do PLP 99/2025. Proposição: PLP 99/2025 – Institui o imposto sobre bens e serviços.',
        startTime: null,
        evidence: { documentId: 'doc-votacao-oficial', title: 'Votação 02/10/2026 – PLP 99/2025: Marina Exemplo votou Não', url: 'https://camara.example/proposicoes/900500#votacao-1', kind: 'vote', site: 'Câmara dos Deputados' },
        actor: { targetId: 'target-marina', name: 'Marina Exemplo', party: 'PDE', scope: 'self' },
      },
      {
        id: 'finding-colega',
        label: 'CONTRADICTS',
        status: 'suggested',
        confidence: 0.81,
        rationale: 'Otávio Fictício, do mesmo partido de Marina Exemplo, votou "Não" no texto-base que simplifica os impostos.',
        date: '2026-10-02',
        excerpt: 'Otávio Fictício votou "Não" na votação: Aprovado o texto-base do PLP 99/2025.',
        startTime: null,
        evidence: { documentId: 'doc-votacao-oficial', title: 'Votação 02/10/2026 – PLP 99/2025: Otávio Fictício votou Não', url: 'https://camara.example/proposicoes/900500#votacao-1-900002', kind: 'vote', site: 'Câmara dos Deputados' },
        actor: { targetId: 'target-otavio', name: 'Otávio Fictício', party: 'PDE', scope: 'party' },
      },
    ],
  },
  {
    id: 'claim-creches',
    kind: 'promise',
    text: 'Marina Exemplo promete lutar por uma creche em cada bairro da cidade.',
    quote: 'e vou lutar por uma creche em cada bairro da nossa cidade',
    topic: 'creches',
    statedAt: '2022-09-28',
    targetId: 'target-marina',
    startTime: 76,
    source: { documentId: 'doc-comicio', title: 'Comício de encerramento – Marina Exemplo 2022', url: 'https://www.youtube.com/watch?v=demo0000001', kind: 'video', site: 'YouTube' },
    findings: [
      {
        id: 'finding-projeto',
        label: 'FULFILLS',
        status: 'confirmed',
        confidence: 0.88,
        rationale: 'Apresentou o PL 1234/2025, que cria repasses federais para creches em bairros sem vagas.',
        date: '2025-12-18',
        excerpt: 'Ementa: Cria o programa Creche Perto de Casa, com repasses federais para creches em bairros sem vagas.',
        startTime: null,
        evidence: { documentId: 'doc-projeto', title: 'PL 1234/2025 – Cria o programa Creche Perto de Casa', url: 'https://camara.example/proposicoes/900321', kind: 'bill', site: 'Câmara dos Deputados' },
        actor: { targetId: 'target-marina', name: 'Marina Exemplo', party: 'PDE', scope: 'self' },
      },
    ],
  },
  {
    id: 'claim-comerciante',
    kind: 'statement',
    text: 'Marina Exemplo afirma que o pequeno comerciante paga tributos demais.',
    quote: 'o pequeno comerciante não aguenta mais pagar cinco tributos diferentes',
    topic: 'pequenas empresas',
    statedAt: '2022-09-28',
    targetId: 'target-marina',
    startTime: 71,
    source: { documentId: 'doc-comicio', title: 'Comício de encerramento – Marina Exemplo 2022', url: 'https://www.youtube.com/watch?v=demo0000001', kind: 'video', site: 'YouTube' },
    findings: [],
  },
];

/** Whose a demo document is comes from its source (handlers' docInvestigation). */
export interface DemoDocument extends Omit<DocumentSummary, 'investigationId'> {
  text: string;
  segments: TimedText[] | null;
  /** The Feed's marks and relevance (unread, not starred, no feedback when left out). */
  read?: boolean;
  starred?: boolean;
  feedback?: -1 | 0 | 1;
  relevance?: number | null;
  /** The picture the item came with. */
  imageUrl?: string | null;
}

/** A made-up picture (an SVG drawn here): the demo never shows a real outlet's photo. */
function picture(hue: number): string {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 160 90"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 55% 62%)"/><stop offset="1" stop-color="hsl(${(hue + 40) % 360} 50% 36%)"/></linearGradient></defs><rect width="160" height="90" fill="url(#g)"/><circle cx="120" cy="28" r="16" fill="hsl(${hue} 80% 88% / .6)"/><path d="M0 72 L38 50 L70 64 L108 40 L160 68 V90 H0Z" fill="hsl(${hue} 40% 22% / .45)"/></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

const hoursAgo = (hours: number) => new Date(Date.now() - hours * 3600000).toISOString();

/** An interest's article (fictional outlets and studies). */
const article = (id: string, sourceId: string, site: string, hours: number, relevance: number, title: string, text: string, hue: number | null = null): DemoDocument => ({
  id,
  sourceId,
  url: `https://${site.toLowerCase().replace(/[^a-z]+/g, '-')}.example/${id}`,
  kind: 'article',
  title,
  author: 'Redação',
  site,
  publishedAt: hoursAgo(hours),
  fetchedAt: hoursAgo(hours - 0.5),
  note: null,
  indexError: null,
  text,
  segments: null,
  relevance,
  imageUrl: hue === null ? null : picture(hue),
});

export const DOCUMENTS: DemoDocument[] = [
  article('doc-ilhotas', 'src-diabetes-news', 'Jornal Fictício', 3, 0.82, 'Estudo testa transplante de ilhotas sem imunossupressão em 12 pacientes',
    'Um grupo de pesquisa (fictício) relatou que 10 de 12 pacientes com diabetes tipo 1 deixaram de precisar de insulina um ano após receber ilhotas produzidas a partir de células-tronco, sem remédios contra a rejeição. Os autores pedem cautela: o estudo é pequeno e o acompanhamento, curto.', 200),
  article('doc-insulina-semanal', 'src-diabetes-rss', 'Revista Saúde Fictícia', 9, 0.74, 'Insulina de aplicação semanal chega à fase final de testes',
    'A insulina de ação prolongada, aplicada uma vez por semana, mostrou controle da glicemia semelhante ao das aplicações diárias em adultos com diabetes tipo 1, segundo resultados preliminares apresentados num congresso.', 160),
  article('doc-sensor', 'src-diabetes-news', 'Portal Exemplo', 30, 0.61, 'Sensor de glicose passa a ser oferecido na rede pública de Cidade Exemplo',
    'A prefeitura de Cidade Exemplo começa a distribuir sensores contínuos de glicose a crianças com diabetes tipo 1 atendidas pelas unidades básicas de saúde.', 30),
  article('doc-merenda', 'src-helena-news', 'Jornal Fictício', 30, 0.9, 'Operação Ponte Quebrada investiga compra de merenda em Cidade Exemplo',
    'A Polícia Federal (fictícia) deflagrou a Operação Ponte Quebrada. Segundo o Ministério Público, a prefeita Helena Prado é investigada por fraude na compra de merenda escolar. A prefeitura diz que colabora com as investigações.', 10),
  article('doc-ia-fabrica', 'src-ia-news', 'Portal Exemplo', 2, 0.79, 'Fábricas de Cidade Exemplo usam IA para prever falhas nas máquinas',
    'Indústrias (fictícias) da região passaram a usar modelos de IA que leem sensores das máquinas e avisam antes de uma peça quebrar. As empresas dizem ter reduzido paradas; os sindicatos pedem treinamento para os operadores.', 260),
  article('doc-ia-regras', 'src-ia-news', 'Jornal Fictício', 6, 0.71, 'Senado (fictício) debate regras para o uso de IA no serviço público',
    'O projeto em discussão exige que órgãos públicos digam quando uma decisão foi tomada com ajuda de IA e permite pedir revisão por uma pessoa.', 290),
  article('doc-ia-modelo', 'src-ia-news', 'Revista Tecnologia Fictícia', 20, 0.66, 'Modelo de IA em português roda em notebook comum',
    'Pesquisadores (fictícios) mostraram um modelo de linguagem treinado em textos em português que responde perguntas sem internet, num notebook de 16 GB de memória.', 220),
  article('doc-academia', 'src-diabetes-news', 'Portal Exemplo', 52, 0.21, 'Rede de academias anuncia promoção de verão',
    'A rede de academias (fictícia) oferece desconto na matrícula para novos alunos até o fim do mês. A notícia cita uma palestra sobre alimentação e diabetes.'),

  {
    id: 'doc-voto',
    sourceId: 'src-news',
    url: 'https://jornal-ficticio.example/politica/marina-exemplo-vota-contra-texto-base',
    kind: 'article',
    title: 'Marina Exemplo vota contra o texto-base da reforma tributária',
    author: 'Redação',
    site: 'Jornal Fictício',
    publishedAt: ago(12),
    fetchedAt: ago(12),
    note: null,
    indexError: null,
    text:
      '## Marina Exemplo vota contra o texto-base\n\nA deputada federal Marina Exemplo votou contra o texto-base da reforma tributária nesta quinta-feira. Durante a campanha de 2022, ela havia prometido apoiar a simplificação dos impostos sobre o consumo.\n\nEm nota, a assessoria afirmou que a mudança de posição se deve a alterações feitas pelo relator, que segundo ela aumentam a carga sobre serviços e prejudicam pequenos municípios.\n\nO texto foi aprovado por 382 votos a 118 e segue para o Senado.',
    segments: null,
  },
  {
    id: 'doc-comicio',
    sourceId: 'src-yt-search',
    url: 'https://www.youtube.com/watch?v=demo0000001',
    kind: 'video',
    title: 'Comício de encerramento – Marina Exemplo 2022',
    author: 'Marina Exemplo',
    site: 'YouTube',
    publishedAt: ago(780),
    fetchedAt: ago(30),
    note: null,
    indexError: null,
    text: '',
    segments: [
      { start: 62, end: 66, text: 'eu quero dizer pra vocês com toda a clareza' },
      { start: 66, end: 71, text: 'eu vou votar pela simplificação dos impostos' },
      { start: 71, end: 76, text: 'o pequeno comerciante não aguenta mais pagar cinco tributos diferentes' },
      { start: 76, end: 81, text: 'e vou lutar por uma creche em cada bairro da nossa cidade' },
    ],
  },
  {
    id: 'doc-creche',
    sourceId: 'src-rss',
    url: 'https://jornal-ficticio.example/cidades/emenda-creches',
    kind: 'article',
    title: 'Emenda da deputada destina R$ 4 milhões a creches na zona norte',
    author: 'Ana Repórter',
    site: 'Jornal Fictício',
    publishedAt: ago(90),
    fetchedAt: ago(89),
    note: null,
    indexError: null,
    text: 'Uma emenda parlamentar de Marina Exemplo destinou R$ 4 milhões para a construção de duas creches na zona norte. A prefeitura informou que as obras começam no próximo semestre.',
    segments: null,
  },
  {
    id: 'doc-votacao-oficial',
    sourceId: 'src-camara',
    url: 'https://camara.example/proposicoes/900500#votacao-1',
    kind: 'vote',
    title: 'Votação 02/10/2026 – PLP 99/2025: Marina Exemplo votou Não',
    author: null,
    site: 'Câmara dos Deputados',
    publishedAt: ago(12),
    fetchedAt: ago(11),
    note: null,
    indexError: null,
    text: 'Em 02/10/2026, na Câmara dos Deputados (PLEN), Marina Exemplo votou "Não" na votação: Aprovado o texto-base do PLP 99/2025.\nProposição: PLP 99/2025 – Institui o imposto sobre bens e serviços.\nResultado: aprovada (382 votos sim, 118 não).',
    segments: null,
  },
  {
    id: 'doc-projeto',
    sourceId: 'src-camara',
    url: 'https://camara.example/proposicoes/900321',
    kind: 'bill',
    title: 'PL 1234/2025 – Cria o programa Creche Perto de Casa',
    author: null,
    site: 'Câmara dos Deputados',
    publishedAt: ago(300),
    fetchedAt: ago(30),
    note: null,
    indexError: null,
    text: 'PL 1234/2025, apresentado com autoria de Marina Exemplo.\n\nEmenta: Cria o programa Creche Perto de Casa, com repasses federais para creches em bairros sem vagas.',
    segments: null,
  },
  {
    id: 'doc-cota',
    sourceId: 'src-camara',
    url: 'https://camara.example/deputados/900001#despesas-2026-08',
    kind: 'expense',
    title: 'Cota parlamentar de Marina Exemplo – 08/2026: R$ 41.230,55',
    author: null,
    site: 'Câmara dos Deputados',
    publishedAt: ago(60),
    fetchedAt: ago(30),
    note: null,
    indexError: null,
    text: 'Cota parlamentar (CEAP) de Marina Exemplo em 08/2026: total R$ 41.230,55.\n\nPor tipo de despesa:\n- DIVULGAÇÃO DA ATIVIDADE PARLAMENTAR: R$ 22.000,00\n- PASSAGEM AÉREA: R$ 12.980,10\n- COMBUSTÍVEIS: R$ 6.250,45',
    segments: null,
  },
  {
    id: 'doc-contratos',
    sourceId: 'src-pncp',
    url: 'https://pncp.example/app/contratos#prefeitura-9999999-2026-09-01',
    kind: 'contract',
    title: 'Contratos publicados pela Prefeitura de Cidade Exemplo em 01/09/2026: 2 (R$ 1.250.500,00)',
    author: null,
    site: 'PNCP',
    publishedAt: ago(44),
    fetchedAt: ago(30),
    note: null,
    indexError: null,
    text: 'Contratos publicados pela Prefeitura de Cidade Exemplo em 01/09/2026: 2 (R$ 1.250.500,00).\n\n- SECRETARIA DE EDUCAÇÃO: Gráfica Exemplo Ltda (CNPJ 11222333000181) – Impressão de material didático para creches – R$ 250.500,00; assinado em 2026-08-20, vigência de 2026-09-01 a 2027-08-31; processo 2026/0001; emenda parlamentar 202690010003\n- SECRETARIA DE OBRAS: Construtora Fictícia S.A. (CNPJ 44555666000199) – Reforma da creche do bairro Jardim Exemplo – R$ 1.000.000,00; assinado em 2026-08-25, vigência de 2026-09-01 a 2027-02-28; processo 2026/0002',
    segments: null,
  },
  {
    id: 'doc-licitacoes',
    sourceId: 'src-pncp',
    url: 'https://pncp.example/app/editais#prefeitura-9999999-2026-09-02',
    kind: 'procurement',
    title: 'Licitações publicadas pela Prefeitura de Cidade Exemplo em 02/09/2026: 1 (R$ 3.000.000,00 estimados)',
    author: null,
    site: 'PNCP',
    publishedAt: ago(43),
    fetchedAt: ago(30),
    note: null,
    indexError: null,
    text: 'Licitações publicadas pela Prefeitura de Cidade Exemplo em 02/09/2026: 1 (R$ 3.000.000,00 estimados).\n\n- SECRETARIA DE EDUCAÇÃO: Construção da creche Perto de Casa no bairro Vila Exemplo (Concorrência - Eletrônica; estimado R$ 3.000.000,00; Divulgada no PNCP; processo 2026/0040)',
    segments: null,
  },
  {
    id: 'doc-candidatura',
    sourceId: 'src-tse',
    url: 'https://tse.example/candidatos-2022#candidato-990000000001',
    kind: 'candidacy',
    title: 'Candidatura de MARINA DO BAIRRO a deputado federal em 2022 (PDE 9090): eleito por qp',
    author: null,
    site: 'TSE',
    publishedAt: '2022-10-02',
    fetchedAt: ago(30),
    note: null,
    indexError: null,
    text: 'Candidatura de MARINA EXEMPLO (MARINA DO BAIRRO) a DEPUTADO FEDERAL em SÃO PAULO nas eleições de 2022, pelo PDE, número 9090.\nResultado: ELEITO POR QP.\n\nBens declarados ao TSE (total R$ 312.000,00):\n- Apartamento: Apartamento em São Paulo – R$ 290.000,00\n- Depósito bancário: Conta corrente – R$ 22.000,00',
    segments: null,
  },
  {
    id: 'doc-entrevista',
    sourceId: 'src-news',
    url: 'https://www.youtube.com/watch?v=demo0000002',
    kind: 'video',
    title: 'Entrevista – Marina Exemplo explica voto',
    author: 'TV Exemplo',
    site: 'YouTube',
    publishedAt: ago(11),
    fetchedAt: ago(10),
    note: 'O YouTube pediu login para mostrar as legendas (escolha um navegador em Configurações › Coleta).',
    indexError: null,
    text: 'A deputada fala sobre a votação da reforma tributária.',
    segments: null,
  },
];
