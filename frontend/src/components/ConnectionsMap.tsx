'use client';

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { ChevronDown, Search, Share2, X } from 'lucide-react';
import { toast } from 'sonner';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Switch } from '@/components/ui/switch';
import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from '@/components/ui/command';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { formatDate, useI18n, type MessageKey } from '@/i18n';
import { openExternal } from '@/lib/openExternal';
import { flowsintService, FlowsintSync, Graph, graphService, GraphLink, GraphNode, GraphOptions } from '@/services/graphService';

/** The groups of the legend; documents of every kind are "records". */
type Group = 'person' | 'party' | 'organization' | 'candidacy' | 'bill' | 'amendment' | 'vote_session' | 'promise' | 'statement' | 'record';

export const GROUPS: { group: Group; color: string; label: MessageKey }[] = [
  { group: 'person', color: '#0f766e', label: 'map.kind.person' },
  { group: 'party', color: '#7c3aed', label: 'map.kind.party' },
  { group: 'organization', color: '#b45309', label: 'map.kind.organization' },
  { group: 'candidacy', color: '#2563eb', label: 'map.kind.candidacy' },
  { group: 'bill', color: '#0891b2', label: 'map.kind.bill' },
  { group: 'amendment', color: '#65a30d', label: 'map.kind.amendment' },
  { group: 'vote_session', color: '#64748b', label: 'map.kind.vote_session' },
  { group: 'promise', color: '#e9a23b', label: 'map.kind.promise' },
  { group: 'statement', color: '#db2777', label: 'map.kind.statement' },
  { group: 'record', color: '#94a3b8', label: 'map.kind.record' },
];
const COLOR = Object.fromEntries(GROUPS.map(g => [g.group, g.color])) as Record<Group, string>;
/** Labeled from a lower zoom than the rest. */
const HUBS = new Set<Group>(['person', 'party', 'candidacy']);
const LINK_COLOR: Record<string, string> = { CONTRADICTS: '#dc2626', FULFILLS: '#16a34a' };
/** Relations that carry money to their target (it is sized by what it received). */
const MONEY_LINKS = new Set(['SPENT_WITH', 'PAID_TO', 'CONTRACTED', 'FUNDS']);
/** The radius of whoever received the most. */
const MAX_MONEY_RADIUS = 24;
/** What each kind of line means: color, dashed, label. */
const LEGEND: [string, boolean, MessageKey][] = [
  ['currentColor', false, 'map.legend.fact'],
  ['currentColor', true, 'map.legend.unreviewed'],
  [LINK_COLOR.CONTRADICTS, false, 'map.legend.contradicts'],
  [LINK_COLOR.FULFILLS, false, 'map.legend.fulfills'],
];

const groupOf = (kind: string): Group => (GROUPS.some(g => g.group === kind) ? (kind as Group) : 'record');

type SimNode = GraphNode & { group: Group; x: number; y: number; vx: number; vy: number; r: number; held?: boolean };
/** `own`: a finding about the claimant's own act (or a report); another person's act is drawn muted. */
type SimLink = { a: SimNode; b: SimNode; strength: number; bias: number; label: string; status: string; vote: string | null; own: boolean };

const REPULSION = 120;
const LINK_LENGTH = 45;
const GRAVITY = 0.04;
const ALPHA_DECAY = 0.0228; // ~300 ticks to settle, as d3-force
const ALPHA_MIN = 0.002;

/** One tick of a d3-style force layout: repulsion, springs on links, a pull to the center. */
function tick(nodes: SimNode[], links: SimLink[], alpha: number) {
  // ponytail: O(n²) repulsion, fine to ~1.5k nodes; swap in d3-force (Barnes–Hut) beyond that
  for (let i = 0; i < nodes.length; i++) {
    const a = nodes[i];
    for (let j = i + 1; j < nodes.length; j++) {
      const b = nodes[j];
      const dx = a.x - b.x || Math.random() - 0.5;
      const dy = a.y - b.y || Math.random() - 0.5;
      const f = (REPULSION * alpha) / Math.max(dx * dx + dy * dy, 1);
      a.vx += dx * f;
      a.vy += dy * f;
      b.vx -= dx * f;
      b.vy -= dy * f;
    }
  }
  for (const { a, b, strength, bias } of links) {
    const dx = b.x + b.vx - a.x - a.vx;
    const dy = b.y + b.vy - a.y - a.vy;
    const d = Math.hypot(dx, dy) || 1;
    const f = ((d - LINK_LENGTH) / d) * alpha * strength;
    b.vx -= dx * f * bias;
    b.vy -= dy * f * bias;
    a.vx += dx * f * (1 - bias);
    a.vy += dy * f * (1 - bias);
  }
  for (const n of nodes) {
    n.vx -= n.x * GRAVITY * alpha;
    n.vy -= n.y * GRAVITY * alpha;
    n.vx *= 0.6;
    n.vy *= 0.6;
    if (!n.held) {
      n.x += n.vx;
      n.y += n.vy;
    }
  }
}

