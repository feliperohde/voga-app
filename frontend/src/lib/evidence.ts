import { DocumentKind, RECORD_KINDS } from '@/services/investigationService';

/** The document page of a record, at the moment cited when it has one. */
export function documentHref(documentId: string | null, startTime: number | null) {
  if (!documentId) return null;
  return `/document?id=${encodeURIComponent(documentId)}${startTime != null ? `&t=${Math.floor(startTime)}` : ''}`;
}

/** The official page of a record (without Voga's "#…" marker), and for a Câmara vote the list of
 * every deputy's vote in that session. */
export function officialLinks(kind: DocumentKind | null, url: string | null) {
  if (!url || !kind || !RECORD_KINDS.includes(kind)) return { page: null, allVotes: null };
  const vote = kind === 'vote' && url.includes('camara.leg.br') ? /#votacao-([^-]+)-/.exec(url)?.[1] : null;
  return { page: url.split('#')[0], allVotes: vote ? `https://dadosabertos.camara.leg.br/api/v2/votacoes/${vote}/votos` : null };
}
