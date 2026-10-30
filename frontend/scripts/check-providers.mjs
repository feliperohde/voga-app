// Self-check of the model picks of "Connect your AI" (src/lib/providers.ts):
//   node scripts/check-providers.mjs
import assert from 'node:assert/strict';
import { pickModels } from '../src/lib/providers.ts';

// The newest small chat model, the cheap embeddings, the light transcription
assert.deepEqual(pickModels('openai', ['gpt-5-mini', 'gpt-6', 'gpt-6-mini', 'text-embedding-3-large', 'text-embedding-3-small', 'gpt-4o-mini-transcribe', 'whisper-1', 'gpt-4o-mini-tts']), {
  chat: 'gpt-6-mini',
  embeddings: 'text-embedding-3-small',
  transcription: 'gpt-4o-mini-transcribe',
});
// Flash, not lite nor pro; Gemini has no transcription API
assert.deepEqual(pickModels('gemini', ['gemini-3.1-flash', 'gemini-3.8-flash', 'gemini-3.8-flash-lite', 'gemini-3-pro', 'gemini-embedding-2']), {
  chat: 'gemini-3.8-flash',
  embeddings: 'gemini-embedding-2',
  transcription: '',
});
// Groq: chat and Whisper, no embeddings; never a transcription model for chat
assert.deepEqual(pickModels('groq', ['whisper-large-v3', 'whisper-large-v3-turbo', 'llama-3.3-70b-versatile', 'openai/gpt-oss-120b']), {
  chat: 'openai/gpt-oss-120b',
  embeddings: '',
  transcription: 'whisper-large-v3-turbo',
});
assert.equal(pickModels('claude', ['claude-sonnet-5-5', 'claude-haiku-5-5']).chat, 'claude-haiku-5-5');
assert.equal(pickModels('claude-code', []).chat, 'sonnet');
assert.equal(pickModels('openrouter', ['a/b']).chat, '', 'OpenRouter lists too many to guess: the person picks');
console.log('providers: ok');