function cssColor(name: string, fallback: string): string {
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value ? `hsl(${value})` : fallback;
}

function truncate(text: string, max: number) {
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

const plain = (text: string) => text.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();

/** A group's chip: opens the list of its nodes, to pick the ones the map focuses on. */
function GroupFilter({
  group,
  items,
  picked,
  hidden,
  onPick,
  onAll,
}: {
  group: (typeof GROUPS)[number];
  items: GraphNode[];
  picked: Set<string> | undefined;
  hidden: boolean;
  onPick: (id: string) => void;
  onAll: () => void;
}) {
  const { t } = useI18n();
  const count = items.filter(n => picked?.has(n.id)).length;
  const all = !hidden && count === 0;
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          className={`flex items-center gap-1.5 rounded-full border border-gray-200 px-2.5 py-1 text-xs ${hidden ? 'text-gray-400' : 'text-gray-700 hover:bg-gray-50'}`}
        >
          <span className="h-2.5 w-2.5 rounded-full" style={{ background: hidden ? 'transparent' : group.color, border: `1.5px solid ${group.color}` }} />
          {t(group.label)} <span className="text-gray-400">{count ? `${count}/${items.length}` : items.length}</span>
          <ChevronDown className="h-3 w-3 text-gray-400" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 p-0">
        <label className="flex cursor-pointer items-center gap-2 border-b px-3 py-2 text-sm hover:bg-accent">
          <Checkbox checked={all} onCheckedChange={onAll} />
          {t('map.all')}
        </label>
        <Command filter={(_, search, keywords) => (plain((keywords ?? []).join(' ')).includes(plain(search)) ? 1 : 0)}>
          <CommandInput placeholder={t('map.filter')} />
          {/* ponytail: every item is rendered; virtualize if a group reaches the thousands */}
          <CommandList className="max-h-[300px]">
            <CommandEmpty>{t('map.noMatch')}</CommandEmpty>
            {items.map(n => (
              <CommandItem key={n.id} value={n.id} keywords={[n.label]} onSelect={() => onPick(n.id)} title={n.label}>
                <Checkbox checked={!!picked?.has(n.id)} tabIndex={-1} className="pointer-events-none" />
                <span className="truncate">{n.label}</span>
              </CommandItem>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

function FlowsintButton({ investigationId }: { investigationId: string }) {
  const { t, locale } = useI18n();
  const [sync, setSync] = useState<FlowsintSync | null>(null);
  const router = useRouter();
  const [pushing, setPushing] = useState(false);
  // The flowsint on this computer may need to start first (a minute or so)
  const [starting, setStarting] = useState(false);

  useEffect(() => {
    flowsintService.sync(investigationId).then(setSync).catch(() => setSync(null));
  }, [investigationId]);

  useEffect(() => {
    if (!pushing) return;
    const unlisten = flowsintService.onProgress(() => setStarting(true));
    return () => {
      unlisten.then(fn => fn());
    };
  }, [pushing]);

  const push = async () => {
    setPushing(true);
    try {
      const result = await flowsintService.push(investigationId);
      setSync(await flowsintService.sync(investigationId));
      const description = result.errors.length ? t('map.flowsintErrors', { count: result.errors.length, first: result.errors[0] }) : undefined;
      toast.success(t('map.flowsintSent', { nodes: result.nodes, edges: result.edges }), {
        description,
        action: { label: t('map.flowsintOpen'), onClick: () => openExternal(result.url) },
      });
    } catch (error) {
      toast.error(t('map.flowsintFailed'), {
        description: String(error),
        action: { label: t('home.openSettings'), onClick: () => router.push('/settings?tab=flowsint') },
      });
    } finally {
      setPushing(false);
      setStarting(false);
    }
  };

  return (
    <div className="flex items-center gap-2">
      {sync && (
        <button onClick={() => openExternal(sync.url)} className="text-xs text-gray-500 hover:text-gray-800 hover:underline">
          {t('map.flowsintLast', { date: formatDate(locale, sync.pushedAt, { dateStyle: 'short', timeStyle: 'short' }) })}
        </button>
      )}
      <Button variant="outline" size="sm" onClick={push} disabled={pushing}>
        {pushing ? <ThinkingOrb variant="working" size={16} className="mr-2" /> : <Share2 className="mr-2 h-4 w-4" />}
        {starting ? t('map.flowsintStarting') : t('map.flowsintSend')}
      </Button>
    </div>
  );
}

type Sanction = { cadastro: string; categoria: string; orgao: string; inicio: string; fim: string; processo: string };

function parseAttrs(node: GraphNode): Record<string, unknown> {
  try {
    return JSON.parse(node.attrs || '{}');
  } catch {
    return {};
  }
}

/** A connection's details: when, how much, what, and the record that shows it. */
function linkFacts(l: GraphLink) {
  let props: Record<string, unknown> = {};
  try {
    props = JSON.parse(l.props || '{}');
  } catch {}
  const amount = Number(props.amount ?? props.valor);
  const what = typeof props.objeto === 'string' ? props.objeto : typeof props.role === 'string' ? props.role : null;
  const scope = typeof props.scope === 'string' ? props.scope : null;
  return { amount: Number.isFinite(amount) && amount > 0 ? amount : null, what, scope };
}

/** "2023-05" → 05/2023; "2026-08-20" → the locale's date. */
function linkDate(locale: Parameters<typeof formatDate>[0], date: string) {
  if (/^\d{4}-\d{2}$/.test(date)) return `${date.slice(5)}/${date.slice(0, 4)}`;
  return date ? formatDate(locale, date, { dateStyle: 'short' }) : '';
}

/** A clicked node: what is known about it, its connections, and for a company the public data. */
function NodePanel({
  node,
  links,
  labelOf,
  relationText,
  onOpen,
  onOpenDocument,
  onClose,
  onEnriched,
}: {
  node: GraphNode;
  links: GraphLink[];
  labelOf: (id: string) => string;
  relationText: (label: string, vote: string | null) => string;
  onOpen?: () => void;
  onOpenDocument: (id: string) => void;
  onClose: () => void;
  onEnriched: () => void;
}) {
  const { t, locale } = useI18n();
  const [enriching, setEnriching] = useState(false);
  const money = (value: number) => value.toLocaleString(locale, { style: 'currency', currency: 'BRL' });
  // The same connection repeated (a supplier paid every month): one line, its occurrences inside
  const groups = new Map<string, { other: string; incoming: boolean; label: string; vote: string | null; status: string; links: GraphLink[] }>();
  for (const l of links) {
    const incoming = l.target === node.id;
    const other = incoming ? l.source : l.target;
    const key = `${l.label}|${other}|${incoming}|${l.status}|${l.label === 'VOTED' ? l.vote : ''}`;
    if (!groups.has(key)) groups.set(key, { other, incoming, label: l.label, vote: l.vote, status: l.status, links: [] });
    groups.get(key)!.links.push(l);
  }
  groups.forEach(g => g.links.sort((a, b) => b.date.localeCompare(a.date)));
  const group = GROUPS.find(g => g.group === groupOf(node.kind))!;
  const attrs = parseAttrs(node);
  const cnpj = typeof attrs.cnpj === 'string' ? attrs.cnpj : null;
  const sanctions = Array.isArray(attrs.sanctions) ? (attrs.sanctions as Sanction[]) : null;
  const partners = typeof attrs.socios === 'number' ? attrs.socios : null;
  const facts: [MessageKey, unknown][] = [
    ['map.panel.cnpj', cnpj],
    ['map.panel.name', attrs.razao_social],
    ['map.panel.status', attrs.situacao],
    ['map.panel.activity', attrs.cnae],
    ['map.panel.place', [attrs.municipio, attrs.uf].filter(Boolean).join(' / ')],
    ['map.panel.opened', attrs.abertura],
  ];

  const enrich = async () => {
    setEnriching(true);
    try {
      await graphService.enrich(node.id);
      onEnriched();
    } catch (error) {
      toast.error(t('map.panel.enrichFailed'), { description: String(error) });
    } finally {
      setEnriching(false);
    }
  };

  return (
    <div className="absolute bottom-4 right-4 top-4 flex w-80 flex-col gap-3 overflow-y-auto rounded-md border border-gray-200 bg-white/95 p-4 text-sm text-gray-700 shadow-sm">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <span className="text-xs font-medium" style={{ color: group.color }}>
            {t(group.label)}
          </span>
          <p className="break-words font-medium text-gray-900">{node.label}</p>
        </div>
        <button onClick={onClose} aria-label={t('map.panel.close')} className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-700">
          <X className="h-4 w-4" />
        </button>
      </div>

      {cnpj && (
        <>
          <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
            {facts
              .filter(([, value]) => value !== undefined && value !== null && value !== '')
              .map(([key, value]) => (
                <React.Fragment key={key}>
                  <dt className="text-gray-500">{t(key)}</dt>
                  <dd className="break-words">{String(value)}</dd>
                </React.Fragment>
              ))}
          </dl>
          {typeof attrs.receita_error === 'string' && <p className="text-xs text-gray-500">{t('map.panel.receitaError')}</p>}
          {partners !== null && partners > 10 && <p className="text-xs text-gray-500">{t('map.panel.partnersMany', { count: partners })}</p>}
          <div>
            <p className="text-xs font-medium text-gray-500">{t('map.panel.sanctions')}</p>
            {sanctions === null ? (
              <p className="text-xs text-gray-500">{t('map.panel.notChecked')}</p>
            ) : sanctions.length === 0 ? (
              <p className="text-xs text-gray-500">{t('map.panel.noSanctions', { date: String(attrs.sanctions_checked ?? '') })}</p>
            ) : (
              <ul className="mt-1 space-y-2">
                {sanctions.map((s, i) => (
                  <li key={i} className="rounded border border-red-200 bg-red-50 px-2 py-1.5 text-xs text-red-800">
                    <span className="font-medium">{s.cadastro}</span> · {s.categoria}
                    <br />
                    {s.orgao}
                    <br />
                    {s.inicio}
                    {s.fim && ` – ${s.fim}`}
                    {s.processo && ` · ${s.processo}`}
                  </li>
                ))}
              </ul>
            )}
          </div>
          <Button variant="outline" size="sm" onClick={enrich} disabled={enriching} title={t('map.panel.enrichHelp')}>
            {enriching ? <ThinkingOrb variant="working" size={16} className="mr-2" /> : <Search className="mr-2 h-4 w-4" />}
            {t('map.panel.enrich')}
          </Button>
        </>
      )}

      {onOpen && (
        <Button variant="outline" size="sm" onClick={onOpen}>
          {t('map.panel.open')}
        </Button>
      )}

      {links.length > 0 && (
        <div>
          <p className="text-xs font-medium text-gray-500">{t('map.panel.connections', { count: links.length })}</p>
          <ul className="mt-1 space-y-1.5 text-xs">
            {[...groups.values()].slice(0, 50).map(g => {
              const rel = (
                <span className="text-gray-500">
                  {relationText(g.label, g.vote)}
                  {g.status === 'suggested' && ` (${t('map.unreviewed')})`}
                </span>
              );
              const total = g.links.reduce((sum, l) => sum + (linkFacts(l).amount ?? 0), 0);
              const occurrences = (
                <ul className="mt-0.5 space-y-0.5 border-l border-gray-200 pl-2 text-gray-600">
                  {g.links.map(l => {
                    const { amount, what } = linkFacts(l);
                    return (
                      <li key={l.id} className="break-words">
                        {[linkDate(locale, l.date), amount !== null && money(amount), what].filter(Boolean).join(' · ')}{' '}
                        {l.documentId && (
                          <button onClick={() => onOpenDocument(l.documentId!)} className="text-blue-700 hover:underline">
                            {t('map.panel.evidence')}
                          </button>
                        )}
                      </li>
                    );
                  })}
                </ul>
              );
              const head = (
                <span className="break-words">
                  {g.incoming ? (
                    <>
                      {labelOf(g.other)} {rel}
                    </>
                  ) : (
                    <>
                      {rel} {labelOf(g.other)}
                    </>
                  )}
                  {g.links.length > 1 && <span className="text-gray-500"> · {t('map.panel.times', { count: g.links.length })}</span>}
                  {total > 0 && <span className="text-gray-500"> · {money(total)}</span>}
                </span>
              );
              const detailed = g.links.some(l => l.date || l.documentId || linkFacts(l).amount !== null);
              return (
                <li key={`${g.label}|${g.other}|${g.incoming}|${g.status}|${g.vote}`}>
                  {g.links.length > 1 ? (
                    <details>
                      <summary className="cursor-pointer">{head}</summary>
                      {occurrences}
                    </details>
                  ) : (
                    <>
                      {head}
                      {detailed && occurrences}
                    </>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}

/** The investigation's connections: people, parties, companies, bills, votes and claims. */
/** The map of one investigation, or of several together (what they share drawn with a ring). */
export function ConnectionsMap({ investigationIds }: { investigationIds: string[] }) {
  const investigationId = investigationIds.join(',');
  const { t } = useI18n();
  const router = useRouter();
  const [graph, setGraph] = useState<Graph | null>(null);
  const [options, setOptions] = useState<GraphOptions>({ allVotes: false, suggested: true });
  const [hidden, setHidden] = useState<Set<Group>>(new Set());
  // Nodes picked in a group's list; none picked means the whole group
  const [picked, setPicked] = useState<Map<Group, Set<string>>>(new Map());
  const [hovered, setHovered] = useState<SimNode | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  // `fit`: keep the whole graph in view while it settles, until the user pans, zooms or drags
  const sim = useRef({ nodes: [] as SimNode[], links: [] as SimLink[], alpha: 1, dirty: true, fit: true });
  const view = useRef({ x: 0, y: 0, k: 1 });
  const positions = useRef(new Map<string, { x: number; y: number }>());
  const hoverRef = useRef<SimNode | null>(null);
  // The clicked node stays lit while nothing is hovered
  const selectedRef = useRef<string | null>(null);

  const load = useCallback(() => {
    graphService
      .load(investigationId.split(','), options)
      .then(setGraph)
      .catch(error => {
        console.error('Failed to load the map:', error);
        setGraph({ nodes: [], links: [] });
      });
  }, [investigationId, options]);

  useEffect(load, [load]);

  const counts = useMemo(() => {
    const c = new Map<Group, number>();
    graph?.nodes.forEach(n => c.set(groupOf(n.kind), (c.get(groupOf(n.kind)) ?? 0) + 1));
    return c;
  }, [graph]);

  const neighbors = useMemo(() => {
    const m = new Map<string, Set<string>>();
    graph?.links.forEach(({ source, target }) => {
      if (!m.has(source)) m.set(source, new Set());
      if (!m.has(target)) m.set(target, new Set());
      m.get(source)!.add(target);
      m.get(target)!.add(source);
    });
    return m;
  }, [graph]);

  const byGroup = useMemo(() => {
    const m = new Map<Group, GraphNode[]>();
    graph?.nodes.forEach(n => m.set(groupOf(n.kind), [...(m.get(groupOf(n.kind)) ?? []), n]));
    m.forEach(nodes => nodes.sort((a, b) => a.label.localeCompare(b.label)));
    return m;
  }, [graph]);

  // With nodes picked, the map shows them and what ties to them (two steps away past non-hubs)
  const shown = useMemo(() => {
    const roots = graph?.nodes.filter(n => picked.get(groupOf(n.kind))?.has(n.id)) ?? [];
    if (!graph || roots.length === 0) return null;
    const groupById = new Map(graph.nodes.map(n => [n.id, groupOf(n.kind)]));
    const ids = new Set<string>();
    for (const root of roots) {
      ids.add(root.id);
      for (const near of neighbors.get(root.id) ?? []) {
        ids.add(near);
        if (!HUBS.has(groupById.get(near)!)) neighbors.get(near)?.forEach(far => ids.add(far));
      }
    }
    return ids;
  }, [graph, picked, neighbors]);

  // (Re)build the simulation for the visible groups, keeping nodes where they were
  useEffect(() => {
    if (!graph) return;
    const previous = sim.current.nodes;
    previous.forEach(n => positions.current.set(n.id, { x: n.x, y: n.y }));
    const spread = Math.sqrt(graph.nodes.length) * 12;
    const near = (id: string) => {
      for (const other of neighbors.get(id) ?? []) {
        const at = positions.current.get(other);
        if (at) return { x: at.x + Math.random() * 20 - 10, y: at.y + Math.random() * 20 - 10 };
      }
    };
    const byId = new Map<string, SimNode>();
    for (const n of graph.nodes) {
      const group = groupOf(n.kind);
      if (hidden.has(group) || (shown && !shown.has(n.id))) continue;
      const at = positions.current.get(n.id) ?? near(n.id);
      byId.set(n.id, { ...n, group, x: at?.x ?? (Math.random() - 0.5) * spread, y: at?.y ?? (Math.random() - 0.5) * spread, vx: 0, vy: 0, r: 0 });
    }
    const pairs = graph.links.flatMap(l => {
      const a = byId.get(l.source);
      const b = byId.get(l.target);
      return a && b ? [{ a, b, l }] : [];
    });
    const degree = new Map<SimNode, number>();
    pairs.forEach(({ a, b }) => {
      degree.set(a, (degree.get(a) ?? 0) + 1);
      degree.set(b, (degree.get(b) ?? 0) + 1);
    });
    const nodes = [...byId.values()];
    // Who received money (a company paid by quota, amendment or contract) is sized by how much it
    // received in all, its area proportional to the amount; the rest by how connected it is
    const received = new Map<string, number>();
    for (const l of graph.links) {
      const amount = MONEY_LINKS.has(l.label) ? linkFacts(l).amount : null;
      if (amount) received.set(l.target, (received.get(l.target) ?? 0) + amount);
    }
    const most = Math.max(0, ...nodes.map(n => received.get(n.id) ?? 0));
    nodes.forEach(n => {
      const money = received.get(n.id);
      n.r = money && most ? 3 + Math.sqrt(money / most) * MAX_MONEY_RADIUS : 3 + Math.sqrt(degree.get(n) ?? 0) * 1.6;
    });
    const links = pairs.map(({ a, b, l }) => {
      const da = degree.get(a)!;
      const db = degree.get(b)!;
      const scope = linkFacts(l).scope;
      return { a, b, strength: 1 / Math.min(da, db), bias: da / (da + db), label: l.label, status: l.status, vote: l.vote, own: !scope || scope === 'self' || scope === 'none' };
    });
    const same = nodes.length === previous.length && previous.every(n => byId.has(n.id));
    const alpha = previous.length === 0 ? 1 : same ? 0.05 : 0.3;
    sim.current = { nodes, links, alpha, dirty: true, fit: sim.current.fit };
  }, [graph, hidden, shown, neighbors]);

  // Animation loop: simulate while the layout is still moving, draw when something changed
  useEffect(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;
    const ctx = canvas.getContext('2d')!;
    let frame = 0;
    let size = { w: 0, h: 0 };

    const resize = () => {
      const dpr = window.devicePixelRatio || 1;
      size = { w: container.clientWidth, h: container.clientHeight };
      canvas.width = size.w * dpr;
      canvas.height = size.h * dpr;
      canvas.style.width = `${size.w}px`;
      canvas.style.height = `${size.h}px`;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      sim.current.dirty = true;
    };
    const observer = new ResizeObserver(resize);
    observer.observe(container);
    resize();

    const draw = () => {
      const { nodes, links } = sim.current;
      const { x: ox, y: oy, k } = view.current;
      const cx = size.w / 2 + ox;
      const cy = size.h / 2 + oy;
      const fg = cssColor('--foreground', '#111');
      const focus = hoverRef.current?.id ?? selectedRef.current ?? undefined;
      const near = focus ? neighbors.get(focus) : undefined;
      const lit = (n: SimNode) => !focus || n.id === focus || !!near?.has(n.id);

      ctx.clearRect(0, 0, size.w, size.h);
      for (const { a, b, label, status, own } of links) {
        const active = focus && (a.id === focus || b.id === focus);
        // Another person's act (the party's or group's at most): muted, never the claimant's red or green
        const finding = own ? LINK_COLOR[label] : undefined;
        const foreign = !own && !!LINK_COLOR[label];
        ctx.strokeStyle = finding ?? fg;
        ctx.lineWidth = finding ? 2 : 1;
        ctx.setLineDash(status === 'suggested' ? [4, 3] : foreign ? [1, 3] : []);
        ctx.globalAlpha = active ? 0.8 : focus ? 0.04 : finding ? 0.7 : 0.15;
        ctx.beginPath();
        ctx.moveTo(cx + a.x * k, cy + a.y * k);
        ctx.lineTo(cx + b.x * k, cy + b.y * k);
        ctx.stroke();
      }
      ctx.setLineDash([]);
      ctx.lineWidth = 1;
      for (const n of nodes) {
        const r = n.r * Math.sqrt(k);
        ctx.globalAlpha = lit(n) ? 1 : 0.15;
        ctx.beginPath();
        ctx.arc(cx + n.x * k, cy + n.y * k, r, 0, Math.PI * 2);
        ctx.fillStyle = COLOR[n.group];
        ctx.fill();
        if (n.investigations?.length > 1) {
          ctx.lineWidth = 2;
          ctx.strokeStyle = fg;
          ctx.stroke();
          ctx.lineWidth = 1;
        }
      }
      ctx.font = '11px ui-sans-serif, system-ui, sans-serif';
      ctx.textAlign = 'center';
      ctx.fillStyle = fg;
      for (const n of nodes) {
        const alpha = focus ? (n.id === focus ? 1 : lit(n) ? 0.8 : 0) : Math.min(0.8, (k - (HUBS.has(n.group) ? 0.5 : 2.5)) * 2);
        if (alpha <= 0) continue;
        ctx.globalAlpha = alpha;
        ctx.fillText(truncate(n.label, n.id === focus ? 80 : 28), cx + n.x * k, cy + n.y * k + n.r * Math.sqrt(k) + 12);
      }
      ctx.globalAlpha = 1;
    };

    const loop = () => {
      const s = sim.current;
      if (s.alpha > ALPHA_MIN) {
        tick(s.nodes, s.links, s.alpha);
        s.alpha += (0 - s.alpha) * ALPHA_DECAY;
        s.dirty = true;
        if (s.fit && s.nodes.length) {
          const xs = s.nodes.map(n => n.x);
          const ys = s.nodes.map(n => n.y);
          const [minX, maxX, minY, maxY] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
          const k = Math.max(0.15, Math.min(1.5, (size.w - 80) / (maxX - minX || 1), (size.h - 80) / (maxY - minY || 1)));
          view.current = { k, x: (-(minX + maxX) / 2) * k, y: (-(minY + maxY) / 2) * k };
        }
      }
      if (s.dirty) {
        draw();
        s.dirty = false;
      }
      frame = requestAnimationFrame(loop);
    };
    frame = requestAnimationFrame(loop);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [neighbors]);

  // Pointer: drag a node, pan the background, wheel to zoom, click to open
  const pointer = useRef<{ node: SimNode | null; x: number; y: number; moved: boolean } | null>(null);

  const toWorld = (e: { clientX: number; clientY: number }) => {
    const rect = canvasRef.current!.getBoundingClientRect();
    const { x, y, k } = view.current;
    return { x: (e.clientX - rect.left - rect.width / 2 - x) / k, y: (e.clientY - rect.top - rect.height / 2 - y) / k };
  };

  const hit = (e: { clientX: number; clientY: number }) => {
    const p = toWorld(e);
    const k = view.current.k;
    let best: SimNode | null = null;
    let bestDist = Infinity;
    for (const n of sim.current.nodes) {
      const d = Math.hypot(n.x - p.x, n.y - p.y);
      if (d <= (n.r * Math.sqrt(k) + 4) / k && d < bestDist) {
        best = n;
        bestDist = d;
      }
    }
    return best;
  };

  const setHover = (node: SimNode | null) => {
    if (hoverRef.current === node) return;
    hoverRef.current = node;
    setHovered(node);
    sim.current.dirty = true;
  };

  /** Where a node's record is, if it has one. */
  const recordOf = (node: GraphNode) => {
    if (node.kind === 'promise' || node.kind === 'statement') return '/claims';
    if (node.documentId) return `/document?id=${encodeURIComponent(node.documentId)}`;
    return null;
  };

  const select = (id: string | null) => {
    selectedRef.current = id;
    setSelectedId(id);
    sim.current.dirty = true;
  };

  const onPointerDown = (e: React.PointerEvent<HTMLCanvasElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    const node = hit(e);
    pointer.current = { node, x: e.clientX, y: e.clientY, moved: false };
    if (node) node.held = true;
  };

  const onPointerMove = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const p = pointer.current;
    if (!p) {
      setHover(hit(e));
      return;
    }
    if (Math.hypot(e.clientX - p.x, e.clientY - p.y) > 3) p.moved = true;
    if (!p.moved) return;
    sim.current.fit = false;
    if (p.node) {
      Object.assign(p.node, toWorld(e));
      sim.current.alpha = Math.max(sim.current.alpha, 0.3);
    } else {
      view.current.x += e.clientX - p.x;
      view.current.y += e.clientY - p.y;
      p.x = e.clientX;
      p.y = e.clientY;
    }
    sim.current.dirty = true;
  };

  const onPointerUp = () => {
    const p = pointer.current;
    pointer.current = null;
    if (!p) return;
    if (p.node) p.node.held = false;
    if (!p.moved) select(p.node?.id ?? null);
  };

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    // Non-passive so the page doesn't scroll while zooming
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      const v = view.current;
      const k = Math.min(6, Math.max(0.15, v.k * Math.exp(-e.deltaY * 0.0015)));
      const px = e.clientX - rect.left - rect.width / 2;
      const py = e.clientY - rect.top - rect.height / 2;
      v.x = px - ((px - v.x) * k) / v.k;
      v.y = py - ((py - v.y) * k) / v.k;
      v.k = k;
      sim.current.fit = false;
      sim.current.dirty = true;
    };
    canvas.addEventListener('wheel', onWheel, { passive: false });
    return () => canvas.removeEventListener('wheel', onWheel);
  }, []);

  const hasPicks = (group: Group) => !!byGroup.get(group)?.some(n => picked.get(group)?.has(n.id));

  const pick = (group: Group, id: string) => {
    sim.current.fit = true;
    setPicked(prev => {
      const ids = new Set(prev.get(group));
      if (!ids.delete(id)) ids.add(id);
      const next = new Map(prev);
      if (ids.size) next.set(group, ids);
      else next.delete(group);
      return next;
    });
    setHidden(prev => {
      const next = new Set(prev);
      next.delete(group);
      return next;
    });
  };

  const toggleAll = (group: Group) => {
    const all = !hidden.has(group) && !hasPicks(group);
    if (hasPicks(group)) sim.current.fit = true;
    setPicked(prev => {
      const next = new Map(prev);
      next.delete(group);
      return next;
    });
    setHidden(prev => {
      const next = new Set(prev);
      if (all) next.add(group);
      else next.delete(group);
      return next;
    });
  };

  const linksOf = (id: string) => graph?.links.filter(l => l.source === id || l.target === id) ?? [];
  const hoveredLinks = hovered ? linksOf(hovered.id) : [];
  const relationText = (label: string, vote: string | null) =>
    label === 'VOTED' && vote ? t('map.rel.VOTED_with', { vote }) : t(`map.rel.${label}` as MessageKey) || label;
  const labels = useMemo(() => new Map(graph?.nodes.map(n => [n.id, n.label])), [graph]);
  const selected = selectedId ? graph?.nodes.find(n => n.id === selectedId) : undefined;
  const selectedRecord = selected ? recordOf(selected) : null;

  return (
    <div className="flex h-full flex-col">
      <div className="flex flex-wrap items-center gap-1.5 border-b border-gray-200 px-8 py-3">
        {GROUPS.filter(g => counts.has(g.group)).map(g => (
          <GroupFilter
            key={g.group}
            group={g}
            items={byGroup.get(g.group) ?? []}
            picked={picked.get(g.group)}
            hidden={hidden.has(g.group)}
            onPick={id => pick(g.group, id)}
            onAll={() => toggleAll(g.group)}
          />
        ))}
        <div className="ml-auto flex items-center gap-4">
          <label className="flex items-center gap-2 text-xs text-gray-600">
            <Switch checked={options.allVotes} onCheckedChange={allVotes => setOptions({ ...options, allVotes })} />
            {t('map.allVotes')}
          </label>
          <label className="flex items-center gap-2 text-xs text-gray-600">
            <Switch checked={options.suggested} onCheckedChange={suggested => setOptions({ ...options, suggested })} />
            {t('map.suggested')}
          </label>
          {investigationIds.length === 1 && <FlowsintButton investigationId={investigationIds[0]} />}
        </div>
      </div>
      <div ref={containerRef} className="relative min-h-0 flex-1 overflow-hidden">
        {graph && graph.nodes.length === 0 && (
          <p className="absolute inset-0 flex items-center justify-center px-8 text-center text-sm text-gray-400">{t('map.empty')}</p>
        )}
        {!graph && (
          <div className="absolute inset-0 flex items-center justify-center gap-2 text-sm text-gray-500">
            <ThinkingOrb variant="working" size={16} />
          </div>
        )}
        <canvas
          ref={canvasRef}
          role="img"
          aria-label={t('nav.map')}
          className={`absolute inset-0 touch-none ${hovered ? 'cursor-pointer' : 'cursor-grab'}`}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          onPointerLeave={() => !pointer.current && setHover(null)}
        />
        <div className={`pointer-events-none absolute bottom-4 left-8 text-xs text-gray-500 ${selected ? 'right-[22rem]' : 'right-8'}`}>
          {hovered ? (
            <div className="max-w-2xl rounded-md border border-gray-200 bg-white/90 px-3 py-2 text-gray-700">
              <span className="font-medium" style={{ color: COLOR[hovered.group] }}>
                {t(GROUPS.find(g => g.group === hovered.group)!.label)}
              </span>
              <br />
              {hovered.label}
              {hoveredLinks.length > 0 && hoveredLinks.length <= 6 && (
                <span className="mt-1 block text-gray-500">
                  {hoveredLinks.map(l => relationText(l.label, l.vote) + (l.status === 'suggested' ? ` (${t('map.unreviewed')})` : '')).join(' · ')}
                </span>
              )}
            </div>
          ) : (
            graph && graph.nodes.length > 0 && t('map.hint')
          )}
        </div>
        {graph && graph.nodes.length > 0 && (
          <div className="pointer-events-none absolute left-8 top-4 space-y-1 rounded-md border border-gray-200 bg-white/90 px-3 py-2 text-xs text-gray-600">
            {LEGEND.map(([color, dashed, label]) => (
              <div key={label} className="flex items-center gap-2">
                <svg width="24" height="6" aria-hidden>
                  <line x1="0" y1="3" x2="24" y2="3" stroke={color} strokeWidth={color === 'currentColor' ? 1 : 2} strokeDasharray={dashed ? '4 3' : undefined} />
                </svg>
                {t(label)}
              </div>
            ))}
          </div>
        )}
        {selected && (
          <NodePanel
            key={selected.id}
            node={selected}
            links={linksOf(selected.id)}
            labelOf={id => labels.get(id) ?? id}
            relationText={relationText}
            onOpen={selectedRecord ? () => router.push(selectedRecord) : undefined}
            onOpenDocument={id => router.push(`/document?id=${encodeURIComponent(id)}`)}
            onClose={() => select(null)}
            onEnriched={load}
          />
        )}
      </div>
    </div>
  );
}

