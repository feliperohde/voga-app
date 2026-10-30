import type { AiProvider } from '@/services/settingsService';

/** An AI the user may already pay for, connected in one go: what Voga uses it for, and where its key is. */
export interface Connectable {
  provider: AiProvider;
  label: string;
  /** Where to get an API key; null: no key (Claude Code uses the user's subscription). */
  keyUrl: string | null;
  /** Also embeds passages for search (else the built-in model does). */
  embeddings: boolean;
  /** Also transcribes live sessions (else Whisper on this computer does). */
  transcription: boolean;
  /** An address to type: a server of the user's. */
  endpoint?: boolean;
}

export const CONNECTABLE: Connectable[] = [
  { provider: 'openai', label: 'OpenAI (ChatGPT)', keyUrl: 'https://platform.openai.com/api-keys', embeddings: true, transcription: true },
  { provider: 'gemini', label: 'Google Gemini', keyUrl: 'https://aistudio.google.com/apikey', embeddings: true, transcription: false },
  { provider: 'claude', label: 'Claude (Anthropic)', keyUrl: 'https://console.anthropic.com/settings/keys', embeddings: false, transcription: false },
  { provider: 'groq', label: 'Groq', keyUrl: 'https://console.groq.com/keys', embeddings: false, transcription: true },
  { provider: 'deepseek', label: 'DeepSeek', keyUrl: 'https://platform.deepseek.com/api_keys', embeddings: false, transcription: false },
  { provider: 'openrouter', label: 'OpenRouter', keyUrl: 'https://openrouter.ai/keys', embeddings: false, transcription: false },
  { provider: 'claude-code', label: 'Claude Code', keyUrl: null, embeddings: false, transcription: false },
  { provider: 'custom-openai', label: 'OpenAI-compatible', keyUrl: null, embeddings: true, transcription: true, endpoint: true },
];

/** Models that are not for chat (in a provider's list). */
const NOT_CHAT = /embed|whisper|transcri|tts|audio|realtime|image|dall|guard|moderation|search|codex|instruct|davinci|babbage|rerank|vision|computer-use/i;

/** What a provider's model list is used for. Newest first: names carry their version. */
const PREFER: Partial<Record<AiProvider, { chat: RegExp[]; embeddings?: RegExp[]; transcription?: RegExp[] }>> = {
  openai: { chat: [/^gpt-[\d.]+-mini$/, /^gpt-[\d.]+-mini/, /^gpt-/], embeddings: [/text-embedding-3-small/, /embedding/], transcription: [/mini-transcribe/, /transcribe/, /whisper/] },
  gemini: { chat: [/^gemini-[\d.]+-flash$/, /^gemini-[\d.]+-flash(?!.*lite)/, /flash/], embeddings: [/gemini-embedding/, /embedding/] },
  claude: { chat: [/haiku/, /sonnet/] },
  groq: { chat: [/gpt-oss-120b/, /llama-3\.3-70b/, /70b/], transcription: [/whisper-large-v3-turbo/, /whisper/] },
  deepseek: { chat: [/flash$/, /chat/, /deepseek/] },
  'custom-openai': { chat: [/./], embeddings: [/embed/], transcription: [/whisper|transcri/] },
};

const pick = (ids: string[], patterns: RegExp[] = [], exclude?: RegExp) => {
  const newest = [...ids].sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
  for (const pattern of patterns) {
    const found = newest.find(id => pattern.test(id) && !(exclude && exclude.test(id)));
    if (found) return found;
  }
  return '';
};

/** A sensible model of each kind from what the provider lists (empty: none, or let the user pick).
 * ponytail: patterns over model names; ask the provider's docs if a family renames itself. */
export function pickModels(provider: AiProvider, ids: string[]): { chat: string; embeddings: string; transcription: string } {
  const prefer = PREFER[provider];
  const connectable = CONNECTABLE.find(c => c.provider === provider);
  return {
    chat: provider === 'claude-code' ? 'sonnet' : pick(ids, prefer?.chat, NOT_CHAT),
    embeddings: connectable?.embeddings ? pick(ids, prefer?.embeddings) : '',
    transcription: connectable?.transcription ? pick(ids, prefer?.transcription) : '',
  };
}

/** The models of a list good for each use, newest first. */
export function modelsFor(ids: string[], use: 'chat' | 'embeddings' | 'transcription'): string[] {
  const newest = [...ids].sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
  if (use === 'chat') return newest.filter(id => !NOT_CHAT.test(id));
  return newest.filter(id => (use === 'embeddings' ? /embed/i : /whisper|transcri/i).test(id));
}
