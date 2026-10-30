'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { History, SendHorizonal, SquarePen } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { PageHeader } from '@/components/PageHeader';
import { ThinkingOrb } from '@/components/ThinkingOrb';
import { CitationCard } from '@/components/CitationCard';
import { useInvestigation } from '@/contexts/InvestigationContext';
import { formatDate, useI18n } from '@/i18n';
import { Answer, askService, AskHistoryEntry } from '@/services/askService';

interface Turn {
  question: string;
  answer: Answer;
}

function TurnView({ turn }: { turn: Turn }) {
  const { t } = useI18n();
  const { answer } = turn;
  return (
    <div className="space-y-3">
      <div className="ml-auto max-w-[85%] rounded-lg bg-gray-100 px-4 py-2.5 text-gray-900">{turn.question}</div>
      {answer.found ? (
        <div className="space-y-3">
          <div className="prose prose-sm max-w-none select-text dark:prose-invert">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{answer.answer}</ReactMarkdown>
          </div>
          {answer.notice && <p className="text-xs text-amber-700">{t('ask.keywordsOnly')}</p>}
          {answer.filtersRelaxed && <p className="text-xs text-gray-500">{t('ask.filtersRelaxed')}</p>}
          <div className="grid gap-2 sm:grid-cols-2">
            {answer.citations.map(citation => (
              <CitationCard key={citation.chunkId} citation={citation} />
            ))}
          </div>
        </div>
      ) : (
        <p className="rounded-md border border-gray-200 bg-gray-50 p-3 text-sm text-gray-700">{t('ask.notFound')}</p>
      )}
    </div>
  );
}

function AskView() {
  const { t, locale } = useI18n();
  // One investigation, or several asked about together
  const { scope } = useInvestigation();
  const ids = scope.map(i => i.id).join(',');
  const [turns, setTurns] = useState<Turn[]>([]);
  const [question, setQuestion] = useState('');
  const [asking, setAsking] = useState(false);
  const [history, setHistory] = useState<AskHistoryEntry[]>([]);
  const bottom = useRef<HTMLDivElement | null>(null);

  const loadHistory = useCallback(() => {
    if (ids) askService.history(ids.split(',')).then(setHistory).catch(() => {});
  }, [ids]);

  useEffect(() => {
    setTurns([]);
    loadHistory();
  }, [loadHistory]);

  useEffect(() => {
    bottom.current?.scrollIntoView({ behavior: 'smooth' });
  }, [turns.length, asking]);

  const ask = async () => {
    const text = question.trim();
    if (!ids || !text || asking) return;
    setAsking(true);
    try {
      const previous = turns.map(turn => ({ question: turn.question, answer: turn.answer.answer }));
      const answer = await askService.ask(ids.split(','), text, previous);
      setTurns([...turns, { question: text, answer }]);
      setQuestion('');
      loadHistory();
    } catch (error) {
      toast.error(t('ask.failed'), { description: String(error) });
    } finally {
      setAsking(false);
    }
  };

  const openSaved = async (entry: AskHistoryEntry) => {
    const answer = await askService.saved(entry.id).catch(() => null);
    if (answer) setTurns([{ question: entry.question, answer }]);
  };

  if (!ids) return <PageHeader title={t('nav.ask')} />;

  return (
    <div className="flex h-screen flex-col">
      <PageHeader
        title={t('nav.ask')}
        actions={
          turns.length > 0 && (
            <Button variant="outline" size="sm" onClick={() => setTurns([])}>
              <SquarePen className="mr-2 h-4 w-4" /> {t('ask.newConversation')}
            </Button>
          )
        }
      />
      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex-1 overflow-y-auto">
            <div className="mx-auto max-w-3xl space-y-8 px-4 py-6 sm:px-8">
              {turns.length === 0 && !asking && (
                <div className="py-10 text-center">
                  <h2 className="text-lg font-semibold text-gray-900">{t('ask.emptyTitle', { name: scope.map(i => i.name).join(' × ') })}</h2>
                  <p className="mt-1 text-sm text-gray-600">{t('ask.emptyHelp')}</p>
                </div>
              )}
              {turns.map((turn, i) => (
                <TurnView key={i} turn={turn} />
              ))}
              {asking && (
                <div className="flex items-center gap-2 text-sm text-gray-600">
                  <ThinkingOrb variant="thinking" size={20} /> {t('ask.thinking')}
                </div>
              )}
              <div ref={bottom} />
            </div>
          </div>
          <div className="border-t border-gray-200 bg-white px-4 py-3 sm:px-8">
            <div className="mx-auto flex max-w-3xl items-end gap-2">
              <Textarea
                value={question}
                onChange={e => setQuestion(e.target.value)}
                onKeyDown={e => {
                  if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    ask();
                  }
                }}
                placeholder={t('ask.placeholder')}
                rows={2}
                className="resize-none"
              />
              <Button onClick={ask} disabled={asking || !question.trim()} aria-label={t('ask.send')}>
                <SendHorizonal className="h-4 w-4" />
              </Button>
            </div>
          </div>
        </div>
        {history.length > 0 && (
          <aside className="hidden w-64 flex-shrink-0 overflow-y-auto border-l border-gray-200 p-3 lg:block">
            <div className="mb-2 flex items-center gap-1.5 px-1 text-xs font-medium uppercase tracking-wide text-gray-500">
              <History className="h-3.5 w-3.5" /> {t('ask.history')}
            </div>
            {history.map(entry => (
              <button key={entry.id} onClick={() => openSaved(entry)} className="block w-full rounded-md px-2 py-1.5 text-left hover:bg-gray-100">
                <span className="line-clamp-2 text-sm text-gray-800">{entry.question}</span>
                <span className="text-xs text-gray-500">{formatDate(locale, entry.createdAt, { dateStyle: 'short', timeStyle: 'short' })}</span>
              </button>
            ))}
          </aside>
        )}
      </div>
    </div>
  );
}

export default function AskPage() {
  return <AskView />;
}
